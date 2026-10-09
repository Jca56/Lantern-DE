//! The layout card's picture: every monitor as a tile where it sits on
//! the desk, to scale, with its number on it. A click picks one; with
//! more than one on, a drag moves one, landing beside the others as it
//! goes (see `arrange`). Monitors that are off wait at the right, out of
//! the way of the ones that are on.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, Sense, Ui};

use super::arrange::{self, Tile};
use super::draft::{self, Draft};
use crate::kit::{self, bits};
use crate::look;
use crate::outputs::Head;

/// How tall the picture is, and the room kept clear around the tiles.
pub const HEIGHT: f64 = 280.0;
const PAD: f64 = 26.0;
/// How near to lined up, on screen, a dragged tile has to be to line up.
const MAGNET: f64 = 14.0;
/// The gap, on the desk, between the monitors that are on and the ones
/// that are off, as a share of the widest of them.
const PARK_GAP: f64 = 0.12;

/// How the desk maps into the picture: a logical pixel is `scale` of
/// ours, and the desk's `origin` is at `at`.
#[derive(Clone, Copy)]
struct Fit {
    scale: f64,
    origin: Vec2,
    at: Vec2,
}

impl Fit {
    fn rect(&self, t: Tile) -> Rect {
        let min = self.at + (Vec2::new(f64::from(t.x), f64::from(t.y)) - self.origin) * self.scale;
        Rect::from_min_size(min.round(), (Vec2::new(f64::from(t.w), f64::from(t.h)) * self.scale).round())
    }

    /// The desk position the picture's `p` stands for.
    fn desk(&self, p: Vec2) -> Vec2 {
        (p - self.at) / self.scale + self.origin
    }
}

/// A tile being dragged.
struct Drag {
    index: usize,
    /// From the tile's corner to where it was taken hold of, on screen.
    hold: Vec2,
    /// The picture as it was when the drag began: it must not rescale
    /// under the pointer as the tile moves.
    fit: Fit,
    moved: bool,
}

#[derive(Default)]
pub struct Canvas {
    drag: Option<Drag>,
}

impl Canvas {
    /// Forget a drag: the monitors changed under it.
    pub fn let_go(&mut self) {
        self.drag = None;
    }
}

/// Where the monitors that are off are shown: in a row to the right of
/// the ones that are on, in their own sizes.
fn parked(on: Tile, sizes: &[(i32, i32)]) -> Vec<Tile> {
    let widest = sizes.iter().map(|s| s.0).chain([on.w]).max().unwrap_or(1);
    let gap = ((f64::from(widest) * PARK_GAP) as i32).max(1);
    let mut x = on.x + on.w + gap;
    sizes
        .iter()
        .map(|&(w, h)| {
            let t = Tile { x, y: on.y, w, h };
            x += w + gap;
            t
        })
        .collect()
}

/// Fit `all` into `room`, centred.
fn fit(room: Rect, all: Tile) -> Fit {
    let scale = (room.width() / f64::from(all.w)).min(room.height() / f64::from(all.h)).max(1e-6);
    let size = Vec2::new(f64::from(all.w), f64::from(all.h)) * scale;
    Fit { scale, origin: Vec2::new(f64::from(all.x), f64::from(all.y)), at: room.center() - size * 0.5 }
}

