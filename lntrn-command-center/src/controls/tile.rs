//! Tile layout helper for the controls row.
//!
//! Tiles declare a preferred logical width and a [`Zone`] (`Left`,
//! `Middle`, or `Right`). The layout packs left-zone tiles from the
//! panel's left edge, right-zone tiles flush to the right edge, and
//! middle-zone tiles centered as a group in the gap between the two.
//! Order within each zone is the order tiles appear in the input slice.

use lntrn_render::Rect;

/// A single tile's bounding box (physical pixels).
#[derive(Debug, Clone, Copy)]
pub struct TileLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl TileLayout {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// Which zone of the controls row a tile docks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Middle,
    Right,
}

/// One tile's request to the layout: which zone it wants and how wide
/// (logical px) it should be drawn.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub zone: Zone,
    pub logical_width: f32,
}

/// Pack tiles into the row, returning a `TileLayout` per input slot in
/// the same order. Left tiles pack left-to-right from the left edge;
/// right tiles are right-aligned (kept in input order, left-to-right);
/// middle tiles are centered as a group in the remaining gap.
pub fn pack(panel: Rect, scale: f32, slots: &[Slot]) -> Vec<TileLayout> {
    use crate::controls::{ROW_HEIGHT, ROW_HORIZONTAL_PAD, ROW_TOP_MARGIN};

    let pad = ROW_HORIZONTAL_PAD * scale;
    let row_top = panel.y + ROW_TOP_MARGIN * scale;
    let row_h = ROW_HEIGHT * scale;
    let gap = TILE_GAP * scale;

    let left_edge = panel.x + pad;
    let right_edge = panel.x + panel.w - pad;

    let mut layouts: Vec<Option<TileLayout>> = vec![None; slots.len()];
    let make = |x: f32, w: f32| TileLayout {
        x,
        y: row_top,
        w,
        h: row_h,
    };

    // Left zone: pack left-to-right from the left edge.
    let mut x = left_edge;
    let mut first = true;
    for (i, slot) in slots.iter().enumerate() {
        if slot.zone != Zone::Left {
            continue;
        }
        if !first {
            x += gap;
        }
        first = false;
        let w = slot.logical_width * scale;
        layouts[i] = Some(make(x, w));
        x += w;
    }
    let left_end = x;

    // Right zone: right-aligned, kept in input (left-to-right) order.
    let right_idx: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.zone == Zone::Right)
        .map(|(i, _)| i)
        .collect();
    let right_total = zone_total(slots, &right_idx, scale, gap);
    let right_begin = if right_idx.is_empty() {
        right_edge
    } else {
        right_edge - right_total
    };
    let mut rx = right_begin;
    for &i in &right_idx {
        let w = slots[i].logical_width * scale;
        layouts[i] = Some(make(rx, w));
        rx += w + gap;
    }

    // Middle zone: centered as a group in the gap between the two sides.
    let mid_idx: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.zone == Zone::Middle)
        .map(|(i, _)| i)
        .collect();
    if !mid_idx.is_empty() {
        let mid_total = zone_total(slots, &mid_idx, scale, gap);
        // Center the middle group on the *panel* center (true visual
        // center of the bar), not the midpoint of the left/right gap —
        // asymmetric side clusters would otherwise pull it off-center.
        // Clamp so it never overlaps the side clusters.
        let panel_center = panel.x + panel.w / 2.0;
        let min_x = left_end + gap;
        let max_x = (right_begin - gap - mid_total).max(min_x);
        let mut mx = (panel_center - mid_total / 2.0).clamp(min_x, max_x);
        for &i in &mid_idx {
            let w = slots[i].logical_width * scale;
            layouts[i] = Some(make(mx, w));
            mx += w + gap;
        }
    }

    layouts
        .into_iter()
        .map(|o| o.expect("slot unfilled"))
        .collect()
}

/// Total physical width of a set of slots including inter-tile gaps.
fn zone_total(slots: &[Slot], idx: &[usize], scale: f32, gap: f32) -> f32 {
    let widths: f32 = idx.iter().map(|&i| slots[i].logical_width * scale).sum();
    widths + gap * idx.len().saturating_sub(1) as f32
}

/// Logical px between adjacent tiles within the same zone.
pub const TILE_GAP: f32 = 4.0;
