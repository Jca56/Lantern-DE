//! The processes: every numbered folder in `/proc`, read each look for
//! what changes (processor time, memory) and once for what doesn't (its
//! name, its command line, whose it is).

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

use crate::ffi;

/// The kernel's mark, in a process's flags, for a thread of its own.
const PF_KTHREAD: u64 = 0x0020_0000;
/// `comm` holds this many bytes of a name and drops the rest.
const COMM_LEN: usize = 15;

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// What it is called: its command's name, whole.
    pub name: Arc<str>,
    /// The program it is part of: what rows are folded together by.
    pub app: Arc<str>,
    /// Its command line, arguments and all; empty for the kernel's own.
    pub command: Arc<str>,
    pub user: Arc<str>,
    /// One of the kernel's own threads rather than a program.
    pub kernel: bool,
    pub threads: u32,
    /// Percent of the whole machine's processor time over the last
    /// interval: every processor flat out is 100.
    pub cpu: f32,
    /// Bytes of memory that are its own: what it holds, less what it
    /// shares with other processes.
    pub memory: u64,
    /// When it started, in clock ticks since the machine did. With the
    /// pid, this tells it from a later process given the same number.
    pub started: u64,
}

/// What `stat` says about a process, as far as is used here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat<'a> {
    pub comm: &'a str,
    pub parent: u32,
    pub flags: u64,
    /// Clock ticks of processor time it has had, in its own code and in
    /// the kernel's on its behalf.
    pub ticks: u64,
    pub threads: u32,
    pub started: u64,
}

/// A `/proc/<pid>/stat` line. The name is in brackets and may itself
/// hold spaces and brackets, so the fields are counted from the last
/// `)`.
pub fn parse_stat(line: &str) -> Option<Stat<'_>> {
    let (open, close) = (line.find('(')?, line.rfind(')')?);
    if close <= open {
        return None;
    }
    // After the name: state, ppid, pgrp, session, tty, tpgid, flags (6),
    // four fault counts, utime (11), stime (12), two more times,
    // priority, nice, threads (17), itrealvalue, starttime (19).
    let f: Vec<&str> = line.get(close + 1..)?.split_ascii_whitespace().take(20).collect();
    let n = |i: usize| f.get(i)?.parse::<u64>().ok();
    Some(Stat { comm: &line[open + 1..close], parent: u32::try_from(n(1)?).ok()?, flags: n(6)?, ticks: n(11)? + n(12)?, threads: u32::try_from(n(17)?).ok()?, started: n(19)? })
}

/// The memory that is a process's own, in pages, from its `statm`:
/// what is resident less what is shared.
pub fn parse_statm(line: &str) -> u64 {
    let mut f = line.split_ascii_whitespace().skip(1).map(|n| n.parse::<u64>().unwrap_or(0));
    let (resident, shared) = (f.next().unwrap_or(0), f.next().unwrap_or(0));
    resident.saturating_sub(shared)
}

/// What to call a process. `comm` is cut short at fifteen bytes, so
/// when the first word of the command line carries on from it, that
/// word's file name is used whole.
pub fn full_name(comm: &str, first_word: &str) -> String {
    let file = first_word.rsplit('/').next().unwrap_or("");
    if comm.len() >= COMM_LEN && file.starts_with(comm) { file.to_owned() } else { comm.to_owned() }
}

/// Programs that only run other things: a script, a Windows program, a
/// shell's command. What they run says more than what they are.
fn runs_others(file: &str) -> bool {
    ["sh", "bash", "zsh", "dash", "fish", "node", "java", "env", "bwrap"].contains(&file) || ["python", "perl", "ruby", "wine", "lua", "php", "electron"].iter().any(|p| file.starts_with(p))
}

/// A program file's name without what packaging hangs on its end:
/// `rustc` for `rustc-bin-1.98.1`, `firefox` for `firefox-bin`.
fn plain(file: &str) -> &str {
    /// `f` without a version after its last dash: `gtk` for `gtk-4.0`.
    fn unversioned(f: &str) -> Option<&str> {
        f.rsplit_once('-').filter(|(_, v)| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.')).map(|(head, _)| head)
    }
    let mut f = file;
    loop {
        match f.strip_suffix("-bin").or(f.strip_suffix(".bin")).or(f.strip_suffix("-wrapped")).or(f.strip_suffix(".real")).or_else(|| unversioned(f)) {
            Some(shorter) if !shorter.is_empty() => f = shorter,
            _ => return f,
        }
    }
}

/// The program a process is part of, for folding a program's many
/// processes into one row: the file it runs, since a browser's helpers
/// call themselves all sorts but all run the browser. For a kernel
/// thread, its kind (`kworker` for `kworker/3:1-events`).
pub fn app_name(name: &str, exe: &str, kernel: bool) -> String {
    if kernel {
        return name.split(['/', ':']).next().unwrap_or(name).to_owned();
    }
    // A program replaced on disk since it started says so after its path.
    let file = exe.strip_suffix(" (deleted)").unwrap_or(exe).rsplit('/').next().unwrap_or("");
    // A file named for its version alone (`2.1.293`) names nothing.
    if file.is_empty() || runs_others(file) || !file.starts_with(char::is_alphabetic) { name.to_owned() } else { plain(file).to_owned() }
}

/// `/etc/passwd`: who each user number is.
pub fn parse_passwd(text: &str) -> HashMap<u32, Arc<str>> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split(':');
            let (name, _, uid) = (f.next()?, f.next()?, f.next()?);
            Some((uid.parse().ok()?, Arc::from(name)))
        })
        .collect()
}

