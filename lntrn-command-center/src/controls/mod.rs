//! Controls row — a strip of small status tiles at the top of the panel.
//!
//! Each control is its own submodule (`clock`, `battery`, `audio`, …)
//! that owns its inline-tile draw + full-content view + any background
//! polling thread it needs.
//!
//! The row is **always visible**. Below the row is the panel's
//! "content area," which is owned by `PanelMode` (see `app.rs`):
//!
//! - `PanelMode::Launcher` → search input + pinned apps + results
//! - `PanelMode::Control(TileId)` → that control's full-content view
//!
//! Clicking a tile switches the mode to that control. Clicking the
//! same tile again, or pressing Esc, returns to `Launcher`.

pub mod audio;
pub mod battery;
pub mod bluetooth;
pub mod brightness;
pub mod clock;
pub mod collapse;
pub mod disk;
pub mod events;
pub mod gaming;
pub mod gaming_ipc;
pub mod gpu;
pub mod network;
pub mod sysmon;
pub mod temp;
pub mod terminal_header;
pub mod tile;
pub mod toolbar;
pub mod toolbar_edit;
pub mod widget_settings;
pub mod wifi;
pub mod workspace;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use self::audio::Audio;
use self::battery::Battery;
use self::bluetooth::Bluetooth;
use self::brightness::Brightness;
use self::clock::Clock;
use self::disk::Disk;
use self::events::Events;
use self::sysmon::SysMon;
use self::wifi::Wifi;

/// Total logical height the controls row reserves at the top of the
/// panel, *not* including the underline.
pub const ROW_HEIGHT: f32 = 60.0;
pub const ROW_TOP_MARGIN: f32 = 12.0;
pub const ROW_HORIZONTAL_PAD: f32 = 24.0;
pub const ROW_UNDERLINE_HEIGHT: f32 = 2.0;
pub const ROW_UNDERLINE_GAP: f32 = 8.0;

/// Total logical y-space the controls row claims at the top of the panel.
pub const fn total_logical_height() -> f32 {
    ROW_TOP_MARGIN + ROW_HEIGHT + ROW_UNDERLINE_GAP + ROW_UNDERLINE_HEIGHT
}

/// Identifier for any tile in the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TileId {
    /// Active workspace number in a rounded box.
    Workspace,
    Clock,
    Audio,
    Brightness,
    Wifi,
    Bluetooth,
    Battery,
    SysMon,
    /// CPU package temperature. Pulls state from `controls.sysmon` so
    /// only the sysmon worker polls /sys/class/thermal.
    Temp,
    /// Network throughput (↓/↑). Reads RX/TX history from `controls.sysmon`.
    Network,
    /// GPU utilization / temp / VRAM. Reads from `controls.sysmon`.
    Gpu,
    /// Disk usage for `/` and `~`. Backed by its own lightweight
    /// `controls.disk` statfs poller.
    Disk,
    /// Gaming Mode toggle — mirrors the compositor's Super+G state over
    /// the gaming IPC socket; click to flip. No expanded view.
    Gaming,
    /// Chevron button that toggles the panel between full and
    /// just-the-row "mini" modes. Special-cased in click handling.
    Collapse,
    /// "Clear" button (Terminal view header). Reserved — pattern-matched
    /// for future wiring but not currently constructed anywhere.
    #[allow(dead_code)]
    TerminalClear,
}

/// Every widget the user can arrange in the toolbar (everything except
/// the chrome: the collapse chevron + terminal-clear button).
pub fn arrangeable_widgets() -> &'static [TileId] {
    &[
        TileId::Workspace,
        TileId::Clock,
        TileId::Audio,
        TileId::Brightness,
        TileId::Wifi,
        TileId::Bluetooth,
        TileId::Battery,
        TileId::SysMon,
        TileId::Temp,
        TileId::Network,
        TileId::Gpu,
        TileId::Disk,
        TileId::Gaming,
    ]
}

impl TileId {
    /// Tiles whose click opens the shared System Monitor expanded view
    /// (CPU / RAM / NET graphs + process list). All the "system stats"
    /// tiles funnel here instead of each owning a near-identical view.
    pub fn opens_sysmon_view(self) -> bool {
        matches!(
            self,
            TileId::SysMon | TileId::Temp | TileId::Network | TileId::Gpu | TileId::Disk
        )
    }

