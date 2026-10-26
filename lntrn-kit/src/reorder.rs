//! A card's entries in an order the user picks. Each has a grip down its
//! left: dragging one lifts its entry over the card, the others making
//! room where it would land, and letting go puts it there. The arrows
//! move the entry whose grip has focus, Home and End to either end.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, KeyStep, Sense, WidgetId};

use crate::card::{Card, ROW_PAD};
use crate::look;

/// The strip down an entry's left that is its grip, and how far its rows'
/// text sits in to clear it, logical.
const GRIP_W: f64 = 58.0;
const INDENT: f64 = 38.0;
/// The grip's six dots: their radius, and between their centres.
const DOT_R: f64 = 3.0;
const DOT_GAP: f64 = 10.0;
/// How far the pointer goes with the button held before an entry lifts.
const DRAG_START: f64 = 8.0;

impl Card<'_, '_> {
    /// `order`'s entries, each as the rows `entry` declares for it (one,
    /// or one with rows `nested` under it), set in to clear its grip. An
    /// entry that declares none (something this machine lacks) takes no
    /// room and has no grip, but keeps its place among the others. `key`
    /// names an entry for good, wherever it moves. `true` when `order`
    /// changed.
    pub fn reorder<T>(&mut self, id: &str, order: &mut [T], key: impl Fn(&T) -> &str, mut entry: impl FnMut(&mut Card, &T)) -> bool {
        let m = self.ui.m;
        let base = self.ui.id(id);
        let mut grips: Vec<WidgetId> = order.iter().map(|t| base.with("grip").with(key(t))).collect();
        // Each entry's top as last laid out, and how far down it took the layout.
        let mut laid: Vec<(f64, f64)> = grips
            .iter()
            .map(|g| {
                let f = self.ui.state.floats(g.with("laid"), [0.0; 4]);
                (f[0], f[1])
            })
            .collect();
        let top = self.ui.cursor().y;
        let total: f64 = laid.iter().map(|l| l.1).sum();
        let mut changed = false;

        // The lifted entry, where it floats, and where it would land.
        let mut floating = None;
        if let Some((i, grab)) = self.lifted(base.with("lift"), &grips, &laid) {
            let h = laid[i].1;
            let y = (self.ui.state.pointer.y - grab).clamp(top, (top + total - h).max(top));
            let others: Vec<f64> = laid.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, l)| l.1).collect();
            let to = spot(&others.iter().map(|&h| h > 0.0).collect::<Vec<_>>(), passed(&others, top, y));
            if self.ui.state.released {
                changed = move_to(order, i, to);
                move_to(&mut grips, i, to);
                move_to(&mut laid, i, to);
            } else {
                floating = Some((i, to, y));
            }
        }

        // Laid out in the order it would be in, the lifted one's place
        // kept open where it would land.
        let mut shown: Vec<usize> = (0..order.len()).collect();
        if let Some((i, to, _)) = floating {
            shown.remove(i);
            shown.insert(to, i);
        }
        let mut visible = Vec::new();
        let mut keyed = None;
        for &j in &shown {
            if floating.is_some_and(|(i, _, _)| i == j) {
                self.hole(laid[j].1);
                visible.push(j);
                continue;
            }
            let Some(step) = self.place(grips[j], false, |c| entry(c, &order[j])) else { continue };
            visible.push(j);
            if step != KeyStep::None {
                keyed = Some((j, step));
            }
        }

        // Drawn last, so over the rest.
        if let Some((i, _, y)) = floating {
            let end = self.ui.cursor().y;
            let (x, w) = (self.ui.cursor().x, self.ui.avail_width());
            let plate = Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, (laid[i].1 - m.gap).max(0.0)));
            let r = m.px(12.0);
            self.ui.draw.shadow(plate.translate(Vec2::new(0.0, m.px(6.0))), r, m.px(18.0), Color::BLACK.fade(0.55));
            self.ui.draw.rounded_rect(plate, r, look::card().lerp(look::up(), 0.05));
            self.ui.draw.stroke_rect(plate, m.px(1.5), r, self.ui.theme.accent.fade(0.7));
            self.ui.space(y - end);
            let rows = std::mem::replace(&mut self.rows, 0);
            self.place(grips[i], true, |c| entry(c, &order[i]));
            self.rows = rows;
            self.ui.space(end - self.ui.cursor().y);
        }

        if let Some((j, step)) = keyed.filter(|_| floating.is_none()) {
            let (p, last) = (visible.iter().position(|&v| v == j).unwrap_or(0), visible.len() - 1);
            let q = match step {
                KeyStep::By(n) => (p as i64 - i64::from(n)).clamp(0, last as i64) as usize,
                KeyStep::Min => 0,
                KeyStep::Max => last,
                KeyStep::None => p,
            };
            let shows: Vec<bool> = (0..order.len()).filter(|&k| k != j).map(|k| visible.contains(&k)).collect();
            changed |= move_to(order, j, spot(&shows, q));
        }
        changed
    }

    /// The entry whose grip the pointer holds, once it has gone far enough
    /// to lift it, and how far below the entry's top it was taken hold of.
    fn lifted(&mut self, id: WidgetId, grips: &[WidgetId], laid: &[(f64, f64)]) -> Option<(usize, f64)> {
        let start = self.ui.m.px(DRAG_START);
        let st = &mut *self.ui.state;
        let held = grips.iter().position(|g| st.active == Some(*g)).filter(|_| st.down || st.released);
        let (press, pointer) = (st.press_pos, st.pointer);
        // Whether it is lifted, the hold, and the press that lifted it.
        let lift = st.floats(id, [0.0; 4]);
        let Some(i) = held else {
            lift[0] = 0.0;
            return None;
        };
        if lift[0] == 0.0 || (lift[2], lift[3]) != (press.x, press.y) {
            if pointer.distance(press) < start {
                lift[0] = 0.0;
                return None;
            }
            *lift = [1.0, press.y - laid[i].0, press.x, press.y];
        }
        Some((i, lift[1]))
    }

    /// One entry: its rows set in to clear its grip, then the grip down
    /// their left. `None` when it declared no rows; else what the keys
    /// asked of it.
    fn place(&mut self, grip: WidgetId, lifted: bool, f: impl FnOnce(&mut Card)) -> Option<KeyStep> {
        let m = self.ui.m;
        let (indent, strip, at) = (self.indent, self.grip_w, self.ui.cursor().y);
        self.indent += m.px(INDENT);
        self.grip_w = m.px(GRIP_W);
        self.first = None;
        f(self);
        (self.indent, self.grip_w) = (indent, strip);
        let h = self.ui.cursor().y - at;
        *self.ui.state.floats(grip.with("laid"), [0.0; 4]) = [at, h, 0.0, 0.0];
        let first = self.first.take().filter(|_| h > 0.0)?;
        let x = self.ui.cursor().x;
        let area = Rect::new(Vec2::new(x, at), Vec2::new(x + m.px(GRIP_W), at + h - m.gap));
        Some(self.handle(grip, area, first.center().y, lifted))
    }

    /// The grip down `area`, its dots level with `y` (the middle of the
    /// entry's first row): what the keys asked of it.
    fn handle(&mut self, id: WidgetId, area: Rect, y: f64, lifted: bool) -> KeyStep {
        let m = self.ui.m;
        let ui = &mut *self.ui;
        let r = ui.interact(id, area, Sense::GRAB);
        let focused = ui.focusable(id, area);
        let hot = r.hovered || r.held;
        if hot || lifted {
            ui.state.cursor_icon = if lifted { CursorIcon::Grabbing } else { CursorIcon::Pointer };
        }
        let dots = Vec2::new(area.min.x + m.px(ROW_PAD + DOT_GAP * 0.5 + DOT_R), y);
        let face = Rect::from_center_size(dots, Vec2::new(m.px(36.0), m.px(48.0)));
        if hot {
            ui.draw.rounded_rect(face, m.px(10.0), look::wash(0.07));
        }
        let ink = if lifted { ui.theme.accent } else if hot { look::text() } else { look::text_dim() };
        for (across, down) in [(-0.5, -1.0), (0.5, -1.0), (-0.5, 0.0), (0.5, 0.0), (-0.5, 1.0), (0.5, 1.0)] {
            ui.draw.circle(dots + Vec2::new(across, down) * m.px(DOT_GAP), m.px(DOT_R), ink);
        }
        ui.focus_ring(id, face);
        if focused { ui.key_step(id) } else { KeyStep::None }
    }

    /// The open place a lifted entry `h` tall would land in.
    fn hole(&mut self, h: f64) {
        let m = self.ui.m;
        let (at, w) = (self.ui.cursor(), self.ui.avail_width());
        let rect = Rect::from_min_size(at, Vec2::new(w, (h - m.gap).max(0.0))).shrink(m.px(8.0));
        let accent = self.ui.theme.accent;
        self.ui.draw.rounded_rect(rect, m.px(10.0), accent.fade(0.08));
        self.ui.draw.stroke_rect(rect, m.px(1.5), m.px(10.0), accent.fade(0.4));
        self.ui.space(h);
        self.rows += 1;
    }
}

