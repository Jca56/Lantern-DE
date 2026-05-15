use lntrn_render::{Color, Painter, Rect};

use crate::app::ViewMode;

// ── Sidebar place icons ─────────────────────────────────────────────────────

pub(super) fn draw_place_icon(painter: &mut Painter, name: &str, cx: f32, cy: f32, color: Color, s: f32) {
    let sw = 1.5 * s; // stroke width
    let u = s; // unit scale
    match name {
        "Home" => {
            painter.line(cx - 7.0*u, cy + 1.0*u, cx, cy - 7.0*u, sw, color);
            painter.line(cx, cy - 7.0*u, cx + 7.0*u, cy + 1.0*u, sw, color);
            painter.rect_stroke(Rect::new(cx - 5.0*u, cy + 1.0*u, 10.0*u, 7.0*u), 0.0, sw, color);
        }
        "Desktop" => {
            painter.rect_stroke(Rect::new(cx - 7.0*u, cy - 6.0*u, 14.0*u, 10.0*u), 1.0*u, sw, color);
            painter.line(cx, cy + 4.0*u, cx, cy + 7.0*u, sw, color);
            painter.line(cx - 4.0*u, cy + 7.0*u, cx + 4.0*u, cy + 7.0*u, sw, color);
        }
        "Documents" => {
            painter.rect_stroke(Rect::new(cx - 5.0*u, cy - 7.0*u, 10.0*u, 14.0*u), 0.0, sw, color);
            painter.line(cx + 5.0*u, cy - 7.0*u, cx + 5.0*u, cy - 3.0*u, sw * 0.75, color);
            painter.line(cx + 1.0*u, cy - 7.0*u, cx + 5.0*u, cy - 3.0*u, sw * 0.75, color);
            painter.line(cx - 3.0*u, cy - 1.0*u, cx + 3.0*u, cy - 1.0*u, 1.0*u, color);
            painter.line(cx - 3.0*u, cy + 2.0*u, cx + 3.0*u, cy + 2.0*u, 1.0*u, color);
            painter.line(cx - 3.0*u, cy + 5.0*u, cx + 1.0*u, cy + 5.0*u, 1.0*u, color);
        }
        "Downloads" => {
            painter.line(cx, cy - 6.0*u, cx, cy + 2.0*u, sw, color);
            painter.line(cx - 4.0*u, cy - 2.0*u, cx, cy + 2.0*u, sw, color);
            painter.line(cx + 4.0*u, cy - 2.0*u, cx, cy + 2.0*u, sw, color);
            painter.line(cx - 6.0*u, cy + 3.0*u, cx - 6.0*u, cy + 7.0*u, sw, color);
            painter.line(cx - 6.0*u, cy + 7.0*u, cx + 6.0*u, cy + 7.0*u, sw, color);
            painter.line(cx + 6.0*u, cy + 3.0*u, cx + 6.0*u, cy + 7.0*u, sw, color);
        }
        "Music" => {
            painter.line(cx - 2.0*u, cy - 6.0*u, cx - 2.0*u, cy + 4.0*u, sw, color);
            painter.circle_filled(cx - 4.0*u, cy + 5.0*u, 3.0*u, color);
            painter.line(cx - 2.0*u, cy - 6.0*u, cx + 4.0*u, cy - 4.0*u, sw, color);
            painter.line(cx + 4.0*u, cy - 4.0*u, cx + 4.0*u, cy + 1.0*u, sw, color);
            painter.circle_filled(cx + 2.0*u, cy + 2.0*u, 2.5*u, color);
        }
        "Pictures" => {
            painter.rect_stroke(Rect::new(cx - 7.0*u, cy - 5.0*u, 14.0*u, 12.0*u), 0.0, sw, color);
            painter.line(cx - 4.0*u, cy + 5.0*u, cx - 1.0*u, cy, sw, color);
            painter.line(cx - 1.0*u, cy, cx + 2.0*u, cy + 3.0*u, sw, color);
            painter.line(cx + 2.0*u, cy + 3.0*u, cx + 5.0*u, cy - 1.0*u, sw, color);
            painter.circle_filled(cx + 3.0*u, cy - 2.0*u, 2.0*u, color);
        }
        "Videos" => {
            painter.rect_stroke(Rect::new(cx - 7.0*u, cy - 5.0*u, 14.0*u, 12.0*u), 1.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 3.0*u, cx - 2.0*u, cy + 5.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 3.0*u, cx + 4.0*u, cy + 1.0*u, sw, color);
            painter.line(cx + 4.0*u, cy + 1.0*u, cx - 2.0*u, cy + 5.0*u, sw, color);
        }
        "Cloud" => {
            // Cloud silhouette: two bumps + flat bottom.
            painter.circle_filled(cx - 4.0*u, cy - 1.0*u, 4.0*u, color);
            painter.circle_filled(cx + 1.0*u, cy - 3.0*u, 5.0*u, color);
            painter.circle_filled(cx + 5.0*u, cy, 3.5*u, color);
            painter.rect_filled(Rect::new(cx - 7.0*u, cy - 1.0*u, 14.0*u, 5.0*u), 2.0*u, color);
        }
        "Trash" => {
            // Lid
            painter.line(cx - 6.0*u, cy - 5.0*u, cx + 6.0*u, cy - 5.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 7.0*u, cx + 2.0*u, cy - 7.0*u, sw, color);
            painter.line(cx - 2.0*u, cy - 7.0*u, cx - 2.0*u, cy - 5.0*u, sw * 0.75, color);
            painter.line(cx + 2.0*u, cy - 7.0*u, cx + 2.0*u, cy - 5.0*u, sw * 0.75, color);
            // Body (tapered)
            painter.line(cx - 5.0*u, cy - 4.0*u, cx - 4.0*u, cy + 7.0*u, sw, color);
            painter.line(cx + 5.0*u, cy - 4.0*u, cx + 4.0*u, cy + 7.0*u, sw, color);
            painter.line(cx - 4.0*u, cy + 7.0*u, cx + 4.0*u, cy + 7.0*u, sw, color);
            // Ribs
            painter.line(cx - 2.0*u, cy - 2.0*u, cx - 2.0*u, cy + 5.0*u, sw * 0.75, color);
            painter.line(cx, cy - 2.0*u, cx, cy + 5.0*u, sw * 0.75, color);
            painter.line(cx + 2.0*u, cy - 2.0*u, cx + 2.0*u, cy + 5.0*u, sw * 0.75, color);
        }
        _ => {
            painter.rect_filled(Rect::new(cx - 8.0*u, cy - 2.0*u, 16.0*u, 10.0*u), 2.0*u, color);
            painter.rect_filled(Rect::new(cx - 8.0*u, cy - 4.0*u, 8.0*u, 4.0*u), 1.0*u, color);
        }
    }
}