/// What is kept about a process between looks.
struct Known {
    started: u64,
    comm: String,
    ticks: u64,
    name: Arc<str>,
    app: Arc<str>,
    command: Arc<str>,
    user: Arc<str>,
}

pub struct ProcSampler {
    known: HashMap<u32, Known>,
    users: HashMap<u32, Arc<str>>,
    /// Reused for every file read, and for every path made.
    text: String,
    path: String,
}

/// Read `path` into `text`, which is emptied first. `false` when the
/// process has gone or won't be read.
fn slurp(path: &str, text: &mut String) -> bool {
    text.clear();
    fs::File::open(path).and_then(|mut f| f.read_to_string(text)).is_ok()
}

impl ProcSampler {
    pub fn new() -> Self {
        Self { known: HashMap::new(), users: parse_passwd(&fs::read_to_string("/etc/passwd").unwrap_or_default()), text: String::new(), path: String::new() }
    }

    fn path(&mut self, pid: u32, file: &str) -> &str {
        use std::fmt::Write;
        self.path.clear();
        let _ = write!(self.path, "/proc/{pid}/{file}");
        &self.path
    }

    /// What doesn't change while a process lives, read when it is first
    /// seen and again when it becomes another program.
    fn learn(&mut self, pid: u32, stat: &Stat) -> Known {
        let kernel = stat.flags & PF_KTHREAD != 0;
        // Arguments are parted by NULs; shown, they are parted by spaces.
        let raw = if kernel { Vec::new() } else { fs::read(self.path(pid, "cmdline")).unwrap_or_default() };
        let first = raw.split(|b| *b == 0).next().map(|w| String::from_utf8_lossy(w).into_owned()).unwrap_or_default();
        let command = String::from_utf8_lossy(&raw).replace('\0', " ").trim().to_owned();
        let exe = if kernel { String::new() } else { fs::read_link(self.path(pid, "exe")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default() };
        let name = full_name(stat.comm, &first);
        let uid = fs::metadata(self.path(pid, "")).map(|m| m.uid()).unwrap_or(0);
        let user = self.users.get(&uid).cloned().unwrap_or_else(|| Arc::from(uid.to_string()));
        Known { started: stat.started, comm: stat.comm.to_owned(), ticks: stat.ticks, app: Arc::from(app_name(&name, &exe, kernel)), name: Arc::from(name), command: Arc::from(command), user }
    }

    /// Every process now. `machine_ticks` is how many clock ticks all the
    /// processors together have counted since the last look: what each
    /// process's own are a share of.
    pub fn sample(&mut self, machine_ticks: u64) -> Vec<Proc> {
        let Ok(dir) = fs::read_dir("/proc") else { return Vec::new() };
        let page = ffi::page_size();
        let mut text = std::mem::take(&mut self.text);
        let mut procs = Vec::with_capacity(self.known.len() + 16);
        for entry in dir.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if !slurp(self.path(pid, "stat"), &mut text) {
                continue;
            }
            let Some(stat) = parse_stat(&text) else { continue };
            // The same number and the same start is the same process;
            // another name on it means it has become another program.
            let fresh = self.known.get(&pid).is_none_or(|k| k.started != stat.started || k.comm != stat.comm);
            let spent = if fresh {
                let learnt = self.learn(pid, &stat);
                self.known.insert(pid, learnt);
                0
            } else {
                self.known.get(&pid).map_or(0, |k| stat.ticks.saturating_sub(k.ticks))
            };
            let (parent, kernel, threads, ticks, started) = (stat.parent, stat.flags & PF_KTHREAD != 0, stat.threads, stat.ticks, stat.started);
            let memory = if slurp(self.path(pid, "statm"), &mut text) { parse_statm(&text) * page } else { 0 };
            let Some(known) = self.known.get_mut(&pid) else { continue };
            known.ticks = ticks;
            let cpu = if machine_ticks == 0 { 0.0 } else { (spent as f64 / machine_ticks as f64 * 100.0).min(100.0) as f32 };
            procs.push(Proc { pid, parent, name: known.name.clone(), app: known.app.clone(), command: known.command.clone(), user: known.user.clone(), kernel, threads, cpu, memory, started });
        }
        self.text = text;
        // Whoever wasn't seen this time has gone.
        if self.known.len() > procs.len() {
            let alive: std::collections::HashSet<u32> = procs.iter().map(|p| p.pid).collect();
            self.known.retain(|pid, _| alive.contains(pid));
        }
        procs
    }
}

