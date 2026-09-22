//! System tray — StatusNotifierItem host for the mini dock.
//!
//! Apps that keep running without a window (Steam, Discord, Spotify,
//! Telegram, …) publish a StatusNotifierItem on the session bus. The
//! Command Center claims the watcher name so those items land here,
//! and the mini dock draws them as a small cluster past the running
//! section. Left click activates (or opens the menu for menu-only
//! items), right click fetches the item's dbusmenu and shows it in the
//! regular context-menu widget.
//!
//! [`sni`] is the wire protocol, [`dbusmenu`] the menu protocol,
//! [`worker`] the thread that pumps the bus. This module is the render
//! thread's view: a cached item list plus a tiny command API.

pub mod dbusmenu;
mod sni;
mod worker;

use std::sync::mpsc;
use std::thread;

use crate::launcher::context_menu::MenuItem;

/// ARGB pixmap from an item that ships raw pixels instead of an icon
/// name (Discord, Telegram). Already converted to RGBA.
#[derive(Debug, Clone)]
pub struct IconPixmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Snapshot of one tray item's visible state.
#[derive(Debug, Clone)]
pub struct TrayItem {
    pub bus_name: String,
    pub obj_path: String,
    pub id: String,
    pub title: String,
    pub tooltip: String,
    /// `Active`, `Passive` (hide) or `NeedsAttention`.
    pub status: String,
    pub icon_name: String,
    pub attention_icon_name: String,
    /// Extra directory to search for `icon_name` (Steam points this at
    /// its own `public/` folder).
    pub icon_theme_path: String,
    pub icon_pixmap: Option<IconPixmap>,
    pub attention_pixmap: Option<IconPixmap>,
    pub item_is_menu: bool,
    pub menu_path: Option<String>,
}

impl TrayItem {
    fn new(bus_name: String, obj_path: String) -> Self {
        Self {
            bus_name,
            obj_path,
            id: String::new(),
            title: String::new(),
            tooltip: String::new(),
            status: "Active".into(),
            icon_name: String::new(),
            attention_icon_name: String::new(),
            icon_theme_path: String::new(),
            icon_pixmap: None,
            attention_pixmap: None,
            item_is_menu: false,
            menu_path: None,
        }
    }

    pub fn needs_attention(&self) -> bool {
        self.status == "NeedsAttention"
    }

    pub fn visible(&self) -> bool {
        self.status != "Passive"
    }

    /// The icon name currently in effect (attention variant wins while
    /// the item asks for it).
    fn effective_icon_name(&self) -> &str {
        if self.needs_attention() && !self.attention_icon_name.is_empty() {
            &self.attention_icon_name
        } else {
            &self.icon_name
        }
    }

    /// Pixmap currently in effect, if the item shipped one.
    pub fn effective_pixmap(&self) -> Option<&IconPixmap> {
        if self.needs_attention() && self.attention_pixmap.is_some() {
            self.attention_pixmap.as_ref()
        } else {
            self.icon_pixmap.as_ref()
        }
    }

    /// Cache key for the icon texture. Includes everything that changes
    /// what we draw so a `NewIcon` lands as a fresh cache entry instead
    /// of a stale hit.
    pub fn icon_key(&self) -> String {
        let pm = self
            .effective_pixmap()
            .map(|p| {
                // Cheap content hash so a same-size pixmap swap (status
                // dot colour changes) still invalidates.
                let mut h: u32 = 2166136261;
                for b in p.rgba.iter().step_by(13) {
                    h = (h ^ *b as u32).wrapping_mul(16777619);
                }
                format!("{}x{}:{h:08x}", p.width, p.height)
            })
            .unwrap_or_default();
        format!(
            "tray:{}:{}:{}:{pm}",
            self.bus_name,
            self.effective_icon_name(),
            self.icon_theme_path
        )
    }

    /// What to hand the icon resolver: an absolute path when the item's
    /// theme dir holds the file, otherwise the bare name for the normal
    /// theme search. `None` when only a pixmap is available (the render
    /// loop uploads that directly).
    pub fn icon_lookup(&self) -> Option<String> {
        let name = self.effective_icon_name();
        if name.is_empty() {
            return None;
        }
        if name.starts_with('/') {
            return Some(name.to_string());
        }
        if !self.icon_theme_path.is_empty() {
            let dir = std::path::Path::new(&self.icon_theme_path);
            for ext in ["svg", "png", "svgz"] {
                let p = dir.join(format!("{name}.{ext}"));
                if p.exists() {
                    return Some(p.to_string_lossy().into_owned());
                }
            }
            // Themed layout under the custom dir (`hicolor/48x48/apps/x.png`).
            for size in ["scalable", "64x64", "48x48", "32x32", "24x24", "22x22", "16x16"] {
                for ext in ["svg", "png"] {
                    let p = dir
                        .join("hicolor")
                        .join(size)
                        .join("apps")
                        .join(format!("{name}.{ext}"));
                    if p.exists() {
                        return Some(p.to_string_lossy().into_owned());
                    }
                }
            }
        }
        Some(name.to_string())
    }

