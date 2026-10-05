//! Mini-dock of apps that floats above the screen's bottom edge while
//! the Command Center is collapsed (or animating into collapse). Click
//! an icon to launch.
//!
//! Shows the apps pinned to the dock on the left — the dock's own list
//! (`dock.toml`), separate from the launcher's pinned grid, reordered by
//! dragging (see [`drag`]). Running-but-unpinned apps fill in to the
//! right in FIFO order (first window opened = leftmost), separated by a
//! thin divider. Apps with at least one open window get a small
//! horizontal accent line above the icon as a "running" indicator.
//!
//! Hovering near the dock magnifies icons macOS-style: the icon under
//! the cursor scales to ~1.4×, immediate neighbors to ~1.25×, and the
//! falloff continues smoothly. The plate widens to accommodate.
//!
//! Past the running section, behind a second divider, sits the system
//! tray: small StatusNotifierItem icons for apps that are alive without
//! a window (Steam after its window is closed, Discord, …). Tray icons
//! don't magnify and never affect the app sections' layout beyond
//! widening the plate.

mod drag;
mod preview;

pub use drag::drop_slot;
pub use preview::*;

use lntrn_render::{Color, Painter, Rect};

use crate::app::PinDrag;
use crate::render::IconRequest;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::toplevel::ToplevelInfo;
use crate::tray::TrayItem;

/// Icon side length (logical px).
pub const ICON_SIZE: f32 = 56.0;
/// Gap between icons (logical px).
pub const ICON_GAP: f32 = 20.0;
/// Distance from the screen's bottom edge to the plate's bottom edge
/// (logical px). Mirrors `PANEL_TOP_MARGIN_LOGICAL` so the dock floats
/// the same distance from the bottom as the panel does from the top.
pub const BOTTOM_GAP: f32 = 48.0;
/// Plate corner radius and padding around the icons.
pub const PLATE_RADIUS: f32 = 18.0;
pub const PLATE_PAD: f32 = 10.0;
/// Magnification scale for the icon directly under the cursor. Neighbor
/// icons fall off smoothly toward 1.0 with distance.
pub const MAG_PEAK: f32 = 1.6;
/// How fast magnification falls off (in slot-pitch units). Larger →
/// more icons magnify; smaller → tighter wave.
const MAG_SIGMA: f32 = 1.5;

const PLATE_RGB: (u8, u8, u8) = (24, 24, 24);
const PLATE_ALPHA: f32 = 0.85;
const PLATE_BORDER_ALPHA: f32 = 0.08;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);
const DIVIDER_ALPHA: f32 = 0.18;
/// Running-indicator strip: a thin pill above the icon for any app that
/// has at least one open window.
const INDICATOR_BAR_H: f32 = 3.0;
const INDICATOR_GAP_ABOVE_ICON: f32 = 6.0;
/// System-tray icon side length + spacing (logical px). Roughly half a
/// dock icon — these are status glyphs, not launch targets.
pub const TRAY_ICON_SIZE: f32 = 26.0;
pub const TRAY_ICON_GAP: f32 = 14.0;
/// Extra room either side of the tray divider so the small icons don't
/// crowd the last app icon.
const TRAY_SECTION_PAD: f32 = 6.0;
/// Attention-status ring colour (NeedsAttention items).
const ATTENTION_RGB: (u8, u8, u8) = (0xe0, 0x5a, 0x3a);

/// One slot in the dock. Either an app pinned to the dock or a running
/// app that isn't (appears on the right of the divider).
#[derive(Debug, Clone)]
pub struct DockEntry {
    pub app_id: String,
    pub name: String,
    pub icon_name: Option<String>,
    /// True when this slot is in the dock's list; false when it's
    /// auto-added because the app has at least one open window.
    pub pinned: bool,
}

/// Pre-computed geometry for one dock frame. Cheap to build, used by
/// both rendering and hit-testing so they always agree.
pub struct DockLayout {
    pub entries: Vec<DockEntry>,
    /// Plate background rect (physical px).
    pub plate: Rect,
    /// Drawn icon rect per entry — already magnified, bottom-aligned to
    /// the plate's icon baseline so larger icons grow *upward*.
    pub icons: Vec<Rect>,
    /// Number of pinned entries at the front of `entries`. The divider
    /// sits between index `pinned_count - 1` and `pinned_count` when
    /// both halves are non-empty.
    pub pinned_count: usize,
    /// System-tray items shown after the app sections.
    pub tray: Vec<TrayItem>,
    /// Icon rect per tray item (physical px), parallel to `tray`.
    pub tray_icons: Vec<Rect>,
    /// Scale factor used to build the layout (physical-px per logical-px).
    pub scale: f32,
}

