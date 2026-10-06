//! Layout math for the Bluetooth view — the rects, row heights and
//! scroll geometry the draw and hit-test paths share. No painter / text
//! calls here.

use lntrn_render::Rect;

use super::detail;
use super::prompt::{prompt_extra_height, row_prompt};
use super::{Bluetooth, Device};

const VIEW_TOP_PAD: f32 = 24.0;
pub(super) const VIEW_HEADER_FONT: f32 = 22.0;
const VIEW_HEADER_BOTTOM_GAP: f32 = 16.0;

const TOGGLES_ROW_HEIGHT: f32 = 36.0;
pub(super) const TOGGLES_ROW_FONT: f32 = 16.0;
const TOGGLES_ROW_BOTTOM_GAP: f32 = 16.0;
const TOGGLE_W: f32 = 44.0;
const TOGGLE_H: f32 = 24.0;
const TOGGLE_LABEL_GAP: f32 = 8.0;
const TOGGLES_INTER_GAP: f32 = 24.0;

pub(super) const SECTION_HEADER_FONT: f32 = 14.0;
pub(super) const SECTION_HEADER_BOTTOM_GAP: f32 = 6.0;
pub(super) const SECTION_GAP: f32 = 12.0;

pub(super) const ROW_HEIGHT: f32 = 56.0;
pub(super) const ROW_INNER_PAD: f32 = 16.0;
pub(super) const ROW_RIGHT_GAP: f32 = 12.0;
/// Cap on rendered rows per section. The list scrolls, so this is purely
/// a "don't lay out a whole stadium of advertisers" sanity bound.
pub(super) const MAX_SECTION_ROWS: usize = 64;
/// Bottom padding the list reserves so the last row doesn't hug the
/// panel edge.
const LIST_BOTTOM_PAD: f32 = 16.0;

pub(super) fn header_row_y(panel_top_y: f32, scale: f32) -> f32 {
    panel_top_y + VIEW_TOP_PAD * scale
}

pub(super) fn toggles_row_y(panel_top_y: f32, scale: f32) -> f32 {
    header_row_y(panel_top_y, scale) + VIEW_HEADER_FONT * scale + VIEW_HEADER_BOTTOM_GAP * scale
}

/// Y-coordinate (physical px) where the scrolling device list starts.
/// The header and the toggles row sit above it and never scroll.
fn list_top_y(panel_top_y: f32, scale: f32) -> f32 {
    toggles_row_y(panel_top_y, scale) + TOGGLES_ROW_HEIGHT * scale + TOGGLES_ROW_BOTTOM_GAP * scale
}

/// Power-toggle pill rect at the top right of the view.
pub(super) fn toggle_rect(panel: Rect, panel_top_y: f32, scale: f32) -> Rect {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_w = panel.w - pad * 2.0;
    let toggle_w = TOGGLE_W * scale;
    let toggle_h = TOGGLE_H * scale;
    let header_font = VIEW_HEADER_FONT * scale;
    let header_y = header_row_y(panel_top_y, scale);
    let toggle_x = panel.x + pad + inner_w - toggle_w;
    let toggle_y = header_y + (header_font - toggle_h) / 2.0;
    Rect::new(toggle_x, toggle_y, toggle_w, toggle_h)
}

/// Layout of the discoverable + scan toggles in the second header row.
pub(super) struct TogglesRow {
    pub discoverable_label: Rect,
    pub discoverable_toggle: Rect,
    pub scan_label: Rect,
    pub scan_toggle: Rect,
}

pub(super) fn toggles_row_layout(panel: Rect, panel_top_y: f32, scale: f32) -> TogglesRow {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let row_y = toggles_row_y(panel_top_y, scale);
    let row_h = TOGGLES_ROW_HEIGHT * scale;
    let label_font = TOGGLES_ROW_FONT * scale;
    let toggle_w = TOGGLE_W * scale;
    let toggle_h = TOGGLE_H * scale;
    let label_gap = TOGGLE_LABEL_GAP * scale;
    let inter = TOGGLES_INTER_GAP * scale;

    // Tight estimates of label widths so the layout is deterministic
    // without measuring text. Slightly generous.
    let disc_label_w = label_font * 7.5; // "Discoverable"
    let scan_label_w = label_font * 4.0; // "Scan"

    let disc_label_x = inner_x;
    let disc_toggle_x = disc_label_x + disc_label_w + label_gap;
    let scan_label_x = disc_toggle_x + toggle_w + inter;
    let scan_toggle_x = scan_label_x + scan_label_w + label_gap;

    let label_y = row_y + (row_h - label_font) / 2.0;
    let toggle_y = row_y + (row_h - toggle_h) / 2.0;

    TogglesRow {
        discoverable_label: Rect::new(disc_label_x, label_y, disc_label_w, label_font),
        discoverable_toggle: Rect::new(disc_toggle_x, toggle_y, toggle_w, toggle_h),
        scan_label: Rect::new(scan_label_x, label_y, scan_label_w, label_font),
        scan_toggle: Rect::new(scan_toggle_x, toggle_y, toggle_w, toggle_h),
    }
}

