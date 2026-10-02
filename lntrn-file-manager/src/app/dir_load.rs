//! Off-thread directory listings for slow mounts (MTP phones, sshfs…).
//!
//! `fs::list_directory` is a readdir plus one stat per entry. On a local
//! disk that's microseconds; on jmtpfs each call is an MTP round-trip and,
//! worse, waits on a global device lock that a thumbnail or copy may hold
//! for minutes while it pulls a whole video. So on those mounts the listing
//! runs on a worker (`bg::Task`) and lands here.
//!
//! A load belongs to its DIRECTORY, not to whichever pane happened to be
//! focused when it was asked for: when it lands it is handed to every tab,
//! pane and tree that shows that directory at that moment. Switching tab or
//! pane while it runs, closing or reordering tabs, two tabs on one folder:
//! none of them can lose it or misdeliver it.
//!
//! One directory is never listed twice at once (`Flights`). A refresh asked
//! for while its listing runs is remembered and done afterwards, after a
//! short rest, so that a burst of reloads (a copy landing file after file
//! in the other pane) costs the device one listing every couple of seconds
//! instead of one thread per reload.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::bg::{Polled, Task};
use crate::fs::{self, FileEntry, SortBy, SortDir};

use super::{App, PaneSide, ViewMode};

/// Pause between two listings of one slow directory that nobody is waiting
/// on (a refresh, not a navigation).
const REST: Duration = Duration::from_secs(2);

struct Flight<K> {
    key: K,
    running: bool,
    /// Asked for again since the current (or last) run started.
    again: bool,
    rest_until: Instant,
}

/// Single-flight bookkeeping: which keys have a run in the air, which want
/// another one, and when that may start.
pub(super) struct Flights<K> {
    slots: Vec<Flight<K>>,
}

impl<K: PartialEq + Clone> Flights<K> {
    pub(super) fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Ask for a run of `key`. True: start it now. False: one is running or
    /// resting; unless `urgent`, another run is remembered as wanted.
    ///
    /// `urgent` is for a view with nothing to show (the user is looking at
    /// "Loading…"): it skips the rest, and it is satisfied by the run that
    /// is already in the air. A refresh is not: the running listing may
    /// predate the change that asked for it.
    pub(super) fn request(&mut self, key: &K, urgent: bool, now: Instant) -> bool {
        let Some(slot) = self.slots.iter_mut().find(|s| &s.key == key) else {
            self.slots.push(Flight {
                key: key.clone(),
                running: true,
                again: false,
                rest_until: now,
            });
            return true;
        };
        if slot.running {
            if !urgent {
                slot.again = true;
            }
            return false;
        }
        if urgent || now >= slot.rest_until {
            slot.running = true;
            slot.again = false;
            return true;
        }
        slot.again = true;
        false
    }

    /// The run of `key` ended (with or without a result).
    pub(super) fn finished(&mut self, key: &K, now: Instant) {
        if let Some(slot) = self.slots.iter_mut().find(|s| &s.key == key) {
            slot.running = false;
            slot.rest_until = now + REST;
        }
    }

    /// Keys that were asked for again and have rested: they are marked
    /// running, the caller starts them (or calls `finished` if it will not).
    /// Rested keys nobody asked for are forgotten.
    pub(super) fn due(&mut self, now: Instant) -> Vec<K> {
        self.slots
            .retain(|s| s.running || s.again || now < s.rest_until);
        let mut start = Vec::new();
        for slot in &mut self.slots {
            if !slot.running && slot.again && now >= slot.rest_until {
                slot.running = true;
                slot.again = false;
                start.push(slot.key.clone());
            }
        }
        start
    }

    pub(super) fn is_running<Q>(&self, key: &Q) -> bool
    where
        K: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.slots
            .iter()
            .any(|s| s.running && s.key.borrow() == key)
    }
}

struct Load {
    dir: PathBuf,
    show_hidden: bool,
    /// `None`: the directory could not be read.
    task: Task<Option<Vec<FileEntry>>>,
}

pub struct DirLoads {
    flights: Flights<PathBuf>,
    running: Vec<Load>,
    /// "Show hidden files" as of the last reload.
    hidden_seen: Option<bool>,
    /// Set for the length of one reload: every request in it is urgent.
    all_urgent: bool,
}

impl DirLoads {
    pub(super) fn new() -> Self {
        Self {
            flights: Flights::new(),
            running: Vec::new(),
            hidden_seen: None,
            all_urgent: false,
        }
    }

    /// Start of a reload. When "show hidden files" was switched since the
    /// last one, what the slow panes show answers the old question and the
    /// user is waiting for the new one: this reload does not rest.
    pub(super) fn begin_reload(&mut self, show_hidden: bool) {
        self.all_urgent = self.hidden_seen.is_some_and(|seen| seen != show_hidden);
        self.hidden_seen = Some(show_hidden);
    }