    /// Stable string key used in the persisted toolbar layout.
    pub fn config_key(self) -> &'static str {
        match self {
            TileId::Workspace => "workspace",
            TileId::Clock => "clock",
            TileId::Audio => "volume",
            TileId::Brightness => "brightness",
            TileId::Wifi => "wifi",
            TileId::Bluetooth => "bluetooth",
            TileId::Battery => "battery",
            TileId::SysMon => "sysmon",
            TileId::Temp => "temp",
            TileId::Network => "network",
            TileId::Gpu => "gpu",
            TileId::Disk => "disk",
            TileId::Gaming => "gaming",
            TileId::Collapse => "collapse",
            TileId::TerminalClear => "terminal_clear",
        }
    }

    pub fn from_config_key(s: &str) -> Option<Self> {
        Some(match s {
            "workspace" => TileId::Workspace,
            "clock" => TileId::Clock,
            "volume" => TileId::Audio,
            "brightness" => TileId::Brightness,
            "wifi" => TileId::Wifi,
            "bluetooth" => TileId::Bluetooth,
            "battery" => TileId::Battery,
            "sysmon" => TileId::SysMon,
            "temp" => TileId::Temp,
            "network" => TileId::Network,
            "gpu" => TileId::Gpu,
            "disk" => TileId::Disk,
            "gaming" => TileId::Gaming,
            _ => return None,
        })
    }

    /// Human-friendly label for the widget (used in the edit-mode tray).
    pub fn display_name(self) -> &'static str {
        match self {
            TileId::Workspace => "Workspace",
            TileId::Clock => "Clock",
            TileId::Audio => "Volume",
            TileId::Brightness => "Brightness",
            TileId::Wifi => "Wi-Fi",
            TileId::Bluetooth => "Bluetooth",
            TileId::Battery => "Battery",
            TileId::SysMon => "System",
            TileId::Temp => "Temp",
            TileId::Network => "Network",
            TileId::Gpu => "GPU",
            TileId::Disk => "Disk",
            TileId::Gaming => "Gaming",
            TileId::Collapse => "Collapse",
            TileId::TerminalClear => "Clear",
        }
    }

    /// Preferred logical width when laid out in the row.
    pub fn logical_width(self) -> f32 {
        match self {
            TileId::Workspace => workspace::TILE_WIDTH,
            TileId::Clock => clock::TILE_WIDTH,
            TileId::Audio => audio::TILE_WIDTH,
            TileId::Brightness => brightness::TILE_WIDTH,
            TileId::Wifi => wifi::TILE_WIDTH,
            TileId::Bluetooth => bluetooth::TILE_WIDTH,
            TileId::Battery => battery::TILE_WIDTH,
            TileId::SysMon => sysmon::TILE_WIDTH,
            TileId::Temp => temp::TILE_WIDTH,
            TileId::Network => network::TILE_WIDTH,
            TileId::Gpu => gpu::TILE_WIDTH,
            TileId::Disk => disk::TILE_WIDTH,
            TileId::Gaming => gaming::TILE_WIDTH,
            TileId::Collapse => collapse::TILE_WIDTH,
            TileId::TerminalClear => terminal_header::TILE_WIDTH,
        }
    }
}

/// Top-level controls state. The "which view is showing" decision lives
/// in `app::AppState::mode`, not here — this struct is purely the
/// per-control backends + tile rendering.
pub struct Controls {
    pub clock: Clock,
    pub events: Events,
    pub audio: Audio,
    pub brightness: Brightness,
    pub wifi: Wifi,
    pub bluetooth: Bluetooth,
    pub battery: Battery,
    pub sysmon: SysMon,
    pub disk: Disk,
    /// Gaming Mode state mirror + toggle channel (compositor IPC).
    pub gaming_ipc: gaming_ipc::GamingIpc,
    /// User-customizable widget arrangement (zones + order + on/off).
    pub toolbar: toolbar::ToolbarLayout,
}