/// Body font for the expanded view + inline strips, scaled off the
/// user's Text Size setting so device rows honour what they picked.
pub(super) fn body_font(text_size: f32, scale: f32) -> f32 {
    (text_size.max(12.0)) * scale
}

/// Total extra height a device row contributes below its header: the
/// inline prompt strip (if any) plus the expanded-detail block (if
/// expanded). A row with a live request always shows its strip, even
/// when not toggled open.
pub(super) fn row_extra_height(bt: &Bluetooth, dev: &Device, text_size: f32, scale: f32) -> f32 {
    let mut h = prompt_extra_height(bt, dev, text_size, scale);
    if bt.expanded_mac.as_deref() == Some(dev.mac.as_str()) {
        h += detail::expanded_extra_height(dev, text_size, scale);
    }
    h
}

fn header_row_rect(inner_x: f32, inner_w: f32, row_y: f32, scale: f32) -> Rect {
    Rect::new(inner_x, row_y, inner_w, ROW_HEIGHT * scale)
}

/// Per-row layout passed to `walk_devices` visitors. `strip_top` is the
/// y where the inline request strip starts (= just below the header);
/// `expanded_top` is where the expanded-detail block starts (below the
/// strip when one is present).
pub(super) struct RowGeom {
    pub header: Rect,
    pub strip_top: f32,
    pub expanded_top: f32,
}

/// Height of a section's label line ("Paired" / "Available") plus the gap
/// under it.
fn section_header_h(scale: f32) -> f32 {
    SECTION_HEADER_FONT * scale + SECTION_HEADER_BOTTOM_GAP * scale
}

/// Walk the device list top-to-bottom, mirroring the renderer's layout,
/// and invoke `visit` for each device with its row geometry. `top` is
/// the y of the first section header: the viewport's top shifted up by
/// the scroll offset. `visit` returns `true` to stop iteration early
/// (used by hit-testing). Returns the y just below the last section,
/// where the footer lines start.
pub(super) fn walk_devices(
    bt: &Bluetooth,
    panel: Rect,
    top: f32,
    text_size: f32,
    scale: f32,
    mut visit: impl FnMut(&Device, RowGeom) -> bool,
) -> f32 {
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let inner_x = panel.x + pad;
    let inner_w = panel.w - pad * 2.0;
    let row_h = ROW_HEIGHT * scale;
    let section_header_h = section_header_h(scale);
    let section_gap = SECTION_GAP * scale;

    let mut cy = top;

    let walk_section =
        |cy: &mut f32, devs: &[&Device], visit: &mut dyn FnMut(&Device, RowGeom) -> bool| -> bool {
            for dev in devs.iter().take(MAX_SECTION_ROWS) {
                let header = header_row_rect(inner_x, inner_w, *cy, scale);
                let strip_top = *cy + row_h;
                let prompt_h = prompt_extra_height(bt, dev, text_size, scale);
                let expanded_top = strip_top + prompt_h;
                if visit(
                    dev,
                    RowGeom {
                        header,
                        strip_top,
                        expanded_top,
                    },
                ) {
                    return true;
                }
                *cy += row_h + row_extra_height(bt, dev, text_size, scale);
            }
            false
        };

    let paired = bt.paired_devices();
    if !paired.is_empty() {
        cy += section_header_h;
        if walk_section(&mut cy, &paired, &mut visit) {
            return cy;
        }
        cy += section_gap;
    }
    let unpaired = bt.unpaired_devices();
    if !unpaired.is_empty() {
        cy += section_header_h;
        if walk_section(&mut cy, &unpaired, &mut visit) {
            return cy;
        }
    }
    cy
}

// ── Scrolling ───────────────────────────────────────────────────────────────