    /// Everything that affects rendering, for change detection.
    fn visual_key(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.icon_key(),
            self.status,
            self.title,
            self.tooltip,
            self.menu_path.as_deref().unwrap_or("")
        )
    }
}

/// Render thread → worker.
pub(crate) enum TrayCmd {
    Activate { bus: String, x: i32, y: i32 },
    RequestMenu { bus: String, x: i32, y: i32 },
    MenuClick { bus: String, menu_path: String, item_id: i32 },
}

/// Worker → render thread.
pub(crate) enum TrayEvent {
    Items(Vec<TrayItem>),
    MenuReady {
        bus: String,
        menu_path: String,
        items: Vec<MenuItem>,
    },
}

/// A menu the worker fetched, ready for the click handler to open.
pub struct ReadyMenu {
    pub bus: String,
    pub menu_path: String,
    pub items: Vec<MenuItem>,
    /// Physical-pixel anchor recorded when the menu was requested.
    pub anchor: (f32, f32),
}

pub struct Tray {
    cmd_tx: mpsc::Sender<TrayCmd>,
    event_rx: mpsc::Receiver<TrayEvent>,
    /// Latest item snapshot, visible items only, in registration order.
    pub items: Vec<TrayItem>,
    /// Where the pending menu should open once the worker delivers it.
    pending_menu_anchor: Option<(f32, f32)>,
    /// Menu delivered by the worker and not yet consumed by the loop.
    pub ready_menu: Option<ReadyMenu>,
}

impl Tray {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::Builder::new()
            .name("lcc-tray-sni".into())
            .spawn(move || worker::run(event_tx, cmd_rx))
            .ok();
        Self {
            cmd_tx,
            event_rx,
            items: Vec::new(),
            pending_menu_anchor: None,
            ready_menu: None,
        }
    }

    /// Drain worker events. Returns true when something visible changed
    /// (item list, or a menu became ready to open).
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                TrayEvent::Items(items) => {
                    self.items = items.into_iter().filter(|i| i.visible()).collect();
                    changed = true;
                }
                TrayEvent::MenuReady {
                    bus,
                    menu_path,
                    items,
                } => {
                    let anchor = self.pending_menu_anchor.take().unwrap_or((0.0, 0.0));
                    if !items.is_empty() {
                        self.ready_menu = Some(ReadyMenu {
                            bus,
                            menu_path,
                            items,
                            anchor,
                        });
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Left click. Menu-only items open their menu directly; everything
    /// else gets `Activate`, and the worker falls back to the menu if
    /// the item rejects it.
    pub fn activate(&mut self, bus: &str, anchor: (f32, f32), screen: (i32, i32)) {
        let is_menu = self
            .items
            .iter()
            .find(|i| i.bus_name == bus)
            .map(|i| i.item_is_menu)
            .unwrap_or(false);
        if is_menu {
            self.request_menu(bus, anchor, screen);
            return;
        }
        self.pending_menu_anchor = Some(anchor);
        let _ = self.cmd_tx.send(TrayCmd::Activate {
            bus: bus.to_string(),
            x: screen.0,
            y: screen.1,
        });
    }

    /// Right click: ask the worker for the item's menu. It arrives via
    /// `tick` as `ready_menu`.
    pub fn request_menu(&mut self, bus: &str, anchor: (f32, f32), screen: (i32, i32)) {
        self.pending_menu_anchor = Some(anchor);
        let _ = self.cmd_tx.send(TrayCmd::RequestMenu {
            bus: bus.to_string(),
            x: screen.0,
            y: screen.1,
        });
    }

    pub fn menu_click(&self, bus: &str, menu_path: &str, item_id: i32) {
        let _ = self.cmd_tx.send(TrayCmd::MenuClick {
            bus: bus.to_string(),
            menu_path: menu_path.to_string(),
            item_id,
        });
    }
}

impl Default for Tray {
    fn default() -> Self {
        Self::new()
    }
}
