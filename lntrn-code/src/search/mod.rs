//! Project-wide search: the query run over every file of the project on
//! a thread (so typing never waits on the disk), hits grouped by file and
//! sent back as each file finishes; open unsaved documents are searched
//! as they are in the editor, not on disk. The view is in [`view`].

pub mod view;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use lntrn_app::Waker;

/// One match: where it is in the file, and the line it is on for the
/// list (a window of it when the line is very long).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// 0-based.
    pub line: usize,
    /// Byte offset in the line, and the match's byte length.
    pub col: usize,
    pub len: usize,
    pub preview: String,
    /// Byte offset of the match in `preview`.
    pub pcol: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<Hit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
}

enum Msg {
    File(u64, FileHits),
    Done { generation: u64, files: usize, capped: bool },
}

/// The most hits one search reports.
const HIT_CAP: usize = 5000;
/// Files larger than this are not read.
const FILE_CAP: u64 = 2 * 1024 * 1024;
/// A preview keeps this much of a long line before and after the match.
const PREVIEW_AROUND: usize = 60;

pub struct Search {
    pub query: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub results: Vec<FileHits>,
    pub total: usize,
    pub capped: bool,
    pub running: bool,
    /// Files the last (or current) search looked at.
    pub files_seen: usize,
    /// Files whose hits are folded away in the list.
    pub collapsed: HashSet<PathBuf>,
    /// The view should put the caret in the query field.
    pub want_focus: bool,
    /// The query changed: run when the typing pauses (frame time).
    pub run_at: Option<f64>,
    /// What the shown results were searched for.
    pub shown_for: Option<Query>,
    rx: Option<Receiver<Msg>>,
    generation: Arc<AtomicU64>,
}

impl Default for Search {
    fn default() -> Self {
        Self { query: String::new(), match_case: false, whole_word: false, results: Vec::new(), total: 0, capped: false, running: false, files_seen: 0, collapsed: HashSet::new(), want_focus: false, run_at: None, shown_for: None, rx: None, generation: Arc::new(AtomicU64::new(0)) }
    }
}

impl Search {
    pub fn current_query(&self) -> Query {
        Query { text: self.query.clone(), match_case: self.match_case, whole_word: self.whole_word }
    }

    /// Start the search over `files` (`overrides` are files whose text is
    /// given rather than read). Any search still running is dropped.
    pub fn start(&mut self, files: Vec<PathBuf>, overrides: Vec<(PathBuf, String)>, waker: Option<Waker>) {
        let query = self.current_query();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.results.clear();
        self.total = 0;
        self.capped = false;
        self.files_seen = 0;
        self.collapsed.clear();
        self.run_at = None;
        self.shown_for = Some(query.clone());
        if query.text.is_empty() {
            self.running = false;
            self.rx = None;
            return;
        }
        self.running = true;
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let gen_cell = Arc::clone(&self.generation);
        let spawned = std::thread::Builder::new().name("search".into()).spawn(move || run(files, overrides, query, generation, &gen_cell, &tx, waker.as_ref()));
        if spawned.is_err() {
            self.running = false;
            self.rx = None;
        }
    }

    /// Take in what the thread found. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        let current = self.generation.load(Ordering::SeqCst);
        let mut changed = false;
        let mut done = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::File(g, f) if g == current => {
                    self.total += f.hits.len();
                    self.results.push(f);
                    changed = true;
                }
                Msg::Done { generation, files, capped } if generation == current => {
                    self.files_seen = files;
                    self.capped = capped;
                    self.running = false;
                    done = true;
                    changed = true;
                }
                _ => {}
            }
        }
        if done {
            self.rx = None;
        }
        changed
    }
}

fn run(files: Vec<PathBuf>, overrides: Vec<(PathBuf, String)>, query: Query, generation: u64, gen_cell: &AtomicU64, tx: &Sender<Msg>, waker: Option<&Waker>) {
    let mut total = 0;
    let mut seen = 0;
    let mut capped = false;
    let mut probe = [0u8; 8192];
    for path in &files {
        if gen_cell.load(Ordering::Relaxed) != generation {
            return;
        }
        let text = match overrides.iter().find(|(p, _)| p == path) {
            Some((_, t)) => t.clone(),
            None => match read_text(path, &mut probe) {
                Some(t) => t,
                None => continue,
            },
        };
        seen += 1;
        let hits = find_in(&text, &query);
        if hits.is_empty() {
            continue;
        }
        total += hits.len();
        if tx.send(Msg::File(generation, FileHits { path: path.clone(), hits })).is_err() {
            return;
        }
        if let Some(w) = waker {
            w.wake();
        }
        if total >= HIT_CAP {
            capped = true;
            break;
        }
    }
    let _ = tx.send(Msg::Done { generation, files: seen, capped });
    if let Some(w) = waker {
        w.wake();
    }
}

/// The file as text, unless it is large or binary.
fn read_text(path: &std::path::Path, probe: &mut [u8]) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > FILE_CAP {
        return None;
    }
    let n = f.read(probe).ok()?;
    if probe[..n].contains(&0) {
        return None;
    }
    let mut bytes = probe[..n].to_vec();
    f.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

