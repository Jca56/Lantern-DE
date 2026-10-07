//! What the Processes page lists: the processes that pass the search,
//! folded by program or not, in the order asked for. Plain data in,
//! plain data out; the page only draws what comes of it.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::sample::procs::Proc;

/// The list's columns, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Name,
    User,
    Cpu,
    Memory,
    Pid,
}

impl Column {
    pub const ALL: [Column; 5] = [Column::Name, Column::User, Column::Cpu, Column::Memory, Column::Pid];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub by: Column,
    pub ascending: bool,
}

impl Default for Sort {
    /// Whatever is working hardest, first.
    fn default() -> Self {
        Self { by: Column::Cpu, ascending: false }
    }
}

/// What is listed and how.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub search: String,
    /// Fold a program's processes into one row.
    pub grouped: bool,
    /// List the kernel's own threads too.
    pub kernel: bool,
    pub sort: Sort,
    /// The programs whose rows are open, showing their processes.
    pub open: BTreeSet<Arc<str>>,
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A program with several processes, as their sums. `user` is empty
    /// when they are not all one user's.
    Group { app: Arc<str>, count: usize, cpu: f32, memory: u64, user: Arc<str>, open: bool },
    /// The process at `index` in the frame; `inside` when it is listed
    /// under its program's row.
    Proc { index: usize, inside: bool },
}

/// What a row stands for, kept from look to look: the selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    App(Arc<str>),
    Pid(u32),
}

impl Row {
    pub fn pick(&self, procs: &[Proc]) -> Option<Pick> {
        match self {
            Row::Group { app, .. } => Some(Pick::App(app.clone())),
            Row::Proc { index, .. } => procs.get(*index).map(|p| Pick::Pid(p.pid)),
        }
    }
}

/// Letters compared without their case, and without making new strings.
fn caseless(a: &str, b: &str) -> Ordering {
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

/// Whether a process passes the search `needle`, which is already in
/// lower case: by its name, its program, its command, its user or its
/// number.
fn matches(p: &Proc, needle: &str) -> bool {
    let has = |s: &str| s.to_lowercase().contains(needle);
    needle.is_empty() || has(&p.name) || has(&p.app) || has(&p.user) || p.pid.to_string().contains(needle) || has(&p.command)
}

/// The processes `view` lets through, as places in `procs`.
fn listed(procs: &[Proc], view: &View) -> Vec<usize> {
    let needle = view.search.trim().to_lowercase();
    (0..procs.len()).filter(|i| (view.kernel || !procs[*i].kernel) && matches(&procs[*i], &needle)).collect()
}

/// What one row is ordered by.
struct Key<'a> {
    name: &'a str,
    user: &'a str,
    cpu: f32,
    memory: u64,
    pid: u32,
}

impl Key<'_> {
    /// Before or after `other` under `sort`. Two that tie are in number
    /// order whichever way the sort runs, so idle rows don't swap places
    /// from one look to the next.
    fn order(&self, other: &Key, sort: Sort) -> Ordering {
        let by = match sort.by {
            Column::Name => caseless(self.name, other.name),
            Column::User => caseless(self.user, other.user).then(caseless(self.name, other.name)),
            Column::Cpu => self.cpu.total_cmp(&other.cpu),
            Column::Memory => self.memory.cmp(&other.memory),
            Column::Pid => self.pid.cmp(&other.pid),
        };
        (if sort.ascending { by } else { by.reverse() }).then(self.pid.cmp(&other.pid))
    }
}

fn proc_key(p: &Proc) -> Key<'_> {
    Key { name: &p.name, user: &p.user, cpu: p.cpu, memory: p.memory, pid: p.pid }
}

/// A program's processes, summed.
struct Group {
    app: Arc<str>,
    members: Vec<usize>,
    cpu: f32,
    memory: u64,
    /// Its lowest process number: the oldest, most often the one that
    /// started the rest.
    pid: u32,
    user: Arc<str>,
}

/// The rows to show for `procs` under `view`.
pub fn rows(procs: &[Proc], view: &View) -> Vec<Row> {
    let mut listed = listed(procs, view);
    listed.sort_by(|a, b| proc_key(&procs[*a]).order(&proc_key(&procs[*b]), view.sort));
    if !view.grouped {
        return listed.into_iter().map(|index| Row::Proc { index, inside: false }).collect();
    }

    // In the sorted order, so each group's members stay sorted too.
    let mut groups: Vec<Group> = Vec::new();
    let mut place: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for index in listed {
        let p = &procs[index];
        let i = *place.entry(&*p.app).or_insert_with(|| {
            groups.push(Group { app: p.app.clone(), members: Vec::new(), cpu: 0.0, memory: 0, pid: p.pid, user: p.user.clone() });
            groups.len() - 1
        });
        let g = &mut groups[i];
        g.members.push(index);
        g.cpu += p.cpu;
        g.memory += p.memory;
        g.pid = g.pid.min(p.pid);
        if g.user != p.user {
            g.user = Arc::from("");
        }
    }
    groups.sort_by(|a, b| Key { name: &a.app, user: &a.user, cpu: a.cpu, memory: a.memory, pid: a.pid }.order(&Key { name: &b.app, user: &b.user, cpu: b.cpu, memory: b.memory, pid: b.pid }, view.sort));

    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        // A program with one process is just that process.
        if let [only] = g.members[..] {
            out.push(Row::Proc { index: only, inside: false });
            continue;
        }
        let open = view.open.contains(&g.app);
        out.push(Row::Group { app: g.app, count: g.members.len(), cpu: g.cpu, memory: g.memory, user: g.user, open });
        if open {
            out.extend(g.members.into_iter().map(|index| Row::Proc { index, inside: true }));
        }
    }
    out
}

