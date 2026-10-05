//! Launcher — pinned favorites + (Phase 2.5) result grid icons.
//!
//! When the search query is empty, we draw a row of pinned app tiles
//! beneath the search input. As the user types, the search/results
//! module takes over the same vertical space.

pub mod context_menu;
pub mod hidden;
pub mod icons;
mod pin_grid;
pub mod pins;

pub use pin_grid::*;

use std::path::PathBuf;

use self::hidden::Hidden;
use self::pins::Pins;
use crate::search::apps::{AppsProvider, DesktopEntry};

pub struct Launcher {
    pins: Pins,
    /// Apps kept in the mini-dock. Its own list, so the dock and the
    /// pinned grid can hold different apps.
    dock: Pins,
    hidden: Hidden,
}

/// One slot in the mixed pinned grid. `App` carries a `DesktopEntry`
/// borrow (same as the legacy code path); `Path` carries a filesystem
/// path the user pinned from the Files view.
pub enum PinnedItem<'a> {
    App(&'a DesktopEntry),
    Path { path: PathBuf, is_dir: bool },
}

impl PinnedItem<'_> {
    /// Human-visible label rendered under the tile.
    pub fn label(&self) -> String {
        match self {
            PinnedItem::App(e) => e.name.clone(),
            PinnedItem::Path { path, .. } => path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        }
    }
}

impl Launcher {
    pub fn new() -> Self {
        let pins = Pins::load();
        let dock = Pins::load_dock(&pins);
        Self {
            pins,
            dock,
            hidden: Hidden::load(),
        }
    }

    #[allow(dead_code)] // used by Phase 2.6 right-click pin/unpin handler
    pub fn pins(&self) -> &Pins {
        &self.pins
    }

    pub fn dock(&self) -> &Pins {
        &self.dock
    }

    pub fn hidden(&self) -> &Hidden {
        &self.hidden
    }

    /// Toggle hidden state for an app_id. Returns whether the app is now hidden.
    pub fn toggle_hidden(&mut self, app_id: &str) -> bool {
        let now_hidden = self.hidden.toggle(app_id);
        tracing::info!(app_id, now_hidden, "hidden toggled");
        now_hidden
    }

    /// Look up the pinned app's `DesktopEntry` from the apps provider.
    /// Returns `None` for pinned ids that are no longer installed
    /// (e.g., the user uninstalled the app); those slots are skipped
    /// in the rendered row. Path entries are skipped entirely.
    pub fn pinned_entries<'a>(&'a self, apps: &'a AppsProvider) -> Vec<&'a DesktopEntry> {
        app_entries(&self.pins, apps)
    }

    /// The apps kept in the dock, in dock order. Same rule as
    /// `pinned_entries`: ids that are no longer installed are skipped.
    pub fn dock_pinned<'a>(&'a self, apps: &'a AppsProvider) -> Vec<&'a DesktopEntry> {
        app_entries(&self.dock, apps)
    }

    /// Resolve the full pin list to a mixed `PinnedItem` list. App
    /// pins whose desktop entry can't be found are skipped; path pins
    /// are always kept.
    pub fn pinned_items<'a>(&'a self, apps: &'a AppsProvider) -> Vec<PinnedItem<'a>> {
        self.pins
            .items()
            .iter()
            .filter_map(|id| {
                if is_path(id) {
                    let path = PathBuf::from(id);
                    let is_dir = path.is_dir();
                    Some(PinnedItem::Path { path, is_dir })
                } else {
                    find_app(apps, id).map(PinnedItem::App)
                }
            })
            .collect()
    }

    /// Toggle pin state for an app_id.
    #[allow(dead_code)] // wired up by right-click handler
    pub fn toggle_pin(&mut self, app_id: &str) {
        let now_pinned = self.pins.toggle(app_id);
        tracing::info!(app_id, now_pinned, "pin toggled");
    }

    /// Toggle whether an app is kept in the dock. New apps join at the
    /// right end of the dock's pinned section.
    pub fn toggle_dock(&mut self, app_id: &str) {
        let now_docked = self.dock.toggle(app_id);
        tracing::info!(app_id, now_docked, "dock toggled");
    }

