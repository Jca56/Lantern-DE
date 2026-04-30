//! Tile layout helper for the controls row.
//!
//! Tiles declare a preferred logical width and a side (`Left` or
//! `Right`). The layout packs left tiles from the panel's left edge,
//! right tiles from the right edge, with a small inter-tile gap.

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

/// Which side of the controls row a tile docks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// One tile's request to the layout: which side it wants and how wide
/// (logical px) it should be drawn.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub side: Side,
    pub logical_width: f32,
}

/// Pack tiles into the row, returning a `TileLayout` per input slot in
/// the same order. Left-side tiles are packed in declaration order from
/// the panel's left edge; right-side tiles are packed in declaration
/// order from the right edge. Within each side a `TILE_GAP` separates
/// adjacent tiles.
pub fn pack(panel: Rect, scale: f32, slots: &[Slot]) -> Vec<TileLayout> {
    use crate::controls::{ROW_HEIGHT, ROW_HORIZONTAL_PAD, ROW_TOP_MARGIN};

    let pad = ROW_HORIZONTAL_PAD * scale;
    let row_top = panel.y + ROW_TOP_MARGIN * scale;
    let row_h = ROW_HEIGHT * scale;
    let gap = TILE_GAP * scale;

    let left_edge = panel.x + pad;
    let right_edge = panel.x + panel.w - pad;

    let mut layouts: Vec<Option<TileLayout>> = vec![None; slots.len()];
    let mut left_x = left_edge;
    let mut right_x = right_edge;
    let mut first_left = true;
    let mut first_right = true;

    // First pass: left tiles in order.
    for (i, slot) in slots.iter().enumerate() {
        if slot.side != Side::Left {
            continue;
        }
        let w = slot.logical_width * scale;
        if !first_left {
            left_x += gap;
        }
        first_left = false;
        layouts[i] = Some(TileLayout { x: left_x, y: row_top, w, h: row_h });
        left_x += w;
    }

    // Second pass: right tiles in order. We compute a tile's x by
    // subtracting its width from the moving right cursor.
    for (i, slot) in slots.iter().enumerate() {
        if slot.side != Side::Right {
            continue;
        }
        let w = slot.logical_width * scale;
        if !first_right {
            right_x -= gap;
        }
        first_right = false;
        right_x -= w;
        layouts[i] = Some(TileLayout { x: right_x, y: row_top, w, h: row_h });
    }

    // Slots are guaranteed populated; unwrap is safe (every slot has a
    // side that the two passes above cover).
    layouts.into_iter().map(|o| o.expect("slot unfilled")).collect()
}

/// Logical px between adjacent tiles within the same side.
pub const TILE_GAP: f32 = 4.0;