/// The processes a pick stands for, each with when it started: what a
/// signal is sent to. A program's are the ones listed under it, so what
/// is ended is what was shown.
pub fn targets(procs: &[Proc], view: &View, pick: &Pick) -> Vec<(u32, u64)> {
    match pick {
        Pick::Pid(pid) => procs.iter().filter(|p| p.pid == *pid).map(|p| (p.pid, p.started)).collect(),
        Pick::App(app) => listed(procs, view).into_iter().map(|i| &procs[i]).filter(|p| p.app == *app).map(|p| (p.pid, p.started)).collect(),
    }
}

/// What a pick is called, for a dialog's title and a toast.
pub fn pick_name(procs: &[Proc], pick: &Pick) -> String {
    match pick {
        Pick::App(app) => app.to_string(),
        Pick::Pid(pid) => procs.iter().find(|p| p.pid == *pid).map_or_else(|| format!("process {pid}"), |p| p.name.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    fn view() -> View {
        View { grouped: true, ..View::default() }
    }

    /// Each row as words: a program as `name×count`, a process as its
    /// number, indented when it is inside a program.
    fn said(procs: &[Proc], view: &View) -> Vec<String> {
        rows(procs, view)
            .iter()
            .map(|r| match r {
                Row::Group { app, count, .. } => format!("{app}×{count}"),
                Row::Proc { index, inside } => format!("{}{}", if *inside { "  " } else { "" }, procs[*index].pid),
            })
            .collect()
    }

    #[test]
    fn programs_are_folded_and_the_busiest_comes_first() {
        let procs = fixture::procs();
        // rustc's two make 23.5 between them, firefox's four 12.4; the
        // kernel's threads are left out; the idle ones tie, in number order.
        assert_eq!(said(&procs, &view()), ["rustc×2", "firefox×4", "1201", "6000", "1", "900", "1300"]);
        let Row::Group { cpu, memory, user, open, .. } = rows(&procs, &view())[1].clone() else { panic!("a group") };
        assert!((cpu - 12.4).abs() < 1e-4);
        assert_eq!((memory, &*user, open), (2100 << 20, "alva", false));
    }

    #[test]
    fn an_open_program_lists_its_processes_in_the_same_order() {
        let procs = fixture::procs();
        let mut v = view();
        v.open.insert(Arc::from("firefox"));
        assert_eq!(said(&procs, &v)[1..6], ["firefox×4", "  4410", "  4411", "  4412", "  4413"]);
        v.sort = Sort { by: Column::Memory, ascending: true };
        let all = said(&procs, &v);
        let at = all.iter().position(|r| r == "firefox×4").unwrap();
        assert_eq!(all[at + 1..at + 5], ["  4413", "  4412", "  4411", "  4410"]);
    }

    #[test]
    fn unfolded_every_process_is_a_row_and_the_kernels_show_when_asked() {
        let procs = fixture::procs();
        let flat = View { sort: Sort { by: Column::Pid, ascending: true }, ..View::default() };
        assert_eq!(said(&procs, &flat), ["1", "900", "1201", "1300", "4410", "4411", "4412", "4413", "5001", "5002", "6000"]);
        let with_kernel = View { kernel: true, grouped: true, sort: Sort { by: Column::Name, ascending: true }, ..View::default() };
        // By name, whatever the case: kthreadd is one thread, the two
        // kworkers fold into their kind.
        assert_eq!(said(&procs, &with_kernel), ["firefox×4", "1", "2", "kworker×2", "1201", "rustc×2", "900", "6000", "1300"]);
    }

    #[test]
    fn a_search_finds_names_programs_users_numbers_and_commands() {
        let procs = fixture::procs();
        let find = |s: &str| said(&procs, &View { search: s.to_owned(), ..view() });
        assert_eq!(find("  FIRE "), ["firefox×4"]);
        // One of a program's processes found by its own name is a row of
        // its own: what is summed is what is shown.
        assert_eq!(find("web content"), ["4411"]);
        assert_eq!(find("4412"), ["4412"]);
        assert_eq!(find("root"), ["1", "900"]);
        assert_eq!(find("--pid-6000"), ["6000"]);
        assert!(find("no such thing").is_empty());
    }

    #[test]
    fn a_pick_names_what_a_signal_would_go_to() {
        let procs = fixture::procs();
        let v = view();
        assert_eq!(targets(&procs, &v, &Pick::Pid(1300)), [(1300, 13_000)]);
        assert_eq!(targets(&procs, &v, &Pick::App(Arc::from("rustc"))), [(5001, 50_010), (5002, 50_020)]);
        assert!(targets(&procs, &v, &Pick::Pid(99_999)).is_empty(), "it has gone");
        // Hidden by the search: not among what the program's row stands for.
        let narrowed = View { search: "web content".to_owned(), ..view() };
        assert_eq!(targets(&procs, &narrowed, &Pick::App(Arc::from("firefox"))), [(4411, 44_110)]);
        // The kernel's threads are not listed, so a program's row never
        // reaches them.
        assert!(targets(&procs, &v, &Pick::App(Arc::from("kworker"))).is_empty());
        assert_eq!((pick_name(&procs, &Pick::Pid(1300)), pick_name(&procs, &Pick::Pid(5)), pick_name(&procs, &Pick::App(Arc::from("rustc")))), ("zsh".to_owned(), "process 5".to_owned(), "rustc".to_owned()));
        assert_eq!(rows(&procs, &v)[0].pick(&procs), Some(Pick::App(Arc::from("rustc"))));
    }
}