/// Build the merged list of dock entries — pinned first (in user order),
/// then unpinned apps that currently have at least one open window
/// (FIFO by first-seen toplevel).
pub fn dock_entries(
    pinned: &[&DesktopEntry],
    toplevels: &[ToplevelInfo],
    apps: &AppsProvider,
) -> Vec<DockEntry> {
    let mut out: Vec<DockEntry> = pinned
        .iter()
        .map(|e| DockEntry {
            app_id: e.app_id.clone(),
            name: e.name.clone(),
            icon_name: e.icon_name.clone(),
            pinned: true,
        })
        .collect();

    // Insertion-order: iterate toplevels (already in creation order),
    // skip any whose app_id is already represented (pinned or already
    // added as a running entry), append the rest.
    for t in toplevels {
        if t.app_id.is_empty() {
            continue;
        }
        if out.iter().any(|e| e.app_id == t.app_id) {
            continue;
        }
        let (name, icon_name) = lookup_app_meta(apps, &t.app_id);
        out.push(DockEntry {
            app_id: t.app_id.clone(),
            name,
            icon_name,
            pinned: false,
        });
    }
    out
}

fn lookup_app_meta(apps: &AppsProvider, app_id: &str) -> (String, Option<String>) {
    // 1) Exact match on the .desktop stem (the usual case).
    for i in 0..apps.count() {
        if let Some(e) = apps.get(i) {
            if e.app_id.eq_ignore_ascii_case(app_id) {
                return (e.name.clone(), e.icon_name.clone());
            }
        }
    }
    // 2) Fall back to matching the window's Wayland app_id against a
    //    .desktop's `Icon=` name. Some apps set an app_id that differs from
    //    their .desktop filename (e.g. window `lntrn-media-player` vs entry
    //    `org.lantern.MediaPlayer`), but the app_id IS the icon name — so
    //    this still recovers both the proper name and the icon.
    for i in 0..apps.count() {
        if let Some(e) = apps.get(i) {
            if e.icon_name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(app_id))
            {
                return (e.name.clone(), e.icon_name.clone());
            }
        }
    }
    // 3) Last resort: use the app_id itself as the icon name, so any icon
    //    file named after the app_id (all our lntrn-* apps) still resolves.
    (app_id.to_string(), Some(app_id.to_string()))
}

/// Smooth Gaussian-shaped magnification curve. `d_slots` is the cursor's
/// distance to the icon's *base* (un-magnified) center measured in
/// slot-pitch units (one slot = ICON_SIZE + ICON_GAP). Returns a scale
/// factor in `[1.0, MAG_PEAK]`.
fn magnification(d_slots: f32) -> f32 {
    let d = d_slots.abs();
    if d > 4.0 {
        return 1.0;
    }
    1.0 + (MAG_PEAK - 1.0) * (-(d * d) / (MAG_SIGMA * MAG_SIGMA)).exp()
}

