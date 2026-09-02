//! Sidebar geometry: header, the ".."/folder rows, and the image tile grid.
//! Shared by input (hit-testing) and render so both agree on where every
//! slot sits. A *slot* is one hit-testable thing in the scroll area:
//! slot 0 is ".." (when not at /), then one per directory, then one per
//! image — matching `SidebarState::entries` order after the parent skip.

use lntrn_render::Rect;

use super::sidebar::{SidebarState, HEADER_H};

/// Logical px (multiply by scale `s`).
pub const PAD: f32 = 10.0;
pub const GAP: f32 = 8.0;
pub const DIR_ROW_H: f32 = 46.0;
/// Height of the filename strip under a tile when names are shown.
pub const NAME_H: f32 = 30.0;
/// Diameter of the "+" add badge in a tile's corner.
pub const ADD_BADGE: f32 = 36.0;
/// Width of the drag band straddling the sidebar's right edge.
pub const RESIZE_GRIP: f32 = 12.0;

pub struct SidebarLayout {
    /// Whole panel (below title bar, above status bar).
    pub side: Rect,
    pub header: Rect,
    /// Scrollable area.
    pub rows_vp: Rect,
    /// Resize drag band (screen space), centred on the right edge.
    pub grip: Rect,
    pub skip_parent: usize,
    pub n_dirs: usize,
    pub n_files: usize,
    pub cols: usize,
    /// Tile side in physical px (the thumbnail box is square).
    pub tile: f32,
    /// Full tile height: `tile` plus the name strip when names are shown.
    pub tile_h: f32,
    pub content_h: f32,
    /// Content-space y where the image grid starts.
    grid_top: f32,
    s: f32,
}

impl SidebarLayout {
    /// Standard placement: full height between the title bar and status bar.
    pub fn compute(sb: &SidebarState, wf: f32, hf: f32, s: f32) -> Self {
        let title_h = crate::TITLE_H * s;
        let status_h = crate::STATUS_H * s;
        let band = Rect::new(0.0, title_h, wf, (hf - title_h - status_h).max(1.0));
        Self::compute_in(sb, band, s)
    }

    /// Place the sidebar at the left edge of `band`, the vertical strip it
    /// may occupy (the viewer hands over a taller one in rice mode).
    pub fn compute_in(sb: &SidebarState, band: Rect, s: f32) -> Self {
        let side = Rect::new(
            band.x,
            band.y,
            sb.phys_width(s).min(band.w),
            band.h.max(1.0),
        );
        let header = Rect::new(side.x, side.y, side.w, HEADER_H * s);
        let rows_vp = Rect::new(
            side.x,
            side.y + header.h,
            side.w,
            (side.h - header.h).max(1.0),
        );
        let grip_w = RESIZE_GRIP * s;
        let grip = Rect::new(side.x + side.w - grip_w * 0.5, side.y, grip_w, side.h);

        let skip_parent = usize::from(sb.current_dir.parent().is_some());
        let n_dirs = sb.entries.iter().take_while(|e| e.is_dir).count();
        let n_files = sb.entries.len() - n_dirs;

        let pad = PAD * s;
        let gap = GAP * s;
        let avail = (side.w - pad * 2.0).max(1.0);
        let target = sb.tile_target * s;
        let cols = ((avail + gap) / (target + gap)).round().max(1.0) as usize;
        let tile = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).max(1.0);
        let tile_h = tile + if sb.show_names { NAME_H * s } else { 0.0 };

        let n_rows = skip_parent + n_dirs;
        let rows_h = n_rows as f32 * DIR_ROW_H * s;
        let grid_top = pad + rows_h + if n_rows > 0 { gap } else { 0.0 };
        let grid_rows = n_files.div_ceil(cols.max(1));
        let grid_h = if grid_rows > 0 {
            grid_rows as f32 * (tile_h + gap) - gap
        } else {
            0.0
        };
        let content_h = grid_top + grid_h + pad;

        Self {
            side,
            header,
            rows_vp,
            grip,
            skip_parent,
            n_dirs,
            n_files,
            cols: cols.max(1),
            tile,
            tile_h,
            content_h,
            grid_top,
            s,
        }
    }

    pub fn scale(&self) -> f32 {
        self.s
    }

    pub fn slot_count(&self) -> usize {
        self.skip_parent + self.n_dirs + self.n_files
    }

    pub fn is_parent(&self, slot: usize) -> bool {
        self.skip_parent == 1 && slot == 0
    }

    /// Index into `SidebarState::entries` for a slot (None for "..").
    pub fn entry_index(&self, slot: usize) -> Option<usize> {
        if self.is_parent(slot) {
            None
        } else {
            Some(slot - self.skip_parent)
        }
    }

    /// Slot rect in content space (y = 0 at the top of the scroll content).
    pub fn slot_rect_content(&self, slot: usize) -> Rect {
        let s = self.s;
        let pad = PAD * s;
        let gap = GAP * s;
        let list_rows = self.skip_parent + self.n_dirs;
        if slot < list_rows {
            let row_h = DIR_ROW_H * s;
            return Rect::new(
                pad,
                pad + slot as f32 * row_h,
                self.side.w - pad * 2.0,
                row_h,
            );
        }
        let f = slot - list_rows;
        let (row, col) = (f / self.cols, f % self.cols);
        Rect::new(
            pad + col as f32 * (self.tile + gap),
            self.grid_top + row as f32 * (self.tile_h + gap),
            self.tile,
            self.tile_h,
        )
    }

    /// Slot rect in screen space for the current scroll offset.
    pub fn slot_rect(&self, slot: usize, scroll: f32) -> Rect {
        self.slot_rect_content(slot)
            .translate(self.rows_vp.x, self.rows_vp.y - scroll)
    }

    /// Slots that intersect the scroll viewport.
    pub fn visible_slots(&self, scroll: f32) -> Vec<usize> {
        let top = scroll;
        let bottom = scroll + self.rows_vp.h;
        (0..self.slot_count())
            .filter(|&slot| {
                let r = self.slot_rect_content(slot);
                r.y + r.h >= top && r.y <= bottom
            })
            .collect()
    }

    /// The square thumbnail area of an image tile (excludes the name strip).
    pub fn thumb_box(&self, tile: &Rect) -> Rect {
        Rect::new(tile.x, tile.y, tile.w, self.tile)
    }

    /// The "+" add-to-canvas badge in a tile's top-right corner.
    pub fn add_badge_rect(&self, tile: &Rect) -> Rect {
        let d = (ADD_BADGE * self.s).min(tile.w * 0.4);
        let m = 6.0 * self.s;
        Rect::new(tile.x + tile.w - d - m, tile.y + m, d, d)
    }
}