impl Controls {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            events: Events::load(),
            audio: Audio::new(),
            brightness: Brightness::new(),
            wifi: Wifi::new(),
            bluetooth: Bluetooth::new(),
            battery: Battery::new(),
            sysmon: SysMon::new(),
            disk: Disk::new(),
            gaming_ipc: gaming_ipc::GamingIpc::new(),
            toolbar: toolbar::ToolbarLayout::load(),
        }
    }

    /// Whether a widget's backend is available on this machine right now.
    /// Disabled-but-present widgets still appear in the edit-mode tray;
    /// not-present ones are hidden entirely.
    pub fn widget_present(&self, id: TileId) -> bool {
        match id {
            TileId::Workspace | TileId::Clock => true,
            TileId::Audio => self.audio.is_present(),
            TileId::Brightness => self.brightness.is_present(),
            TileId::Wifi => self.wifi.is_present(),
            TileId::Bluetooth => self.bluetooth.is_present(),
            TileId::Battery => self.battery.is_present(),
            // Network / Temp ride on the (always-present) sysmon worker.
            TileId::SysMon | TileId::Network | TileId::Temp => self.sysmon.is_present(),
            TileId::Gpu => self.sysmon.has_gpu(),
            TileId::Disk => self.disk.is_present(),
            // Always present — when the compositor socket is down the tile
            // renders dimmed rather than vanishing.
            TileId::Gaming => true,
            TileId::Collapse | TileId::TerminalClear => true,
        }
    }

    /// A widget's effective logical width: its content width (clock varies
    /// with its options) scaled by the per-widget size multiplier, plus
    /// any extra spacing padded around it.
    pub fn widget_width(&self, id: TileId) -> f32 {
        let opts = self.toolbar.opts(id);
        let base = if id == TileId::Clock {
            clock::tile_width(&opts.clock)
        } else {
            id.logical_width()
        };
        base * opts.size + opts.space
    }

    /// Re-poll any sysfs / D-Bus / etc. backed state. Cheap to call
    /// every frame; each control rate-limits its own re-reads.
    pub fn tick(&mut self) {
        self.audio.tick();
        self.brightness.tick();
        self.wifi.tick();
        self.bluetooth.tick();
        self.battery.tick();
        self.disk.tick();
        self.gaming_ipc.poll();
    }

    /// Ordered list of widgets currently rendered in the row, paired with
    /// their layout slot (zone + width). In the Default view this comes
    /// from the user's `toolbar` layout (presence-filtered); Terminal and
    /// Files own their own row, so only the collapse chevron rides along.
    fn tile_slots(&self, panel_view: crate::app::PanelView) -> Vec<(TileId, tile::Slot)> {
        let mut out: Vec<(TileId, tile::Slot)> = Vec::new();

        if !matches!(panel_view, crate::app::PanelView::Default) {
            out.push((
                TileId::Collapse,
                tile::Slot { zone: tile::Zone::Right, logical_width: collapse::TILE_WIDTH },
            ));
            return out;
        }

        for &id in &self.toolbar.left {
            if self.widget_present(id) {
                out.push((id, tile::Slot { zone: tile::Zone::Left, logical_width: self.widget_width(id) }));
            }
        }
        for &id in &self.toolbar.middle {
            if self.widget_present(id) {
                out.push((id, tile::Slot { zone: tile::Zone::Middle, logical_width: self.widget_width(id) }));
            }
        }
        for &id in &self.toolbar.right {
            if self.widget_present(id) {
                out.push((id, tile::Slot { zone: tile::Zone::Right, logical_width: self.widget_width(id) }));
            }
        }

        // Collapse chevron is panel chrome — always pinned to the far
        // right, after the user's right-zone widgets.
        out.push((
            TileId::Collapse,
            tile::Slot { zone: tile::Zone::Right, logical_width: collapse::TILE_WIDTH },
        ));
        out
    }

    /// Hit-test a click against the controls row.
    pub fn hit_test(
        &self,
        panel: Rect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
        panel_view: crate::app::PanelView,
    ) -> Option<TileId> {
        let slots = self.tile_slots(panel_view);
        let just_slots: Vec<_> = slots.iter().map(|(_, s)| *s).collect();
        let layouts = tile::pack(panel, scale, &just_slots);
        for ((id, _), layout) in slots.iter().zip(layouts.iter()) {
            if layout.contains(phys_x, phys_y) {
                return Some(*id);
            }
        }
        None
    }

    /// Every widget's resolved (id, physical layout) pair for a view,
    /// in render order (includes the Collapse chevron). Used by edit mode
    /// for hit-testing + drop resolution.
    pub fn widget_layouts(
        &self,
        panel: Rect,
        scale: f32,
        panel_view: crate::app::PanelView,
    ) -> Vec<(TileId, tile::TileLayout)> {
        let slots = self.tile_slots(panel_view);
        let just_slots: Vec<_> = slots.iter().map(|(_, s)| *s).collect();
        let layouts = tile::pack(panel, scale, &just_slots);
        slots.iter().map(|(id, _)| *id).zip(layouts).collect()
    }

    /// Resolved physical-pixel layout for a specific tile, if it's
    /// currently present in the row.
    pub fn tile_layout(
        &self,
        id: TileId,
        panel: Rect,
        scale: f32,
        panel_view: crate::app::PanelView,
    ) -> Option<tile::TileLayout> {
        let slots = self.tile_slots(panel_view);
        let just_slots: Vec<_> = slots.iter().map(|(_, s)| *s).collect();
        let layouts = tile::pack(panel, scale, &just_slots);
        slots
            .iter()
            .zip(layouts.iter())
            .find(|((tid, _), _)| *tid == id)
            .map(|(_, layout)| *layout)
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Phys-px y-coordinate where the panel's content area starts (just
/// below the controls-row underline). Used by every "view" renderer.
pub fn content_top_y(panel: Rect, scale: f32) -> f32 {
    // Add the split-panel gap (zero in normal mode) so body content
    // sits below the visible gap between bar and body window.
    panel.y + total_logical_height() * scale + crate::app::split_gap_px()
}

/// Draw the controls row + underline only. The body of the panel is
/// drawn separately by `crate::render::draw_content` based on the
/// current `PanelMode`. `selected_tile` highlights one tile (the one
/// whose view is currently showing).
#[allow(clippy::too_many_arguments)]
pub fn draw_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    controls: &Controls,
    selected_tile: Option<TileId>,
    hovered_tile: Option<TileId>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
    icons: &mut Vec<crate::render::IconRequest>,
    panel_view: crate::app::PanelView,
    workspace_num: Option<u32>,
) {
    let pad = ROW_HORIZONTAL_PAD * scale;
    let row_top = panel.y + ROW_TOP_MARGIN * scale;
    let row_h = ROW_HEIGHT * scale;
    let underline_y = row_top + row_h + ROW_UNDERLINE_GAP * scale;

    let slot_pairs = controls.tile_slots(panel_view);
    let just_slots: Vec<_> = slot_pairs.iter().map(|(_, s)| *s).collect();
    let layouts = tile::pack(panel, scale, &just_slots);

    for ((id, _), layout) in slot_pairs.iter().zip(layouts.iter()) {
        // Gold-icon highlight: a tile reads as "lit" when it's hovered
        // OR when the panel is currently showing its expanded view.
        // We compute it here and pass it into each tile so the icon
        // itself recolors, matching the waffle-button pattern.
        let is_hovered = hovered_tile == Some(*id);
        // Every navigable tile lights up when its expanded view is open.
        // Collapse / TerminalClear never become the selected mode, so a
        // simple equality check covers them by default.
        let is_active = selected_tile == Some(*id);
        let lit = is_hovered || is_active;

        // Per-widget size: the slot was packed at this widget's size, so
        // draw its content at the matching scale (Collapse is chrome and
        // never scaled). The widget's extra `space` is reserved as equal
        // padding on both sides by insetting the draw rect, so it gaps the
        // widget from its neighbours symmetrically.
        let wopts = controls.toolbar.opts(*id);
        let ws = scale * wopts.size;
        let pad = wopts.space * scale * 0.5;
        let dl = tile::TileLayout {
            x: layout.x + pad,
            y: layout.y,
            w: (layout.w - pad * 2.0).max(1.0),
            h: layout.h,
        };
        let layout = &dl;

        match id {
            TileId::Workspace => workspace::draw_inline(
                painter, text, layout, ws, alpha, surface_w, surface_h, workspace_num,
            ),
            TileId::Clock => clock::draw_inline(
                painter, text, &controls.clock, layout, ws, alpha, surface_w, surface_h, lit,
                &wopts.clock,
            ),
            TileId::Audio => audio::draw_inline(
                painter, text, &controls.audio, layout, ws, alpha, surface_w, surface_h, lit,
            ),
            TileId::Brightness => brightness::draw_inline(
                painter, text, &controls.brightness, layout, ws, alpha, surface_w, surface_h, lit,
            ),
            TileId::Wifi => wifi::draw_inline(
                painter, text, &controls.wifi, layout, ws, alpha, surface_w, surface_h, lit,
            ),
            TileId::Bluetooth => bluetooth::draw_inline(
                painter, text, &controls.bluetooth, layout, ws, alpha, surface_w, surface_h, lit,
            ),
            TileId::Battery => battery::draw_inline(
                painter, text, &controls.battery, layout, ws, alpha, surface_w, surface_h,
            ),
            TileId::SysMon => sysmon::tile::draw_inline(
                painter, text, icons, &controls.sysmon, layout, ws, alpha, surface_w, surface_h,
            ),
            TileId::Temp => temp::draw_inline(
                painter, text, icons, &controls.sysmon, layout, ws, alpha, surface_w, surface_h,
            ),
            TileId::Network => network::draw_inline(
                painter, text, icons, &controls.sysmon, layout, ws, alpha, surface_w, surface_h,
            ),
            TileId::Gpu => gpu::draw_inline(
                painter, text, icons, &controls.sysmon, layout, ws, alpha, surface_w, surface_h,
            ),
            TileId::Disk => disk::draw_inline(
                painter, text, &controls.disk, layout, ws, alpha, surface_w, surface_h,
            ),
            TileId::Gaming => gaming::draw_inline(
                painter, text, &controls.gaming_ipc, layout, ws, alpha, surface_w, surface_h, lit,
            ),
            TileId::Collapse => {} // drawn separately so we can pass `collapsed` state
            TileId::TerminalClear => {
                terminal_header::draw_inline(painter, text, layout, ws, alpha)
            }
        }
    }

    // Underline beneath the row.
    painter.rect_filled(
        Rect::new(
            panel.x + pad,
            underline_y,
            panel.w - pad * 2.0,
            ROW_UNDERLINE_HEIGHT * scale,
        ),
        1.0 * scale,
        Color::rgba(1.0, 1.0, 1.0, 0.18 * alpha),
    );
}

