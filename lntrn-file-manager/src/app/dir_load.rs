//! Off-thread directory listings for slow mounts (MTP phones, sshfs…).
//!
//! `fs::list_directory` is a readdir plus one stat per entry. On a local
//! disk that's microseconds; on jmtpfs each call is an MTP round-trip and,
//! worse, waits on a global device lock that a thumbnail or copy may hold
//! for minutes while it pulls a whole video. So on those mounts the listing
//! runs on a throwaway thread and lands here via `poll_dir_loads`. Each
//! target keeps at most one load in flight, and a result whose directory
//! no longer matches the target (the user navigated on) is dropped.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::fs::{self, FileEntry, SortBy, SortDir};

use super::{App, PaneSide};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DirLoadTarget {
    /// The focused pane (`App::current_dir` / `App::entries`).
    Focused,
    /// A background tab (drag-drop onto a tab-strip entry).
    Tab(usize),
    /// The unfocused split-view pane.
    Inactive,
}

pub struct DirLoad {
    dir: PathBuf,
    target: DirLoadTarget,
    rx: Receiver<Vec<FileEntry>>,
}

impl App {
    pub(super) fn spawn_dir_load(
        &mut self,
        dir: PathBuf,
        target: DirLoadTarget,
        (sort_by, sort_dir): (SortBy, SortDir),
    ) {
        // Supersede any older load for the same target; its result would
        // only be overwritten by this one anyway.
        self.dir_loads.retain(|l| l.target != target);
        let (tx, rx) = mpsc::channel();
        let show_hidden = self.show_hidden;
        let list_dir = dir.clone();
        let spawned = std::thread::Builder::new()
            .name("fox-dir-load".into())
            .spawn(move || {
                let _ = tx.send(fs::list_directory(&list_dir, show_hidden, sort_by, sort_dir));
            });
        if spawned.is_ok() {
            self.dir_loads.push(DirLoad { dir, target, rx });
        }
    }

    /// True while the focused pane is waiting on a listing — drives the
    /// "Loading…" state and keeps the event loop polling.
    pub fn dir_loading(&self) -> bool {
        self.dir_loads
            .iter()
            .any(|l| l.target == DirLoadTarget::Focused)
    }

    /// Install finished listings. Returns true when a view changed.
    pub fn poll_dir_loads(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.dir_loads.len() {
            match self.dir_loads[i].rx.try_recv() {
                Ok(entries) => {
                    let load = self.dir_loads.remove(i);
                    changed |= self.apply_dir_load(load, entries);
                }
                Err(TryRecvError::Empty) => i += 1,
                Err(TryRecvError::Disconnected) => {
                    self.dir_loads.remove(i);
                }
            }
        }
        changed
    }

    fn apply_dir_load(&mut self, load: DirLoad, entries: Vec<FileEntry>) -> bool {
        match load.target {
            DirLoadTarget::Focused => {
                if load.dir != self.current_dir {
                    return false;
                }
                self.apply_listing(entries);
                true
            }
            DirLoadTarget::Tab(idx) => match self.tabs.get_mut(idx) {
                Some(tab) if tab.path == load.dir => {
                    tab.entries = keep_selection(&tab.entries, entries);
                    true
                }
                _ => false,
            },
            DirLoadTarget::Inactive => {
                let Some(split) = self.split.as_mut() else {
                    return false;
                };
                let tab = match split.focused {
                    PaneSide::Left => &mut split.right_tab,
                    PaneSide::Right => &mut self.tabs[self.current_tab],
                };
                if tab.path != load.dir {
                    return false;
                }
                tab.entries = keep_selection(&tab.entries, entries);
                true
            }
        }
    }
}

/// Carry the old listing's selection over to the fresh one.
fn keep_selection(old: &[FileEntry], mut fresh: Vec<FileEntry>) -> Vec<FileEntry> {
    let selected: Vec<&PathBuf> = old.iter().filter(|e| e.selected).map(|e| &e.path).collect();
    if !selected.is_empty() {
        for e in &mut fresh {
            e.selected = selected.contains(&&e.path);
        }
    }
    fresh
}