/// Every place `needle` occurs in `hay` (byte offsets), by the query's
/// case and word rules.
pub fn find_all(hay: &str, needle: &str, match_case: bool, whole_word: bool) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    let ascii = needle.is_ascii();
    // A non-ASCII needle without case: lowercase both, and trust the
    // offsets only when lowercasing kept the length.
    let lowered;
    let (h, n, hay_bytes) = if !match_case && !ascii {
        lowered = (hay.to_lowercase(), needle.to_lowercase());
        if lowered.0.len() == hay.len() { (lowered.0.as_bytes(), lowered.1.as_bytes(), h) } else { (h, n, h) }
    } else {
        (h, n, h)
    };
    let mut i = 0;
    while i + n.len() <= h.len() {
        let window = &h[i..i + n.len()];
        let same = if match_case || !ascii { window == n } else { window.eq_ignore_ascii_case(n) };
        if same && hay.is_char_boundary(i) && hay.is_char_boundary(i + n.len()) {
            let bounded = !whole_word || ((i == 0 || !is_word(hay_bytes[i - 1])) && (i + n.len() == hay_bytes.len() || !is_word(hay_bytes[i + n.len()])));
            if bounded {
                out.push(i);
                i += n.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The hits in one file's text.
pub fn find_in(text: &str, q: &Query) -> Vec<Hit> {
    let mut out = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        for col in find_all(line, &q.text, q.match_case, q.whole_word) {
            let (preview, pcol) = preview_of(line, col, q.text.len());
            out.push(Hit { line: line_no, col, len: q.text.len(), preview, pcol });
            if out.len() >= HIT_CAP {
                return out;
            }
        }
    }
    out
}

/// The line for the list: whole when short, else a window around the
/// match. Leading whitespace is dropped either way.
fn preview_of(line: &str, col: usize, len: usize) -> (String, usize) {
    let lead = line.len() - line.trim_start().len();
    let start = lead.min(col);
    if line.len() - start <= PREVIEW_AROUND * 3 {
        return (line[start..].to_owned(), col - start);
    }
    let mut a = col.saturating_sub(PREVIEW_AROUND).max(start);
    while !line.is_char_boundary(a) {
        a -= 1;
    }
    let mut b = (col + len + PREVIEW_AROUND).min(line.len());
    while !line.is_char_boundary(b) {
        b += 1;
    }
    let mut s = String::new();
    if a > start {
        s.push('…');
    }
    let pcol = s.len() + (col - a);
    s.push_str(&line[a..b]);
    if b < line.len() {
        s.push('…');
    }
    (s, pcol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_rules() {
        assert_eq!(find_all("Foo foo FOO", "foo", false, false), vec![0, 4, 8]);
        assert_eq!(find_all("Foo foo FOO", "foo", true, false), vec![4]);
        assert_eq!(find_all("foobar foo_bar foo", "foo", false, true), vec![15]);
        assert_eq!(find_all("aaaa", "aa", true, false), vec![0, 2], "no overlapping hits");
        assert_eq!(find_all("Ünïcode ünïcode", "ünïcode", false, false), vec![0, 10]);
        assert_eq!(find_all("naïve", "ï", true, false), vec![2]);
        assert!(find_all("short", "longer needle", false, false).is_empty());
        assert!(find_all("x", "", false, false).is_empty());
    }

    #[test]
    fn hits_with_previews() {
        let q = Query { text: "needle".into(), match_case: false, whole_word: false };
        let text = "    let needle = 1;\nnothing\n".to_owned() + &"x".repeat(300) + "needle" + &"y".repeat(300);
        let hits = find_in(&text, &q);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].line, hits[0].col, hits[0].preview.as_str(), hits[0].pcol), (0, 8, "let needle = 1;", 4));
        let h = &hits[1];
        assert_eq!((h.line, h.col), (2, 300));
        assert!(h.preview.starts_with('…') && h.preview.ends_with('…'));
        assert_eq!(&h.preview[h.pcol..h.pcol + 6], "needle");
    }

    #[test]
    fn runs_on_a_thread() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn alpha() {}\nfn beta() { alpha() }\n").unwrap();
        std::fs::write(dir.join("b.txt"), "no match here\n").unwrap();
        std::fs::write(dir.join("bin"), b"\0\0alpha").unwrap();
        std::fs::write(dir.join("c.rs"), "stale on disk\n").unwrap();
        let mut s = Search { query: "alpha".into(), ..Search::default() };
        let files = vec![dir.join("a.rs"), dir.join("b.txt"), dir.join("bin"), dir.join("c.rs")];
        s.start(files, vec![(dir.join("c.rs"), "alpha in the editor\n".into())], None);
        for _ in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            s.poll();
            if !s.running {
                break;
            }
        }
        assert!(!s.running, "the search finished");
        assert_eq!((s.results.len(), s.total, s.files_seen), (2, 3, 3), "binary skipped, override searched");
        assert_eq!(s.results[0].hits.len(), 2);
        assert_eq!(s.results[1].hits[0].preview, "alpha in the editor");
        s.query.clear();
        s.start(Vec::new(), Vec::new(), None);
        assert!(s.results.is_empty() && !s.running);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