// ── View mode icon ──────────────────────────────────────────────────────────

pub(super) fn draw_view_mode_icon(painter: &mut Painter, mode: ViewMode, r: Rect, color: Color, s: f32) {
    match mode {
        ViewMode::Grid => {
            // 2x2 squares
            let vx = r.x + 8.0 * s;
            let vy = r.y + 8.0 * s;
            let sq = 8.0 * s;
            let gap = 3.0 * s;
            painter.rect_filled(Rect::new(vx, vy, sq, sq), 1.0 * s, color);
            painter.rect_filled(Rect::new(vx + sq + gap, vy, sq, sq), 1.0 * s, color);
            painter.rect_filled(Rect::new(vx, vy + sq + gap, sq, sq), 1.0 * s, color);
            painter.rect_filled(Rect::new(vx + sq + gap, vy + sq + gap, sq, sq), 1.0 * s, color);
        }
        ViewMode::List => {
            // Three horizontal lines with bullet dots
            let lx = r.x + 8.0 * s;
            let ly = r.y + 10.0 * s;
            let lw = 18.0 * s;
            let gap = 6.0 * s;
            for i in 0..3 {
                let y = ly + i as f32 * gap;
                painter.circle_filled(lx + 2.0 * s, y + 1.0 * s, 1.5 * s, color);
                painter.rect_filled(Rect::new(lx + 6.0 * s, y, lw, 2.0 * s), 1.0 * s, color);
            }
        }
        ViewMode::Tree => {
            // Tree structure: vertical line with branches
            let tx = r.x + 10.0 * s;
            let ty = r.y + 8.0 * s;
            let sw = 1.5 * s;
            // Trunk
            painter.line(tx, ty, tx, ty + 18.0 * s, sw, color);
            // Branch 1
            painter.line(tx, ty + 3.0 * s, tx + 8.0 * s, ty + 3.0 * s, sw, color);
            painter.rect_filled(Rect::new(tx + 10.0 * s, ty + 1.0 * s, 8.0 * s, 4.0 * s), 1.0 * s, color);
            // Branch 2
            painter.line(tx, ty + 10.0 * s, tx + 8.0 * s, ty + 10.0 * s, sw, color);
            painter.rect_filled(Rect::new(tx + 10.0 * s, ty + 8.0 * s, 8.0 * s, 4.0 * s), 1.0 * s, color);
            // Branch 3
            painter.line(tx, ty + 17.0 * s, tx + 8.0 * s, ty + 17.0 * s, sw, color);
            painter.rect_filled(Rect::new(tx + 10.0 * s, ty + 15.0 * s, 8.0 * s, 4.0 * s), 1.0 * s, color);
        }
    }
}

