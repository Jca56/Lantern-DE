//! The tool bar that sits by the region once one is set: the drawing
//! tools, the colours, the thicknesses and undo.
//!
//! Like the bottom pill, its layout is computed fresh from the screen, the
//! scale and the region, and the same [`BarLayout`] serves hit-testing and
//! drawing, so the two never drift apart.

use lntrn_render::{Color, Painter, Rect};

use super::{Tool, PALETTE, SIZES};

// Base sizes in logical px; multiplied by `scale` (>= 1.0) at layout time.
// Generous because the user prefers big, easy-to-hit controls.
const BTN: f32 = 52.0;
const BTN_GAP: f32 = 4.0;
const GROUP_GAP: f32 = 16.0;
const PANEL_PAD: f32 = 10.0;
const PANEL_RADIUS: f32 = 18.0;
const BTN_RADIUS: f32 = 12.0;
const ICON: f32 = 28.0;
/// Room left between the region and the bar, and the bar and the screen.
const MARGIN: f32 = 12.0;

fn text_tan() -> Color {
    Color::from_rgba8(0xe8, 0xdc, 0xc8, 0xff)
}
fn accent_orange() -> Color {
    Color::from_rgba8(0xff, 0x9b, 0x42, 0xff)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BarAction {
    Tool(Tool),
    Color(usize),
    Size(usize),
    Undo,
}

/// What the bar shows as chosen.
pub struct BarState {
    pub tool: Option<Tool>,
    pub color: usize,
    pub size: usize,
    pub can_undo: bool,
}

struct Slot {
    action: BarAction,
    rect: Rect,
}

pub struct BarLayout {
    panel: Rect,
    slots: Vec<Slot>,
    scale: f32,
}

impl BarLayout {
    /// Place the bar for a region `x, y, w, h`: just below it, else just
    /// above (clear of the `readout_h` its size readout takes there), else
    /// along its bottom edge inside it.
    pub fn compute(
        screen_w: f32,
        screen_h: f32,
        scale: f32,
        (rx, ry, rw, rh): (f32, f32, f32, f32),
        readout_h: f32,
    ) -> Self {
        let groups: [Vec<BarAction>; 4] = [
            Tool::ALL.into_iter().map(BarAction::Tool).collect(),
            (0..PALETTE.len()).map(BarAction::Color).collect(),
            (0..SIZES).map(BarAction::Size).collect(),
            vec![BarAction::Undo],
        ];
        let count: usize = groups.iter().map(Vec::len).sum();
        let base_w = 2.0 * PANEL_PAD
            + count as f32 * BTN
            + (count - groups.len()) as f32 * BTN_GAP
            + (groups.len() - 1) as f32 * GROUP_GAP;

        // Narrower than the bar wants: shrink the bar to what there is.
        let mut s = scale.max(1.0);
        let most = screen_w - 2.0 * MARGIN * s;
        if base_w * s > most && most > 0.0 {
            s = most / base_w;
        }
        let (panel_w, panel_h) = (base_w * s, (BTN + 2.0 * PANEL_PAD) * s);
        let margin = MARGIN * s;

        let x = (rx + rw / 2.0 - panel_w / 2.0)
            .min(screen_w - margin - panel_w)
            .max(margin);
        let below = ry + rh + margin;
        let above = ry - readout_h - margin - panel_h;
        let y = if below + panel_h + margin <= screen_h {
            below
        } else if above >= margin {
            above
        } else {
            ry + rh - margin - panel_h
        };

        let mut slots = Vec::with_capacity(count);
        let mut bx = x + PANEL_PAD * s;
        for group in groups {
            for action in group {
                slots.push(Slot {
                    action,
                    rect: Rect::new(bx, y + PANEL_PAD * s, BTN * s, BTN * s),
                });
                bx += (BTN + BTN_GAP) * s;
            }
            bx += (GROUP_GAP - BTN_GAP) * s;
        }

        Self {
            panel: Rect::new(x, y, panel_w, panel_h),
            slots,
            scale: s,
        }
    }

    /// True if the point is anywhere on the bar (so a press there is
    /// absorbed rather than drawing or moving the region).
    pub fn panel_contains(&self, x: f32, y: f32) -> bool {
        self.panel.contains(x, y)
    }

    /// The action whose button contains the point, if any.
    pub fn action_at(&self, x: f32, y: f32) -> Option<BarAction> {
        self.slots
            .iter()
            .find(|slot| slot.rect.contains(x, y))
            .map(|slot| slot.action)
    }

    /// Draw the bar. `cursor` drives hover feedback.
    pub fn render(&self, painter: &mut Painter, cursor: (f32, f32), state: &BarState) {
        let s = self.scale;

        // Pill background + hairline border, as the bottom pill has.
        painter.rect_filled(
            self.panel,
            PANEL_RADIUS * s,
            Color::from_rgba8(0x1a, 0x16, 0x12, 0xee),
        );
        painter.rect_stroke(
            self.panel,
            PANEL_RADIUS * s,
            1.0 * s,
            Color::from_rgba8(0xff, 0x9b, 0x42, 0x55),
        );

        for slot in &self.slots {
            let hovered = slot.rect.contains(cursor.0, cursor.1);
            let enabled = slot.action != BarAction::Undo || state.can_undo;
            // The tool in hand and the thickness light up as the pill's
            // active button does; the colour gets a ring instead.
            let lit = match slot.action {
                BarAction::Tool(tool) => state.tool == Some(tool),
                BarAction::Size(size) => state.size == size,
                BarAction::Color(_) | BarAction::Undo => false,
            };
            if lit {
                painter.rect_filled(slot.rect, BTN_RADIUS * s, accent_orange().with_alpha(0.22));
                painter.rect_stroke(slot.rect, BTN_RADIUS * s, 1.5 * s, accent_orange());
            } else if hovered && enabled {
                painter.rect_filled(
                    slot.rect,
                    BTN_RADIUS * s,
                    Color::from_rgba8(0xff, 0xff, 0xff, 0x22),
                );
            }
            let fg = if lit { accent_orange() } else { text_tan() };
            let fg = if enabled { fg } else { fg.with_alpha(0.35) };

            let (cx, cy) = (slot.rect.center_x(), slot.rect.center_y());
            let icon = Rect::new(cx - ICON * s / 2.0, cy - ICON * s / 2.0, ICON * s, ICON * s);
            match slot.action {
                BarAction::Tool(tool) => draw_tool_icon(painter, tool, icon, fg, s),
                BarAction::Color(index) => {
                    let [r, g, b] = PALETTE[index];
                    painter.circle_filled(cx, cy, 13.0 * s, Color::from_rgb8(r, g, b));
                    if state.color == index {
                        painter.circle_stroke(cx, cy, 19.0 * s, 3.0 * s, text_tan());
                    }
                }
                BarAction::Size(size) => {
                    painter.circle_filled(cx, cy, [4.0, 7.0, 11.0][size] * s, fg);
                }
                BarAction::Undo => draw_undo_icon(painter, icon, fg, s),
            }
        }
    }
}

/// A tool's picture, drawn from lines in the box `r`.
fn draw_tool_icon(painter: &mut Painter, tool: Tool, r: Rect, color: Color, scale: f32) {
    let at = |fx: f32, fy: f32| (r.x + r.w * fx, r.y + r.h * fy);
    let w = 2.5 * scale;
    match tool {
        // A squiggle.
        Tool::Pen => painter.polyline_round(
            &[
                at(0.06, 0.72),
                at(0.28, 0.26),
                at(0.5, 0.68),
                at(0.72, 0.28),
                at(0.94, 0.64),
            ],
            w,
            color,
        ),
        // A see-through band over a line of writing.
        Tool::Marker => {
            let (a, b) = (at(0.06, 0.5), at(0.94, 0.5));
            painter.line(a.0, a.1, b.0, b.1, r.h * 0.5, color.with_alpha(0.45));
            painter.polyline_round(&[at(0.2, 0.5), at(0.8, 0.5)], w, color);
        }
        Tool::Arrow => {
            let tip = at(0.88, 0.12);
            painter.polyline_round(&[at(0.12, 0.88), tip], w, color);
            painter.polyline_round(&[at(0.46, 0.12), tip, at(0.88, 0.54)], w, color);
        }
        Tool::Rect => painter.polyline_round(
            &[
                at(0.08, 0.2),
                at(0.92, 0.2),
                at(0.92, 0.8),
                at(0.08, 0.8),
                at(0.08, 0.2),
            ],
            w,
            color,
        ),
        Tool::Ellipse => {
            painter.circle_stroke(r.center_x(), r.center_y(), r.w * 0.42, w, color);
        }
        // A capital T.
        Tool::Text => {
            painter.polyline_round(&[at(0.16, 0.14), at(0.84, 0.14)], w * 1.2, color);
            painter.polyline_round(&[at(0.5, 0.14), at(0.5, 0.88)], w * 1.2, color);
        }
    }
}

/// Undo: an arrow that turns back on itself.
fn draw_undo_icon(painter: &mut Painter, r: Rect, color: Color, scale: f32) {
    let at = |fx: f32, fy: f32| (r.x + r.w * fx, r.y + r.h * fy);
    let w = 2.5 * scale;
    let tip = at(0.14, 0.4);
    painter.polyline_round(&[at(0.86, 0.84), at(0.86, 0.4), tip], w, color);
    painter.polyline_round(&[at(0.38, 0.16), tip, at(0.38, 0.64)], w, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (f32, f32) = (1920.0, 1080.0);

    fn bar(region: (f32, f32, f32, f32)) -> BarLayout {
        BarLayout::compute(SCREEN.0, SCREEN.1, 1.0, region, 34.0)
    }

    #[test]
    fn it_sits_below_the_region_and_every_button_is_hit_where_it_is_drawn() {
        let b = bar((600.0, 300.0, 500.0, 300.0));
        assert!(b.panel.y >= 600.0, "below the region");
        assert_eq!(b.slots.len(), Tool::ALL.len() + PALETTE.len() + SIZES + 1);
        for slot in &b.slots {
            let (x, y) = (slot.rect.center_x(), slot.rect.center_y());
            assert!(b.panel_contains(x, y));
            assert!(b.action_at(x, y) == Some(slot.action));
        }
    }

    #[test]
    fn it_moves_above_then_inside_when_there_is_no_room_below() {
        // The region reaches the bottom of the screen: above it.
        let b = bar((600.0, 500.0, 500.0, 580.0));
        assert!(b.panel.y + b.panel.h <= 500.0 - 34.0);
        // The region is the whole screen: along its bottom edge, inside.
        let b = bar((0.0, 0.0, SCREEN.0, SCREEN.1));
        assert!(b.panel.y > SCREEN.1 / 2.0 && b.panel.y + b.panel.h < SCREEN.1);
    }

    #[test]
    fn it_stays_on_screen_and_shrinks_to_fit_a_narrow_one() {
        let b = bar((1800.0, 300.0, 100.0, 100.0));
        assert!(b.panel.x + b.panel.w <= SCREEN.0);
        let b = BarLayout::compute(600.0, 800.0, 1.0, (100.0, 100.0, 200.0, 200.0), 34.0);
        assert!(b.panel.x >= 0.0 && b.panel.x + b.panel.w <= 600.0);
        assert!(b.scale < 1.0);
    }
}
