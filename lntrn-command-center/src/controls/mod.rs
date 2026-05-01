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
pub mod events;
pub mod tile;
pub mod wifi;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use self::audio::Audio;
use self::battery::Battery;
use self::bluetooth::Bluetooth;
use self::brightness::Brightness;
use self::clock::Clock;
use self::events::Events;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileId {
    Clock,
    Audio,
    Brightness,
    Wifi,
    Bluetooth,
    Battery,
    // Mpris, Power
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
        }
    }

    /// Re-poll any sysfs / D-Bus / etc. backed state. Cheap to call
    /// every frame; each control rate-limits its own re-reads.
    pub fn tick(&mut self) {
        self.audio.tick();
        self.brightness.tick();
        self.wifi.tick();
        self.bluetooth.tick();
        self.battery.tick();
    }

    /// Ordered list of tiles currently rendered in the row, paired with
    /// their layout slot (side + width). Clock + audio + brightness
    /// dock to the left, battery to the right.
    fn tile_slots(&self) -> Vec<(TileId, tile::Slot)> {
        let mut out: Vec<(TileId, tile::Slot)> = Vec::new();
        out.push((
            TileId::Clock,
            tile::Slot { side: tile::Side::Left, logical_width: clock::TILE_WIDTH },
        ));
        if self.audio.is_present() {
            out.push((
                TileId::Audio,
                tile::Slot { side: tile::Side::Left, logical_width: audio::TILE_WIDTH },
            ));
        }
        if self.brightness.is_present() {
            out.push((
                TileId::Brightness,
                tile::Slot {
                    side: tile::Side::Left,
                    logical_width: brightness::TILE_WIDTH,
                },
            ));
        }
        if self.wifi.is_present() {
            out.push((
                TileId::Wifi,
                tile::Slot {
                    side: tile::Side::Left,
                    logical_width: wifi::TILE_WIDTH,
                },
            ));
        }
        if self.bluetooth.is_present() {
            out.push((
                TileId::Bluetooth,
                tile::Slot {
                    side: tile::Side::Left,
                    logical_width: bluetooth::TILE_WIDTH,
                },
            ));
        }
        if self.battery.is_present() {
            out.push((
                TileId::Battery,
                tile::Slot { side: tile::Side::Right, logical_width: battery::TILE_WIDTH },
            ));
        }
        out
    }

    /// Hit-test a click against the controls row.
    pub fn hit_test(&self, panel: Rect, scale: f32, phys_x: f32, phys_y: f32) -> Option<TileId> {
        let slots = self.tile_slots();
        let just_slots: Vec<_> = slots.iter().map(|(_, s)| *s).collect();
        let layouts = tile::pack(panel, scale, &just_slots);
        for ((id, _), layout) in slots.iter().zip(layouts.iter()) {
            if layout.contains(phys_x, phys_y) {
                return Some(*id);
            }
        }
        None
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Phys-px y-coordinate where the panel's content area starts (just
/// below the controls-row underline). Used by every "view" renderer.
pub fn content_top_y(panel: Rect, scale: f32) -> f32 {
    panel.y + total_logical_height() * scale
}

/// Draw the controls row + underline only. The body of the panel is
/// drawn separately by `crate::render::draw_content` based on the
/// current `PanelMode`. `selected_tile` highlights one tile (the one
/// whose view is currently showing).
pub fn draw_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    controls: &Controls,
    selected_tile: Option<TileId>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = ROW_HORIZONTAL_PAD * scale;
    let row_top = panel.y + ROW_TOP_MARGIN * scale;
    let row_h = ROW_HEIGHT * scale;
    let underline_y = row_top + row_h + ROW_UNDERLINE_GAP * scale;

    let slot_pairs = controls.tile_slots();
    let just_slots: Vec<_> = slot_pairs.iter().map(|(_, s)| *s).collect();
    let layouts = tile::pack(panel, scale, &just_slots);

    for ((id, _), layout) in slot_pairs.iter().zip(layouts.iter()) {
        let _is_selected = selected_tile == Some(*id);
        // Selection highlight is currently subtle / off; a future
        // iteration could draw a faint accent dot or underline beneath
        // the active tile.

        match id {
            TileId::Clock => clock::draw_inline(
                painter, text, &controls.clock, layout, scale, alpha, surface_w, surface_h,
            ),
            TileId::Audio => audio::draw_inline(
                painter, text, &controls.audio, layout, scale, alpha, surface_w, surface_h,
            ),
            TileId::Brightness => brightness::draw_inline(
                painter, text, &controls.brightness, layout, scale, alpha, surface_w, surface_h,
            ),
            TileId::Wifi => wifi::draw_inline(
                painter, text, &controls.wifi, layout, scale, alpha, surface_w, surface_h,
            ),
            TileId::Bluetooth => bluetooth::draw_inline(
                painter, text, &controls.bluetooth, layout, scale, alpha, surface_w, surface_h,
            ),
            TileId::Battery => battery::draw_inline(
                painter, text, &controls.battery, layout, scale, alpha, surface_w, surface_h,
            ),
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
            painter, text, &controls.bluetooth, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
        TileId::Battery => battery::draw_view(
            painter, text, &controls.battery, panel, top_y, scale, alpha, surface_w, surface_h,
        ),
    };
}