    /// Start of a reload after a change that makes what is shown wrong,
    /// not merely old (a picker's file-type filter was switched).
    pub(super) fn begin_urgent_reload(&mut self, show_hidden: bool) {
        self.all_urgent = true;
        self.hidden_seen = Some(show_hidden);
    }

    pub(super) fn end_reload(&mut self) {
        self.all_urgent = false;
    }
}

impl App {
    /// List `dir` on a worker and deliver the result to whoever shows it
    /// then. `urgent`: a view is empty and waiting for this listing.
    pub(super) fn request_dir_load(&mut self, dir: PathBuf, urgent: bool) {
        let urgent = urgent || self.dir_loads.all_urgent;
        if self.dir_loads.flights.request(&dir, urgent, Instant::now()) {
            self.spawn_dir_load(dir);
        }
    }

    fn spawn_dir_load(&mut self, dir: PathBuf) {
        let show_hidden = self.show_hidden;
        let list_dir = dir.clone();
        // Unsorted: each view that receives it applies its own sort.
        let task = Task::spawn("fox-dir-load", move || {
            fs::read_directory(&list_dir, show_hidden)
        });
        self.dir_loads.running.push(Load {
            dir,
            show_hidden,
            task,
        });
    }

    /// True while the focused pane is waiting on a listing — drives the
    /// "Loading…" state and keeps the event loop polling.
    pub fn dir_loading(&self) -> bool {
        self.dir_is_loading(&self.current_dir)
    }

    /// A listing of `dir` is in the air.
    pub fn dir_is_loading(&self, dir: &Path) -> bool {
        self.dir_loads.flights.is_running(dir)
    }

    /// Install finished listings and start the refreshes whose rest is
    /// over. Returns true when a view changed.
    pub fn poll_dir_loads(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.dir_loads.running.len() {
            let polled = self.dir_loads.running[i].task.poll();
            if matches!(polled, Polled::Pending) {
                i += 1;
                continue;
            }
            let load = self.dir_loads.running.remove(i);
            self.dir_loads.flights.finished(&load.dir, Instant::now());
            // "Loading…" has to go even when nothing is installed.
            changed = true;
            let Polled::Ready(listing) = polled else {
                continue;
            };
            if load.show_hidden != self.show_hidden {
                // Hidden files were switched while it ran: this listing
                // answers the old question. Ask again.
                if self.shows_dir(&load.dir) {
                    self.request_dir_load(load.dir, true);
                }
                continue;
            }
            crate::fs::note_listing(&load.dir, listing.is_some());
            match listing {
                Some(listing) => self.deliver_listing(&load.dir, listing),
                // It could not be read (a phone unplugged, a share gone).
                // What is on screen stays rather than turn into "empty";
                // a view with nothing yet says that it cannot be read.
                None if self.current_dir == load.dir && !self.entries.is_empty() => {
                    self.set_status_note(
                        "This folder could not be read just now. Showing what was listed before.",
                    );
                }
                None => self.deliver_listing(&load.dir, Vec::new()),
            }
        }
        for dir in self.dir_loads.flights.due(Instant::now()) {
            if self.shows_dir(&dir) {
                self.spawn_dir_load(dir);
            } else {
                self.dir_loads.flights.finished(&dir, Instant::now());
            }
        }
        changed
    }

    /// The left pane's sort: the flat fields while it is focused, parked
    /// otherwise. Background tabs belong to the left pane.
    fn left_sort(&self) -> (SortBy, SortDir) {
        match self.split.as_ref() {
            Some(split) if split.focused == PaneSide::Right => {
                (split.parked_view.sort_by, split.parked_view.sort_dir)
            }
            _ => (self.sort_by, self.sort_dir),
        }
    }

    /// Some tab, pane or tree shows `dir` right now.
    fn shows_dir(&self, dir: &Path) -> bool {
        self.current_dir == dir
            || self.tabs.iter().any(|t| t.path == dir)
            || self.split.as_ref().is_some_and(|s| s.right_tab.path == dir)
            || self.tree_wants(dir)
    }

