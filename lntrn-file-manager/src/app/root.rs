//! Root mode: file operations in one folder go through sudo.
//!
//! It is switched on for a folder, in a tab (or the right pane), and it is
//! kept there: `DirectoryTab::root_dir` names the folder. Root mode is on
//! exactly while the focused pane shows that folder, so it cannot follow
//! the user anywhere:
//!  - every way of leaving (navigation, Back, Forward, Up, another tab, a
//!    new tab, the other pane) clears it on the way out, so coming back
//!    does not bring it back;
//!  - and even a path that forgot to clear it finds root mode off, because
//!    the folder shown is no longer the one it was switched on for
//!    (`expire_root_mode` then drops the stale mark).
//!
//! While it is on the nav bar carries the ROOT badge (`sections/root_badge`)
//! and the content has a frame in the same colour.

use std::path::Path;

use super::{App, DirectoryTab};

impl DirectoryTab {
    /// Root mode is on for the folder this tab shows.
    pub fn root_mode(&self) -> bool {
        self.root_dir.as_deref() == Some(self.path.as_path())
    }

    /// Drop a mark that belongs to a folder the tab has left.
    fn expire_root(&mut self) {
        if !self.root_mode() {
            self.root_dir = None;
        }
    }
}

/// `dir` is the root-mode folder or lies inside it (a row of an expanded
/// folder in the tree view, a folder a file is dropped on).
fn within(root: &Path, dir: &Path) -> bool {
    dir.starts_with(root)
}

/// The same on the disk: a row that is a link to a folder somewhere else is
/// inside by its path and outside by where it leads.
fn really_within(root: &Path, dir: &Path) -> bool {
    match (root.canonicalize(), dir.canonicalize()) {
        (Ok(root), Ok(dir)) => dir.starts_with(root),
        // Not there (yet): the path as written is all there is to go by.
        _ => true,
    }
}

/// Why root mode cannot be switched on for `dir`, if it cannot.
fn refusal(dir: &Path, picking: bool) -> Option<&'static str> {
    if picking {
        // A file chooser serves another program, which will read or write
        // the chosen file with its own rights.
        return Some("Root mode is not available while choosing a file for another app.");
    }
    if crate::fs::is_slow_path(dir) {
        return Some(
            "Root mode is not available on phones and network folders: \
             the administrator account cannot see them.",
        );
    }
    if crate::trash::locate(dir).is_some() {
        return Some("Root mode is not available inside the Trash.");
    }
    None
}

impl App {
    /// Root mode is on for the folder the focused pane shows.
    pub fn root_mode(&self) -> bool {
        self.active_nav_tab_ref().root_dir.as_deref() == Some(self.current_dir.as_path())
    }

    /// Root mode is on and `dir` is its folder or inside it: what is made
    /// or changed there goes through sudo. Anything else (another pane, a
    /// tab, a sidebar place) is never touched as root.
    pub fn root_covers(&self, dir: &Path) -> bool {
        self.root_mode()
            && within(&self.current_dir, dir)
            // Root cannot see a phone or a share a link may lead onto, and
            // asking about one could hang.
            && !crate::fs::is_slow_path(dir)
            && really_within(&self.current_dir, dir)
    }

    /// `root_covers` for the folder an item is in.
    pub fn root_covers_item(&self, item: &Path) -> bool {
        item.parent().is_some_and(|dir| self.root_covers(dir))
    }

    /// Switch root mode on for the folder shown. Says why when it cannot.
    pub fn enter_root_mode(&mut self) {
        if let Some(why) = refusal(&self.current_dir, self.pick.is_some()) {
            self.show_notice("Root mode is not available here", vec![why.to_string()]);
            return;
        }
        let dir = self.current_dir.clone();
        self.active_nav_tab().root_dir = Some(dir);
    }

    /// Switch root mode off. Called by everything that leaves the folder,
    /// before it does, and by the badge and the menu.
    pub fn leave_root_mode(&mut self) {
        self.active_nav_tab().root_dir = None;
    }

    /// The menu's toggle for the folder shown.
    pub fn toggle_root_mode(&mut self) {
        if self.root_mode() {
            self.leave_root_mode();
        } else {
            self.enter_root_mode();
        }
    }

    /// Safety net, run every loop iteration: no tab but the focused one
    /// keeps a root mark, and the focused one only for the folder it shows.
    pub fn expire_root_mode(&mut self) {
        let right_focused = self.split_focused() == Some(super::PaneSide::Right);
        let current = self.current_tab;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if right_focused || i != current {
                tab.root_dir = None;
            } else {
                tab.expire_root();
            }
        }
        if let Some(split) = self.split.as_mut() {
            if right_focused {
                split.right_tab.expire_root();
            } else {
                split.right_tab.root_dir = None;
            }
        }
        // Administrator rights end with the badge: once root mode is off
        // and nothing privileged is in hand, the sudo ticket goes too.
        if self.root_mode() {
            self.root_ticket = true;
        } else if self.root_ticket && self.sudo_prompt.is_none() && self.priv_queue.is_empty() {
            self.root_ticket = false;
            crate::sudo::drop_ticket();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn root_mode_belongs_to_one_folder_and_does_not_come_back() {
        let mut tab = DirectoryTab::new(PathBuf::from("/etc/portage"));
        assert!(!tab.root_mode());
        tab.root_dir = Some(PathBuf::from("/etc/portage"));
        assert!(tab.root_mode());

        // The tab shows another folder (however it got there): off at once.
        tab.path = PathBuf::from("/home/alva/Documents");
        assert!(!tab.root_mode());
        // The stale mark is dropped, so going back does not switch it on.
        tab.expire_root();
        tab.path = PathBuf::from("/etc/portage");
        assert!(!tab.root_mode());

        // A subfolder or the parent is another folder too.
        tab.root_dir = Some(PathBuf::from("/etc/portage"));
        tab.path = PathBuf::from("/etc/portage/sets");
        assert!(!tab.root_mode());
        tab.path = PathBuf::from("/etc");
        assert!(!tab.root_mode());
    }

    #[test]
    fn root_mode_reaches_its_folder_and_below_only() {
        let root = Path::new("/etc/portage");
        assert!(within(root, Path::new("/etc/portage")));
        assert!(within(root, Path::new("/etc/portage/sets")));
        assert!(!within(root, Path::new("/etc")));
        assert!(!within(root, Path::new("/etc/portage2")));
        assert!(!within(root, Path::new("/home/alva")));
    }

    #[test]
    fn a_link_out_of_the_root_folder_is_not_inside_it() {
        let dir = std::env::temp_dir().join(format!("fox-root-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("root/sub")).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("root/link")).unwrap();
        let root = dir.join("root");
        assert!(really_within(&root, &root));
        assert!(really_within(&root, &root.join("sub")));
        // Inside by its path...
        assert!(within(&root, &root.join("link")));
        // ...and outside by where it leads.
        assert!(!really_within(&root, &root.join("link")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn root_mode_is_refused_where_it_makes_no_sense() {
        assert!(refusal(Path::new("/etc"), false).is_none());
        assert!(refusal(Path::new("/etc"), true).is_some());
        let trash = crate::trash::home_trash().files();
        assert!(refusal(&trash, false).is_some());
    }
}