/// Build the full dock layout for the current frame. Returns `None`
/// when there's nothing to show (no pinned + no running apps).
///
/// Magnification kicks in only when `cursor_phys` is provided **and**
/// the cursor is within the dock's hover zone — the plate's vertical
/// band plus enough headroom above to cover a fully-magnified icon.
/// Outside that zone the dock renders flat (1.0× everywhere).
pub fn compute_layout(
    panel: Rect,
    surface_h: f32,
    scale: f32,
    pinned: &[&DesktopEntry],
    toplevels: &[ToplevelInfo],
    apps: &AppsProvider,
    tray: &[TrayItem],
    cursor_phys: Option<(f32, f32)>,
) -> Option<DockLayout> {
    let entries = dock_entries(pinned, toplevels, apps);
    if entries.is_empty() && tray.is_empty() {
        return None;
    }

    let icon = ICON_SIZE * scale;
    let gap = ICON_GAP * scale;
    let pad = PLATE_PAD * scale;
    let bottom_gap = BOTTOM_GAP * scale;
    let pinned_count = entries.iter().take_while(|e| e.pinned).count();
    let n = entries.len();

    // Tray section width: divider padding + icons. Zero when empty.
    let tray_icon = TRAY_ICON_SIZE * scale;
    let tray_gap = TRAY_ICON_GAP * scale;
    let tray_n = tray.len();
    let tray_icons_w = tray_n as f32 * tray_icon + (tray_n as f32 - 1.0).max(0.0) * tray_gap;
    let tray_section_w = if tray_n == 0 {
        0.0
    } else if n == 0 {
        tray_icons_w
    } else {
        // gap before divider + divider + gap after, then the icons.
        TRAY_SECTION_PAD * scale * 2.0 + gap + tray_icons_w
    };

    // Base plate (no magnification). We need its left edge to map the
    // cursor into slot-distance space; downstream we re-center the
    // *magnified* plate on the panel center.
    let base_icons_w = n as f32 * icon + (n as f32 - 1.0).max(0.0) * gap;
    let base_plate_w = base_icons_w + tray_section_w + pad * 2.0;
    let center_x = panel.x + panel.w / 2.0;
    let base_plate_x = center_x - base_plate_w / 2.0;
    let slot_pitch = icon + gap;

    // Base plate vertical extent (un-magnified) — used as the y anchor
    // and as the cursor hover-zone reference.
    let base_plate_h = icon + pad * 2.0;
    let base_plate_y = surface_h - bottom_gap - base_plate_h;

    // Vertical hover zone for magnification. We extend it *upward* by
    // the maximum amount a magnified icon overflows the plate so the
    // cursor can hover over a giant icon and keep it big; we also add
    // a small buffer above the plate so approaching from above feels
    // sticky. Below the plate is irrelevant (it's already at the
    // screen edge).
    let mag_overflow_top = (MAG_PEAK - 1.0) * icon;
    let zone_top = base_plate_y - mag_overflow_top - 8.0 * scale;
    let zone_bottom = base_plate_y + base_plate_h;
    let cursor_in_zone = cursor_phys
        .map(|(_, cy)| cy >= zone_top && cy <= zone_bottom)
        .unwrap_or(false);

    // Per-icon magnification factor based on cursor distance — only when
    // the cursor is within the vertical hover zone. Otherwise the dock
    // renders flat.
    let mags: Vec<f32> = (0..n)
        .map(|i| {
            if !cursor_in_zone {
                return 1.0;
            }
            let base_cx = base_plate_x + pad + i as f32 * slot_pitch + icon / 2.0;
            match cursor_phys {
                Some((cx, _)) => magnification((cx - base_cx) / slot_pitch),
                None => 1.0,
            }
        })
        .collect();

    // Magnified plate: width grows to fit larger icons.
    let mag_icons_w: f32 =
        mags.iter().map(|m| icon * m).sum::<f32>() + (n as f32 - 1.0).max(0.0) * gap;
    let plate_w = mag_icons_w + tray_section_w + pad * 2.0;
    let plate_h = icon + pad * 2.0;
    let plate_x = center_x - plate_w / 2.0;
    let plate_y = surface_h - bottom_gap - plate_h;
    let plate = Rect::new(plate_x, plate_y, plate_w, plate_h);

    // Bottom-align icons inside the plate so larger ones grow upward.
    let baseline_y = plate.y + plate.h - pad;
    let mut icons_out: Vec<Rect> = Vec::with_capacity(n);
    let mut x_cursor = plate.x + pad;
    for (i, m) in mags.iter().enumerate() {
        let w = icon * m;
        let h = w; // square icons
        let y = baseline_y - h;
        icons_out.push(Rect::new(x_cursor, y, w, h));
        x_cursor += w;
        if i + 1 < n {
            x_cursor += gap;
        }
    }

    // Tray icons: vertically centred in the plate, flat (no wave).
    let mut tray_icons: Vec<Rect> = Vec::with_capacity(tray_n);
    if tray_n > 0 {
        if n > 0 {
            x_cursor += gap + TRAY_SECTION_PAD * scale * 2.0;
        }
        let y = plate.y + (plate.h - tray_icon) / 2.0;
        for i in 0..tray_n {
            tray_icons.push(Rect::new(x_cursor, y, tray_icon, tray_icon));
            x_cursor += tray_icon;
            if i + 1 < tray_n {
                x_cursor += tray_gap;
            }
        }
    }

    Some(DockLayout {
        entries,
        plate,
        icons: icons_out,
        pinned_count,
        tray: tray.to_vec(),
        tray_icons,
        scale,
    })
}

/// Hit-test the dock. Returns the entry index under `(px, py)` if any.
pub fn hit_test(layout: &DockLayout, px: f32, py: f32) -> Option<usize> {
    for (i, r) in layout.icons.iter().enumerate() {
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(i);
        }
    }
    None
}

/// Hit-test the tray cluster. Returns the tray item index under
/// `(px, py)`. The zone is padded by half the icon gap so the small
/// icons are comfortable targets.
pub fn hit_test_tray(layout: &DockLayout, px: f32, py: f32) -> Option<usize> {
    let pad = TRAY_ICON_GAP * layout.scale * 0.5;
    for (i, r) in layout.tray_icons.iter().enumerate() {
        if px >= r.x - pad && px <= r.x + r.w + pad && py >= r.y - pad && py <= r.y + r.h + pad {
            return Some(i);
        }
    }
    None
}