/// Text lines the view stacks under the device sections: the
/// hidden-devices note (or an empty-state message), the last error and
/// the last received file. Each one advances the cursor a body-font line.
fn footer_lines(bt: &Bluetooth) -> usize {
    let note = if bt.unpaired_devices().is_empty() {
        bt.paired_devices().is_empty()
    } else {
        bt.hidden_unpaired_count() > 0
    };
    note as usize + bt.last_error().is_some() as usize + bt.last_received.is_some() as usize
}

/// Total height of the scrolling content in physical px: both device
/// sections (request strips and the expanded block included) plus the
/// footer lines. Zero while the controller is off — nothing lists then.
fn content_height(bt: &Bluetooth, panel: Rect, text_size: f32, scale: f32) -> f32 {
    if !bt.is_powered() {
        return 0.0;
    }
    let row_font = body_font(text_size, scale);
    let mut h = walk_devices(bt, panel, 0.0, text_size, scale, |_, _| false);
    let footer = footer_lines(bt);
    if footer > 0 {
        // Footer lines are drawn half a line below the cursor, so the
        // last one needs that much extra room or its tail clips.
        h += (footer as f32 + 0.5) * row_font;
    }
    h
}

/// Top and bottom of the first row carrying a live request strip, in
/// physical px from the top of the content. The bottom is the strip's,
/// not the expanded block's: the Accept/Reject buttons are what matter.
fn request_row_span(bt: &Bluetooth, panel: Rect, text_size: f32, scale: f32) -> Option<(f32, f32)> {
    let mut span = None;
    walk_devices(bt, panel, 0.0, text_size, scale, |dev, geom| {
        if row_prompt(bt, dev).is_none() {
            return false;
        }
        // Request rows float to the front of their section, so take the
        // section label along rather than cutting it off.
        let top = (geom.header.y - section_header_h(scale)).max(0.0);
        span = Some((top, geom.expanded_top));
        true
    });
    span
}

/// The scrolling device list's window on screen. Everything under the
/// toggles row lives in it; content past its edges is clipped.
pub(super) struct ListViewport {
    /// Top edge, physical px.
    pub top: f32,
    /// Height, physical px.
    pub height: f32,
    /// Furthest the content can scroll, physical px. Zero when it fits.
    pub max_scroll: f32,
    /// Scroll offset in effect, physical px: `Bluetooth::scroll` clamped
    /// to the content as it is now, then nudged to keep a live request
    /// row on screen while the view is following one.
    pub scroll: f32,
}

impl ListViewport {
    /// Whether any of the span `y .. y + h` lands inside the viewport.
    pub fn shows(&self, y: f32, h: f32) -> bool {
        y + h >= self.top && y <= self.top + self.height
    }
}

/// Resolve the list's viewport and scroll offset. Draw and hit-test both
/// go through here, so a click always lands on the row that's painted.
pub(super) fn list_viewport(
    bt: &Bluetooth,
    panel: Rect,
    panel_top_y: f32,
    text_size: f32,
    scale: f32,
) -> ListViewport {
    let top = list_top_y(panel_top_y, scale);
    let bottom = panel.y + panel.h - LIST_BOTTOM_PAD * scale;
    let height = (bottom - top).max(0.0);
    let max_scroll = (content_height(bt, panel, text_size, scale) - height).max(0.0);

    let mut scroll = bt.scroll * scale;
    if bt.follow_request {
        if let Some((row_top, row_bottom)) = request_row_span(bt, panel, text_size, scale) {
            // Bottom edge into view first, then the top: a row taller
            // than the viewport shows its top, where the buttons are.
            scroll = scroll.max(row_bottom - height).min(row_top);
        }
    }

    ListViewport {
        top,
        height,
        max_scroll,
        scroll: scroll.clamp(0.0, max_scroll),
    }
}

/// Scroll the device list by a wheel delta (logical px, positive = down).
/// Starts from the offset actually on screen, so a list that shrank
/// under a stale offset doesn't swallow the first notches, and takes
/// over from request-following: once the user scrolls, they're driving.
pub fn scroll_by(
    bt: &mut Bluetooth,
    dy: f32,
    panel: Rect,
    panel_top_y: f32,
    scale: f32,
    text_size: f32,
) {
    let vp = list_viewport(bt, panel, panel_top_y, text_size, scale);
    bt.follow_request = false;
    bt.scroll = (vp.scroll + dy * scale).clamp(0.0, vp.max_scroll) / scale;
}