/// When process `pid` started, if it is still there: what a process
/// about to be signalled is checked against, so the signal never goes
/// to a newer process that was handed the same number.
pub fn started(pid: u32) -> Option<u64> {
    parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?).map(|s| s.started)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_is_read_past_a_name_with_spaces_and_brackets_in_it() {
        let line = "4410 (Web Content (x) y) S 4300 4300 4300 0 -1 4194304 136 0 0 0 250 50 0 0 20 0 27 0 863164 5890048 460 18446744073709551615 1 1 0";
        let s = parse_stat(line).unwrap();
        assert_eq!(s, Stat { comm: "Web Content (x) y", parent: 4300, flags: 4_194_304, ticks: 300, threads: 27, started: 863_164 });
        assert_eq!(s.flags & PF_KTHREAD, 0);
        // The kernel's own.
        let kthread = parse_stat("2 (kthreadd) S 0 0 0 0 -1 2129984 0 0 0 0 0 5 0 0 20 0 1 0 1 0 0").unwrap();
        assert_ne!(kthread.flags & PF_KTHREAD, 0);
        for bad in ["", "12 no brackets", "12 )backwards( S 1", "12 (short) S 1 2 3"] {
            assert_eq!(parse_stat(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn memory_of_its_own_is_what_it_holds_less_what_it_shares() {
        assert_eq!(parse_statm("50000 12000 9000 100 0 3000 0\n"), 3000);
        assert_eq!(parse_statm("50000 100 900"), 0, "never below nothing");
        assert_eq!(parse_statm(""), 0);
    }

    #[test]
    fn a_name_cut_short_is_made_whole_from_the_command_line() {
        assert_eq!(full_name("lntrn-system-se", "/home/a/.lantern/bin/lntrn-system-settings"), "lntrn-system-settings");
        assert_eq!(full_name("zsh", "-zsh"), "zsh");
        // Fifteen long but not the start of the command: a name it gave itself.
        assert_eq!(full_name("Isolated Web Co", "/usr/lib/firefox/firefox"), "Isolated Web Co");
        assert_eq!(full_name("kworker/0:1", ""), "kworker/0:1");
    }

    #[test]
    fn a_process_belongs_to_the_program_it_runs() {
        assert_eq!(app_name("Isolated Web Co", "/usr/lib/firefox/firefox", false), "firefox");
        assert_eq!(app_name("chrome", "/opt/google/chrome/chrome (deleted)", false), "chrome");
        // What packaging hangs on a file's name is taken off, for every
        // process that runs it alike.
        assert_eq!(app_name("rustc", "/opt/rust-bin-1.98.1/bin/rustc-bin-1.98.1", false), "rustc");
        assert_eq!(app_name("Web Content", "/usr/lib64/firefox/firefox-bin", false), "firefox");
        assert_eq!(app_name("claude", "/home/a/.local/share/claude/versions/2.1.293", false), "claude");
        assert_eq!((plain("-bin"), plain("gtk-4"), plain("x-1.2-bin-3")), ("-bin", "gtk", "x"));
        // What only runs other things is known by what it runs.
        assert_eq!(app_name("Borderlands4.exe", "/usr/bin/wine64-preloader", false), "Borderlands4.exe");
        assert_eq!(app_name("backup.py", "/usr/bin/python3.13", false), "backup.py");
        assert_eq!(app_name("shotwell", "/usr/bin/shotwell", false), "shotwell", "not a shell for starting with sh");
        // Someone else's: the file it runs can't be read.
        assert_eq!(app_name("sshd", "", false), "sshd");
        assert_eq!(app_name("kworker/3:1-events", "", true), "kworker");
    }

    #[test]
    fn users_are_named_from_passwd() {
        let users = parse_passwd("root:x:0:0:root:/root:/bin/bash\nalva:x:1000:1000::/home/alva:/bin/zsh\nbroken line\n");
        assert_eq!(users.get(&1000).map(|n| &**n), Some("alva"));
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn this_machine_is_read_and_this_test_is_in_it() {
        let mut s = ProcSampler::new();
        let first = s.sample(0);
        let me = first.iter().find(|p| p.pid == std::process::id()).expect("this process");
        assert!(!me.kernel && me.memory > 0 && me.threads >= 1 && !me.command.is_empty());
        assert!(first.iter().all(|p| p.cpu == 0.0), "nothing to compare the first look with");
        assert_eq!(started(me.pid), Some(me.started));
        assert_eq!(started(u32::MAX), None);
        // A second look gives everyone a share, and nobody more than all.
        let again = s.sample(100);
        assert!(again.iter().all(|p| (0.0..=100.0).contains(&p.cpu)));
        assert!(s.known.len() <= again.len());
    }
}