/// Draw the picture in `rect` and take clicks and drags on it. `true`
/// when a drag moved a monitor: the draft changed. With `live` off,
/// nothing can be dragged (a setup is on trial).
pub fn draw(ui: &mut Ui, rect: Rect, heads: &[Head], drafts: &mut [Draft], selected: &mut usize, st: &mut Canvas, live: bool) -> bool {
    let m = ui.m;
    // The row of buttons under it brings its own room.
    let well = Rect::new(rect.min + Vec2::splat(m.px(14.0)), Vec2::new(rect.max.x - m.px(14.0), rect.max.y));
    ui.draw.rounded_rect(well, m.px(12.0), look::WELL);
    let n = drafts.len().min(heads.len());
    if n == 0 {
        return false;
    }
    let on: Vec<usize> = (0..n).filter(|&i| drafts[i].enabled).collect();
    let off: Vec<usize> = (0..n).filter(|&i| !drafts[i].enabled).collect();
    let tile_of = |i: usize, drafts: &[Draft]| draft::tile(&heads[i], &drafts[i]);
    let can_drag = live && on.len() > 1;
    if st.drag.as_ref().is_some_and(|d| !can_drag || d.index >= n || !drafts[d.index].enabled) {
        st.drag = None;
    }

    // The tile in hand goes where the pointer has it, on the nearest
    // place beside the others.
    let mut changed = false;
    if let Some(d) = &mut st.drag {
        let others: Vec<Tile> = on.iter().filter(|&&i| i != d.index).map(|&i| tile_of(i, drafts)).collect();
        let at = d.fit.desk(ui.state.pointer - d.hold);
        let mine = tile_of(d.index, drafts);
        let magnet = (m.px(MAGNET) / d.fit.scale).round() as i32;
        let landed = arrange::snap(Tile { x: at.x.round() as i32, y: at.y.round() as i32, ..mine }, &others, magnet);
        if (landed.x, landed.y) != (mine.x, mine.y) {
            (drafts[d.index].x, drafts[d.index].y) = (landed.x, landed.y);
            d.moved = true;
        }
    }

    let on_tiles: Vec<Tile> = on.iter().map(|&i| tile_of(i, drafts)).collect();
    let on_box = arrange::bounds(&on_tiles);
    let off_sizes: Vec<(i32, i32)> = off.iter().map(|&i| tile_of(i, drafts)).map(|t| (t.w, t.h)).collect();
    let off_tiles = parked(on_box, &off_sizes);
    let all: Vec<Tile> = on_tiles.iter().chain(&off_tiles).copied().collect();
    let room = Rect::new(well.min + Vec2::splat(m.px(PAD)), well.max - Vec2::splat(m.px(PAD)));
    let view = st.drag.as_ref().map_or_else(|| fit(room, arrange::bounds(&all)), |d| d.fit);

    // The one in hand (else the one picked) is drawn last, over the rest.
    let mut order: Vec<(usize, Tile)> = on.iter().copied().zip(on_tiles).chain(off.iter().copied().zip(off_tiles)).collect();
    let top = st.drag.as_ref().map_or(*selected, |d| d.index);
    order.sort_by_key(|(i, _)| *i == top);
    ui.push_id("tiles");
    let mut ended = false;
    for &(i, tile) in &order {
        // A hair in from its edges, so two that touch show a seam.
        let face = view.rect(tile).shrink(m.px(2.0));
        let id = ui.id("tile").with_index(i);
        let draggable = can_drag && drafts[i].enabled;
        let mut r = ui.interact(id, face, if draggable { Sense::DRAG } else { Sense::CLICK });
        ui.focusable(id, face);
        ui.key_click(id, &mut r);
        if r.pressed || r.clicked {
            *selected = i;
        }
        if r.pressed && draggable {
            st.drag = Some(Drag { index: i, hold: ui.state.pointer - view.rect(tile).min, fit: view, moved: false });
        }
        let held = st.drag.as_ref().is_some_and(|d| d.index == i);
        if held && r.released {
            ended = true;
        }
        if held {
            ui.state.cursor_icon = CursorIcon::Grabbing;
        } else if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        paint(ui, face, i, &heads[i], &drafts[i], *selected == i, r.hovered || held);
        ui.focus_ring(id, face);
    }
    ui.pop_id();
    if ended && let Some(d) = st.drag.take() {
        changed = d.moved;
    }
    changed
}