/// Preview pane toggle icon — rectangle with a vertical bar marking the
/// right-side info pane.
pub(super) fn draw_preview_pane_icon(painter: &mut Painter, r: Rect, color: Color, s: f32) {
    let sw = 1.5 * s;
    // Outer rect with rounded corners
    let outer = Rect::new(r.x + 6.0 * s, r.y + 9.0 * s, 24.0 * s, 18.0 * s);
    painter.rect_stroke(outer, 2.0 * s, sw, color);
    // Vertical divider ~2/3 of the way across — marks the preview side
    let dx = outer.x + outer.w * 0.62;
    painter.line(dx, outer.y + 2.0 * s, dx, outer.y + outer.h - 2.0 * s, sw, color);
    // A couple of mini "content" rows on the preview side
    let lx = dx + 2.0 * s;
    let lw = outer.x + outer.w - lx - 2.5 * s;
    for i in 0..3 {
        let y = outer.y + 4.0 * s + i as f32 * 4.0 * s;
        painter.rect_filled(Rect::new(lx, y, lw, 1.5 * s), 0.75 * s, color);
    }
}

/// Sort icon — three horizontal lines of decreasing length + a small arrow
/// at the right indicating direction.
pub(super) fn draw_sort_icon(painter: &mut Painter, r: Rect, color: Color, dir: crate::fs::SortDir, s: f32) {
    let lx = r.x + 7.0 * s;
    let ly = r.y + 10.0 * s;
    let gap = 5.0 * s;
    let sw = 2.0 * s;
    // Lines descending in length: visual cue for "sort"
    let widths = [16.0, 12.0, 8.0];
    for (i, w) in widths.iter().enumerate() {
        let y = ly + i as f32 * gap;
        painter.rect_filled(Rect::new(lx, y, w * s, sw), 1.0 * s, color);
    }
    // Direction arrow on the right
    let ax = r.x + r.w - 9.0 * s;
    let ay_top = r.y + 9.0 * s;
    let ay_bot = r.y + 24.0 * s;
    let head = 3.0 * s;
    match dir {
        crate::fs::SortDir::Asc => {
            // Upward arrow
            painter.line(ax, ay_top, ax, ay_bot, sw, color);
            painter.line(ax, ay_top, ax - head, ay_top + head, sw, color);
            painter.line(ax, ay_top, ax + head, ay_top + head, sw, color);
        }
        crate::fs::SortDir::Desc => {
            // Downward arrow
            painter.line(ax, ay_top, ax, ay_bot, sw, color);
            painter.line(ax, ay_bot, ax - head, ay_bot - head, sw, color);
            painter.line(ax, ay_bot, ax + head, ay_bot - head, sw, color);
        }
    }
}

pub(super) fn draw_phone_icon(painter: &mut Painter, cx: f32, cy: f32, color: Color, s: f32) {
    let sw = 1.5 * s;
    let u = s;
    // Phone body
    painter.rect_stroke(
        Rect::new(cx - 5.0 * u, cy - 9.0 * u, 10.0 * u, 18.0 * u),
        2.0 * u,
        sw,
        color,
    );
    // Screen area
    painter.rect_stroke(
        Rect::new(cx - 4.0 * u, cy - 7.0 * u, 8.0 * u, 13.0 * u),
        0.5 * u,
        sw * 0.75,
        color,
    );
    // Speaker slit at top
    painter.line(cx - 1.5 * u, cy - 8.0 * u, cx + 1.5 * u, cy - 8.0 * u, sw * 0.75, color);
    // Home dot at bottom
    painter.circle_filled(cx, cy + 7.5 * u, 0.9 * u, color);
}

pub(super) fn draw_drive_icon(painter: &mut Painter, cx: f32, cy: f32, color: Color, s: f32) {
    let sw = 1.5 * s;
    let u = s;
    // Simple disk/drive shape
    painter.rect_stroke(
        Rect::new(cx - 8.0*u, cy - 5.0*u, 16.0*u, 10.0*u),
        2.0*u, sw, color,
    );
    // Drive bay lines
    painter.line(cx - 5.0*u, cy - 1.0*u, cx + 5.0*u, cy - 1.0*u, 1.0*u, color);
    painter.line(cx - 5.0*u, cy + 2.0*u, cx + 5.0*u, cy + 2.0*u, 1.0*u, color);
    // Activity LED dot
    painter.circle_filled(cx + 5.0*u, cy - 3.0*u, 1.5*u, color);
}