/// Draw the full-content view for `tile_id`. This fills the panel's
/// content area below the controls row.
pub fn draw_view(
    painter: &mut Painter,
    text: &mut TextRenderer,
    controls: &Controls,
    tile_id: TileId,
    panel: Rect,
    scale: f32,
    alpha: f32,
    text_size: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let top_y = content_top_y(panel, scale);
    let _bottom = match tile_id {
        TileId::Clock => clock::draw_view(
            painter, text, &controls.clock, &controls.events, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
        TileId::Audio => audio::draw_view(
            painter, text, &controls.audio, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
        TileId::Brightness => brightness::draw_view(
            painter, text, &controls.brightness, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
        TileId::Wifi => wifi::draw_view(
            painter, text, &controls.wifi, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
        TileId::Bluetooth => bluetooth::draw_view(
            painter, text, &controls.bluetooth, panel, top_y, scale, alpha, text_size, surface_w, surface_h,
        ),
        TileId::Battery => battery::draw_view(
            painter, text, &controls.battery, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
        TileId::SysMon => sysmon::view::draw_view(
            painter, text, &controls.sysmon, panel, top_y, scale, alpha, text_size, surface_w, surface_h,
        ),
        // Temp / Network / GPU / Disk all share the System Monitor
        // expanded view — clicking any of them opens the same panel with
        // CPU / mem / net history + the process list.
        TileId::Temp | TileId::Network | TileId::Gpu | TileId::Disk => sysmon::view::draw_view(
            painter, text, &controls.sysmon, panel, top_y, scale, alpha, text_size, surface_w, surface_h,
        ),
        // Tiles that don't open an expanded view — their clicks are
        // intercepted in the input handler before reaching here.
        TileId::Workspace | TileId::Gaming | TileId::Collapse | TileId::TerminalClear => top_y,
    };
}