/// One monitor's tile: its number, and under it its name and what it is
/// set to, as far as there is room for them.
fn paint(ui: &mut Ui, face: Rect, index: usize, head: &Head, d: &Draft, picked: bool, hot: bool) {
    let m = ui.m;
    let accent = ui.theme.accent;
    let radius = m.px(10.0);
    let (fill, edge, ink) = match (d.enabled, picked) {
        (true, true) => (accent.lerp(look::CARD, 0.8), accent, look::TEXT),
        (true, false) => (look::BUTTON, if hot { look::TEXT_DIM } else { look::TRACK }, look::TEXT),
        (false, true) => (look::CARD, accent.fade(0.7), look::TEXT_DIM),
        (false, false) => (look::CARD, if hot { look::TEXT_DIM } else { look::LINE }, look::TEXT_DIM),
    };
    ui.draw.rounded_rect(face, radius, fill);
    ui.draw.stroke_rect(face, m.px(if picked { 3.0 } else { 2.0 }), radius, edge);

    // The number is as big as the tile lets it be, never small.
    let mut big = kit::title_style(ui);
    big.size = (face.height() * 0.4).clamp(m.px(26.0), m.px(110.0)) as f32;
    let (body, small) = (ui.text_style(), kit::small_style(ui));
    let number = (index + 1).to_string();
    let detail = match draft::mode_of(head, d) {
        Some(mode) if d.enabled => format!("{} × {} · {} · {}", mode.width, mode.height, draft::rate_label(mode.refresh), kit::percent(d.scale)),
        _ => "Off".to_owned(),
    };
    let inner = face.width() - m.px(16.0);
    let mut lines: Vec<(&str, f64, Color, u8)> = vec![(&number, f64::from(big.line_height()), if picked && d.enabled { accent } else { ink }, 0)];
    if ui.measure(&d.name, &body) <= inner {
        lines.push((&d.name, f64::from(body.line_height()), ink, 1));
    }
    if ui.measure(&detail, &small) <= inner {
        lines.push((&detail, f64::from(small.line_height()), look::TEXT_DIM, 2));
    }
    // Lines that don't fit under the number go, the last first.
    while lines.len() > 1 && lines.iter().map(|l| l.1).sum::<f64>() > face.height() - m.px(12.0) {
        lines.pop();
    }
    let mut y = (face.center().y - lines.iter().map(|l| l.1).sum::<f64>() * 0.5).round();
    for (text, h, color, which) in lines {
        let line = Rect::from_min_size(Vec2::new(face.min.x, y), Vec2::new(face.width(), h));
        let style = match which {
            0 => &big,
            1 => &body,
            _ => &small,
        };
        ui.text_centered(text, style, line, color);
        y += h;
    }
    if d.primary && face.width() >= bits::badge_width(ui, "Main") + m.px(20.0) && face.height() >= m.px(bits::BADGE_H) * 3.0 {
        bits::badge(ui, face.min.x + m.px(10.0), face.min.y + m.px(10.0) + m.px(bits::BADGE_H) * 0.5, "Main", accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desk_fits_the_room_and_maps_back() {
        let room = Rect::from_xywh(100.0, 50.0, 800.0, 300.0);
        let all = Tile { x: 0, y: 0, w: 4663, h: 1543 };
        let f = fit(room, all);
        // Wide: the width is what limits it, and it sits in the middle.
        let whole = f.rect(all);
        assert!((whole.width() - 800.0).abs() <= 1.0 && whole.height() < 300.0);
        assert!((whole.center().y - room.center().y).abs() <= 1.0);
        let right = Tile { x: 2743, y: 0, w: 1920, h: 1080 };
        let back = f.desk(f.rect(right).min);
        assert!((back.x - 2743.0).abs() < 4.0 && back.y.abs() < 4.0, "{back:?}");
        // Tall: the height limits it.
        let f = fit(room, Tile { x: -10, y: 5, w: 1000, h: 3000 });
        assert!((f.rect(Tile { x: -10, y: 5, w: 1000, h: 3000 }).height() - 300.0).abs() <= 1.0);
    }

    #[test]
    fn monitors_that_are_off_wait_at_the_right() {
        let on = Tile { x: 0, y: 0, w: 2743, h: 1543 };
        let off = parked(on, &[(1920, 1080), (1280, 720)]);
        assert_eq!(off[0], Tile { x: 2743 + 329, y: 0, w: 1920, h: 1080 });
        assert_eq!(off[1].x, off[0].x + 1920 + 329);
        assert!(!off[0].overlaps(on) && !off[1].overlaps(off[0]));
        assert!(parked(on, &[]).is_empty());
    }
}