/// Draw the dock. `drag` is the reorder gesture in flight, if any: once
/// it has started, the dragged icon's slot dims and the drop marker +
/// cursor ghost are drawn on top.
pub fn draw(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    layout: &DockLayout,
    toplevels: &[ToplevelInfo],
    drag: Option<&PinDrag>,
    alpha: f32,
) {
    let drag = drag.filter(|d| d.started && d.from_idx < layout.pinned_count);
    let lifted = drag.map(|d| d.from_idx);
    let scale = layout.scale;
    let plate = layout.plate;
    let radius = PLATE_RADIUS * scale;

    // Plate background + faint border.
    painter.rect_filled(
        plate,
        radius,
        Color::from_rgb8(PLATE_RGB.0, PLATE_RGB.1, PLATE_RGB.2).with_alpha(PLATE_ALPHA * alpha),
    );
    painter.rect_stroke_sdf(
        plate,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(PLATE_BORDER_ALPHA * alpha),
    );

    // Divider between pinned section and unpinned-running section.
    if layout.pinned_count > 0
        && layout.pinned_count < layout.entries.len()
        && layout.pinned_count <= layout.icons.len()
    {
        let left_icon = layout.icons[layout.pinned_count - 1];
        let right_icon = layout.icons[layout.pinned_count];
        let div_x = (left_icon.x + left_icon.w + right_icon.x) / 2.0;
        let div_w = (1.5 * scale).max(1.0);
        let inset = 4.0 * scale;
        let div = Rect::new(
            div_x - div_w / 2.0,
            plate.y + inset,
            div_w,
            plate.h - inset * 2.0,
        );
        painter.rect_filled(
            div,
            div_w * 0.5,
            Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(DIVIDER_ALPHA * alpha),
        );
    }

    // Divider between the app sections and the tray cluster.
    if !layout.tray_icons.is_empty() && !layout.icons.is_empty() {
        let left_icon = layout.icons[layout.icons.len() - 1];
        let right_icon = layout.tray_icons[0];
        let div_x = (left_icon.x + left_icon.w + right_icon.x) / 2.0;
        let div_w = (1.5 * scale).max(1.0);
        let inset = 4.0 * scale;
        painter.rect_filled(
            Rect::new(div_x - div_w / 2.0, plate.y + inset, div_w, plate.h - inset * 2.0),
            div_w * 0.5,
            Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(DIVIDER_ALPHA * alpha),
        );
    }

    // Tray icons. Items that shipped a pixmap are uploaded under their
    // key by the render loop before this frame's icon requests resolve,
    // so the request below hits the cache either way.
    for (i, item) in layout.tray.iter().enumerate() {
        let r = layout.tray_icons[i];
        if item.needs_attention() {
            let ring = 3.0 * scale;
            painter.rect_stroke_sdf(
                Rect::new(r.x - ring, r.y - ring, r.w + ring * 2.0, r.h + ring * 2.0),
                (r.w + ring * 2.0) * 0.3,
                (1.5 * scale).max(1.0),
                Color::from_rgb8(ATTENTION_RGB.0, ATTENTION_RGB.1, ATTENTION_RGB.2)
                    .with_alpha(0.9 * alpha),
            );
        }
        icons.push(IconRequest {
            app_id: item.icon_key(),
            icon_name: item.icon_lookup(),
            x: r.x,
            y: r.y,
            size: r.w,
            opacity: alpha,
            clip: None,
        });
    }

    // Icons.
    for (i, entry) in layout.entries.iter().enumerate() {
        let r = layout.icons[i];

        // No hover highlight here — the magnification grow *is* the hover
        // affordance (see MAG_PEAK).

        icons.push(IconRequest {
            app_id: entry.app_id.clone(),
            icon_name: entry.icon_name.clone(),
            x: r.x,
            y: r.y,
            size: r.w,
            opacity: if lifted == Some(i) {
                alpha * drag::LIFTED_ALPHA
            } else {
                alpha
            },
            clip: None,
        });

        // Running-window indicator: a thin pill *above* the icon for any
        // app with at least one open window. Sits a small gap above the
        // (possibly magnified) icon top edge so it tracks the wave.
        let has_window = toplevels.iter().any(|t| t.app_id == entry.app_id);
        if has_window {
            let indicator_w = r.w;
            let indicator_h = INDICATOR_BAR_H * scale;
            let indicator_x = r.x + (r.w - indicator_w) / 2.0;
            let indicator_y = r.y - INDICATOR_GAP_ABOVE_ICON * scale - indicator_h;
            painter.rect_filled(
                Rect::new(indicator_x, indicator_y, indicator_w, indicator_h),
                indicator_h * 0.5,
                Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(0.95 * alpha),
            );
        }
    }

    if let Some(drag) = drag {
        drag::draw_overlay(painter, icons, layout, drag, alpha);
    }
}

/// All windows whose `app_id` matches the given pinned app's id.
pub fn windows_for_app<'a>(toplevels: &'a [ToplevelInfo], app_id: &str) -> Vec<&'a ToplevelInfo> {
    toplevels.iter().filter(|t| t.app_id == app_id).collect()
}