    /// Hand a fresh listing of `dir` to everything that shows it.
    fn deliver_listing(&mut self, dir: &Path, listing: Vec<FileEntry>) {
        let sorted = |(by, order): (SortBy, SortDir)| {
            let mut entries = listing.clone();
            fs::sort_entries(&mut entries, by, order);
            entries
        };

        // Expanded folders of a tree view (and a picker's fixed root) read
        // their rows from this cache; see app/tree.rs.
        let in_tree = self.tree_wants(dir);
        if in_tree {
            self.tree_cache.insert(dir.to_path_buf(), listing.clone());
        }

        // Background tabs.
        let left_sort = self.left_sort();
        for i in 0..self.tabs.len() {
            // The current tab is a pane, handled below.
            if i == self.current_tab || self.tabs[i].path != dir {
                continue;
            }
            let fresh = keep_selection(&self.tabs[i].entries, sorted(left_sort));
            self.tabs[i].entries = fresh;
        }

        // The unfocused pane.
        let inactive_shows = self
            .inactive_pane()
            .is_some_and(|(tab, _, _)| tab.path == dir);
        if inactive_shows {
            let sort = self
                .split
                .as_ref()
                .map(|s| (s.parked_view.sort_by, s.parked_view.sort_dir))
                .unwrap_or(left_sort);
            self.install_inactive_listing(sorted(sort));
        } else if in_tree && self.split.is_some() {
            self.rebuild_parked_tree();
        }

        // The focused pane (the flat fields).
        if self.current_dir == dir {
            // Rebuilds the tree as well.
            self.apply_listing(sorted((self.sort_by, self.sort_dir)));
        } else if in_tree && self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }
}

/// Carry the old listing's selection over to the fresh one.
pub(super) fn keep_selection(old: &[FileEntry], mut fresh: Vec<FileEntry>) -> Vec<FileEntry> {
    let selected: std::collections::HashSet<&PathBuf> =
        old.iter().filter(|e| e.selected).map(|e| &e.path).collect();
    if !selected.is_empty() {
        for e in &mut fresh {
            e.selected = selected.contains(&e.path);
        }
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn one_directory_is_never_listed_twice_at_once() {
        let t0 = Instant::now();
        let mut f: Flights<&str> = Flights::new();
        assert!(f.request(&"phone", false, t0));
        assert!(f.is_running(&"phone"));
        // A storm of reloads while it runs starts nothing…
        for ms in 0..50 {
            assert!(!f.request(&"phone", false, at(t0, ms * 100)));
        }
        // …and another directory is not held up by it.
        assert!(f.request(&"other", false, t0));

        // It is remembered that one more is wanted, once, after a rest.
        f.finished(&"phone", at(t0, 5_000));
        assert!(!f.is_running(&"phone"));
        assert!(f.due(at(t0, 5_100)).is_empty());
        assert_eq!(f.due(at(t0, 5_000) + REST), vec!["phone"]);
        assert!(f.is_running(&"phone"));
        assert!(f.due(at(t0, 5_000) + REST).is_empty());
    }

    #[test]
    fn a_run_nobody_asked_about_again_is_not_repeated() {
        let t0 = Instant::now();
        let mut f: Flights<&str> = Flights::new();
        assert!(f.request(&"dir", false, t0));
        f.finished(&"dir", at(t0, 100));
        assert!(f.due(at(t0, 100) + REST).is_empty());
        // Forgotten after its rest: the next request starts at once.
        assert!(f.request(&"dir", false, at(t0, 100) + REST));
    }

    #[test]
    fn a_refresh_during_the_rest_waits_and_a_waiting_view_does_not() {
        let t0 = Instant::now();
        let mut f: Flights<&str> = Flights::new();
        assert!(f.request(&"dir", false, t0));
        f.finished(&"dir", at(t0, 100));

        // Refresh inside the rest: deferred, then due.
        assert!(!f.request(&"dir", false, at(t0, 200)));
        assert_eq!(f.due(at(t0, 100) + REST), vec!["dir"]);
        f.finished(&"dir", at(t0, 3_000));

        // A view with nothing to show does not wait out the rest.
        assert!(f.request(&"dir", true, at(t0, 3_001)));
        // And while that runs, a second waiting view starts no second
        // listing and asks for no repeat: the one in the air serves it.
        assert!(!f.request(&"dir", true, at(t0, 3_002)));
        f.finished(&"dir", at(t0, 4_000));
        assert!(f.due(at(t0, 4_000) + REST).is_empty());
    }

    #[test]
    fn selection_survives_a_fresh_listing() {
        let entry = |name: &str, selected: bool| FileEntry {
            name: name.into(),
            path: PathBuf::from("/d").join(name),
            is_dir: false,
            size: 0,
            modified: None,
            is_symlink: false,
            selected,
            folder_icon: None,
            folder_color: None,
        };
        let old = vec![entry("a", true), entry("b", false), entry("gone", true)];
        let fresh = keep_selection(&old, vec![entry("b", false), entry("a", false)]);
        assert!(!fresh[0].selected);
        assert!(fresh[1].selected);
    }
}