    /// Reorder pins by index — used by drag-and-drop. Persists on commit.
    ///
    /// `visible_from` / `visible_to` are indices into the **visible**
    /// pin list (what the user sees and drags). `pins.items()` can
    /// contain entries for app_ids whose DesktopEntry isn't currently
    /// installed — those are filtered out of the visible list. We
    /// translate to the raw items-index space before reordering so a
    /// missing entry between two visible pins doesn't shift the move
    /// onto an unrelated slot.
    pub fn reorder_pins(&mut self, visible_from: usize, visible_to: usize, apps: &AppsProvider) {
        reorder_visible(&mut self.pins, visible_from, visible_to, apps, true);
    }

    /// Reorder the dock's kept apps by index — the dock's own
    /// drag-and-drop. Indices are into the visible list, exactly as
    /// for `reorder_pins`.
    pub fn reorder_dock(&mut self, visible_from: usize, visible_to: usize, apps: &AppsProvider) {
        reorder_visible(&mut self.dock, visible_from, visible_to, apps, false);
    }
}

/// Linear scan; app counts are small and pin counts tiny (typically
/// <16), so it's fine.
fn find_app<'a>(apps: &'a AppsProvider, app_id: &str) -> Option<&'a DesktopEntry> {
    (0..apps.count())
        .filter_map(|i| apps.get(i))
        .find(|e| e.app_id == app_id)
}

/// The installed apps in `list`, in list order. Path entries and ids
/// with no DesktopEntry are skipped.
fn app_entries<'a>(list: &'a Pins, apps: &'a AppsProvider) -> Vec<&'a DesktopEntry> {
    list.items()
        .iter()
        .filter(|id| !is_path(id))
        .filter_map(|id| find_app(apps, id))
        .collect()
}

/// Move the entry at visible index `visible_from` to `visible_to`,
/// translating both through `visible_to_items_mapping` first.
fn reorder_visible(
    list: &mut Pins,
    visible_from: usize,
    visible_to: usize,
    apps: &AppsProvider,
    paths_visible: bool,
) {
    let mapping = visible_to_items_mapping(list, apps, paths_visible);
    let visible_len = mapping.len();
    if visible_from >= visible_len {
        return;
    }
    let items_from = mapping[visible_from];
    let items_to = if visible_to >= visible_len {
        list.items().len()
    } else {
        mapping[visible_to]
    };
    list.reorder(items_from, items_to);
    tracing::info!(
        visible_from,
        visible_to,
        items_from,
        items_to,
        "pins reordered"
    );
}

/// Map each *visible* index to the index of the same entry in
/// `list.items()`. App-ids whose DesktopEntry isn't installed are
/// skipped; path entries are visible in the pinned grid
/// (`paths_visible`) and never in the dock.
fn visible_to_items_mapping(list: &Pins, apps: &AppsProvider, paths_visible: bool) -> Vec<usize> {
    list.items()
        .iter()
        .enumerate()
        .filter_map(|(items_idx, id)| {
            let visible = if is_path(id) {
                paths_visible
            } else {
                find_app(apps, id).is_some()
            };
            visible.then_some(items_idx)
        })
        .collect()
}

/// Treat any entry whose first character is `/` as a filesystem path.
pub fn is_path(item: &str) -> bool {
    item.starts_with('/')
}

/// Pick a (cache-key, freedesktop-icon-name) pair for a pinned path
/// tile. Folders resolve to the Lantern `folder` icon (a gold variant
/// of Adwaita's folder shape shipped at `~/.lantern/icons/folder.svg`).
/// Files map by extension to image/video/text-generic mime icons.
pub fn path_icon(path: &std::path::Path, is_dir: bool) -> (String, String) {
    if is_dir {
        return ("__pin_folder".into(), "folder".into());
    }
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "ico"
        | "heic" | "heif" | "avif" => ("__pin_image".into(), "image-x-generic".into()),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "m4v" | "3gp"
        | "ogv" => ("__pin_video".into(), "video-x-generic".into()),
        _ => ("__pin_file".into(), "text-x-generic".into()),
    }
}

