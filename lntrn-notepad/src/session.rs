//! What Notepad keeps between runs so nothing is ever lost to a closed
//! window: a draft of every document with unsaved work, and the list of
//! what was open. Both live in `~/.lantern/config/lntrn-notepad/`.
//!
//! More than one Notepad can run. Each keeps drafts of its own
//! documents; the first one running is the one that brings back what was
//! open and writes the list.
//!
//! Drafts are written by a thread of their own: a document handed to it
//! is a copy that shares its paragraphs, so handing over costs nothing
//! and typing never waits on the disk.

use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};

use lntrn_data::{Doc as Data, toml};

use crate::doc::{Doc, Pos};
use crate::io;

/// Where it all is.
pub fn dir() -> PathBuf {
    #[cfg(test)]
    crate::sandboxed();
    lntrn_sys::dirs::lantern_config().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config")).join("lntrn-notepad")
}

fn drafts_dir() -> PathBuf {
    dir().join("drafts")
}

/// Where this run keeps draft `id`. The file's name says whose it is
/// (`<id>.<pid>.lnote`): a draft belongs to the Notepad that wrote it
/// for as long as that one is running.
pub fn draft_path(id: u64) -> PathBuf {
    drafts_dir().join(format!("{id:016x}.{}.lnote", std::process::id()))
}

/// Whether process `pid` is a Notepad that is running.
fn alive(pid: u32) -> bool {
    let exe = |pid: u32| std::fs::read_link(format!("/proc/{pid}/exe")).ok().and_then(|p| p.file_name().map(|n| n.to_owned()));
    pid == std::process::id() || exe(pid).is_some_and(|theirs| Some(theirs) == exe(std::process::id()))
}

/// Every draft in the folder: its number, its file, and whether the
/// Notepad that wrote it is still running.
fn drafts() -> Vec<(u64, PathBuf, bool)> {
    let Ok(files) = std::fs::read_dir(drafts_dir()) else { return Vec::new() };
    let parse = |path: PathBuf| {
        let name = path.file_name()?.to_str()?.strip_suffix(".lnote")?.to_owned();
        let (id, pid) = name.split_once('.')?;
        Some((u64::from_str_radix(id, 16).ok()?, alive(pid.parse().ok()?), path))
    };
    files.flatten().filter_map(|f| parse(f.path())).map(|(id, owned, path)| (id, path, owned)).collect()
}

/// Take over draft `id`, left by a Notepad that has gone: read it, and
/// make its file this run's. `None` when there is no such draft, when a
/// running Notepad still has it, or when it can't be read.
pub fn adopt(id: u64) -> Option<Doc> {
    let (_, path, _) = drafts().into_iter().find(|(draft, path, owned)| *draft == id && (!*owned || *path == draft_path(id)))?;
    let doc = std::fs::read_to_string(&path).ok().and_then(|text| io::lnote::read(&text))?;
    if path != draft_path(id) {
        let _ = std::fs::rename(&path, draft_path(id));
    }
    Some(doc)
}

/// Drafts nobody running has and no remembered tab names: left by a
/// crash, or by a window closed with work in it. They come back as tabs
/// too.
pub fn strays(known: &[Entry]) -> Vec<u64> {
    let mut ids: Vec<u64> = drafts().into_iter().filter(|(id, _, owned)| !owned && !known.iter().any(|e| e.draft == Some(*id))).map(|(id, _, _)| id).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Whether this run is the one that remembers what was open: the first
/// Notepad running. One started while it runs is a window of its own
/// that brings nothing back and leaves the list alone.
pub fn claim() -> bool {
    let file = dir().join("instance.pid");
    let held = std::fs::read_to_string(&file).ok().and_then(|pid| pid.trim().parse::<u32>().ok()).is_some_and(|pid| pid != std::process::id() && alive(pid));
    if held {
        return false;
    }
    io::write_whole(&file, std::process::id().to_string().as_bytes()).is_ok()
}

/// A number for a new draft: the clock, and a count so two made in the
/// same instant differ.
pub fn new_draft_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    (nanos ^ (u64::from(std::process::id()) << 40)).wrapping_add(COUNT.fetch_add(1, Ordering::Relaxed)).max(1)
}

/// A tab as it is remembered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// The file it is of; none for one never saved.
    pub path: Option<PathBuf>,
    /// Its draft, when it has unsaved work.
    pub draft: Option<u64>,
    /// What an unsaved one is called.
    pub name: String,
    pub caret: Pos,
    pub scroll: f64,
    /// It was the one in front.
    pub front: bool,
}