/// How many of the entries that show a lifted one lands after, its top
/// at `y`: each whose middle its top has gone past, as they would lie
/// from `top` without it (`others`: their heights, none for one that
/// doesn't show). So it moves past one once it covers half of it.
fn passed(others: &[f64], top: f64, y: f64) -> usize {
    let mut at = top;
    let mut n = 0;
    for &h in others {
        if h > 0.0 && y > at + h * 0.5 {
            n += 1;
        }
        at += h;
    }
    n
}

/// Where among `others` (whether each shows) an entry goes to come after
/// `nth` of the ones that show: right after the last of them, or before
/// the first that shows when it is none. The ones that don't show keep
/// their places beside the ones that do.
fn spot(others: &[bool], nth: usize) -> usize {
    if nth == 0 {
        return others.iter().position(|&s| s).unwrap_or(0);
    }
    others.iter().enumerate().filter(|(_, s)| **s).nth(nth - 1).map_or(others.len(), |(k, _)| k + 1)
}

/// Move `items[from]` to `to`, the ones between shifting over one.
/// `true` when it moved.
fn move_to<T>(items: &mut [T], from: usize, to: usize) -> bool {
    if from < to {
        items[from..=to].rotate_left(1);
    } else if to < from {
        items[to..=from].rotate_right(1);
    }
    from != to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_one_shifts_the_ones_between() {
        let mut v = ['a', 'b', 'c', 'd'];
        assert!(move_to(&mut v, 0, 2));
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        assert!(move_to(&mut v, 3, 0));
        assert_eq!(v, ['d', 'b', 'c', 'a']);
        assert!(!move_to(&mut v, 1, 1));
    }

    #[test]
    fn a_lifted_entry_goes_past_one_it_covers_half_of() {
        // Two entries 100 tall from 0 with one this machine lacks between.
        let others = [100.0, 0.0, 100.0];
        assert_eq!(passed(&others, 0.0, 40.0), 0);
        assert_eq!(passed(&others, 0.0, 60.0), 1);
        assert_eq!(passed(&others, 0.0, 120.0), 1, "the missing one doesn't count");
        assert_eq!(passed(&others, 0.0, 160.0), 2);
    }

    #[test]
    fn what_doesnt_show_keeps_its_place_beside_what_does() {
        let others = [false, true, false, true];
        assert_eq!((spot(&others, 0), spot(&others, 1), spot(&others, 2)), (1, 2, 4));
        assert_eq!((spot(&[false, false], 0), spot(&[], 0)), (0, 0));
    }
}