fn entry_data(e: &Entry) -> Data {
    let mut tab = Data::map();
    tab.set("name", Data::Str(e.name.clone()));
    if let Some(path) = &e.path {
        tab.set("path", Data::Str(path.display().to_string()));
    }
    if let Some(draft) = e.draft {
        tab.set("draft", Data::Str(format!("{draft:016x}")));
    }
    tab.set("para", Data::Int(e.caret.para as i64));
    tab.set("byte", Data::Int(e.caret.byte as i64));
    tab.set("scroll", Data::Float(e.scroll.round()));
    if e.front {
        tab.set("front", Data::Bool(true));
    }
    tab
}

fn entry_from(tab: &Data) -> Entry {
    let text = |key: &str| tab.get(key).and_then(Data::as_str).unwrap_or("");
    let number = |key: &str| tab.get(key).and_then(Data::as_f64).filter(|n| n.is_finite() && *n >= 0.0).unwrap_or(0.0);
    Entry { path: Some(text("path")).filter(|p| !p.is_empty()).map(PathBuf::from), draft: u64::from_str_radix(text("draft"), 16).ok(), name: text("name").to_owned(), caret: Pos::new(number("para") as usize, number("byte") as usize), scroll: number("scroll"), front: tab.get("front").and_then(Data::as_bool).unwrap_or(false) }
}

pub fn write(entries: &[Entry]) -> String {
    let mut doc = Data::map();
    doc.set("tab", Data::List(entries.iter().map(entry_data).collect()));
    toml::write(&doc)
}

pub fn read(text: &str) -> Vec<Entry> {
    let Ok(doc) = toml::parse(text) else { return Vec::new() };
    doc.get("tab").and_then(Data::as_list).map(|tabs| tabs.iter().map(entry_from).collect()).unwrap_or_default()
}

/// The tabs that were open, as last remembered.
pub fn load() -> Vec<Entry> {
    std::fs::read_to_string(dir().join("session.toml")).map(|text| read(&text)).unwrap_or_default()
}

pub fn save(entries: &[Entry]) -> Result<(), String> {
    io::write_whole(&dir().join("session.toml"), write(entries).as_bytes())
}

enum Job {
    Write(u64, Doc),
    Remove(u64),
}

/// The thread that writes drafts.
pub struct Saver {
    jobs: Option<Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn run(job: Job) {
    let done = match &job {
        Job::Write(id, doc) => io::write_whole(&draft_path(*id), crate::io::lnote::write(doc).as_bytes()),
        Job::Remove(id) => match std::fs::remove_file(draft_path(*id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    };
    if let Err(e) = done {
        lntrn_core::log_warn!("draft: {e}");
    }
}

impl Default for Saver {
    fn default() -> Self {
        let (jobs, todo) = channel::<Job>();
        let thread = std::thread::Builder::new().name("drafts".into()).spawn(move || todo.into_iter().for_each(run)).ok();
        Saver { jobs: Some(jobs), thread }
    }
}

impl Saver {
    fn send(&self, job: Job) {
        // With no thread to give it to, it is done here.
        match &self.jobs {
            Some(jobs) if self.thread.is_some() => drop(jobs.send(job)),
            _ => run(job),
        }
    }

    /// Keep `doc` as draft `id`.
    pub fn keep(&self, id: u64, doc: &Doc) {
        self.send(Job::Write(id, doc.clone()));
    }

    /// Draft `id` is not needed any more.
    pub fn forget(&self, id: u64) {
        self.send(Job::Remove(id));
    }

    /// Wait until everything handed over is on the disk: before the
    /// window closes, and before a test looks.
    pub fn flush(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        *self = Saver::default();
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_open_is_remembered_as_it_was() {
        let entries = vec![Entry { path: Some(PathBuf::from("/home/a/My Notes/list.lnote")), draft: Some(0x9f3a), name: "list.lnote".into(), caret: Pos::new(3, 10), scroll: 120.0, front: false }, Entry { path: None, draft: Some(7), name: "Untitled 2".into(), caret: Pos::default(), scroll: 0.0, front: true }, Entry { path: Some(PathBuf::from("/tmp/plain.txt")), name: "plain.txt".into(), ..Entry::default() }];
        assert_eq!(read(&write(&entries)), entries);
        assert_eq!((read(""), read("not toml [")), (vec![], vec![]));
        // Nonsense is put right rather than believed.
        let odd = read("[[tab]]\nname = \"x\"\npara = -4\nbyte = 2.9\ndraft = \"zz\"\npath = \"\"\n");
        assert_eq!(odd, [Entry { name: "x".into(), caret: Pos::new(0, 2), ..Entry::default() }]);
        let (a, b) = (new_draft_id(), new_draft_id());
        assert!(a != b && a != 0);
    }
}
