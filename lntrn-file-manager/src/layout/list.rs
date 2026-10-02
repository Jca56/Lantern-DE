//! List view geometry shared by the header, the rows, the hit rects and the
//! icon pass: which columns there is room for, and where the rows may draw.

use lntrn_render::Rect;

use super::list_zoom_multiplier;

/// The name column never gets narrower than this (times the zoom
/// multiplier and the scale): about fifteen characters. The Modified column
/// goes first to keep it, then Size.
const NAME_MIN_W: f32 = 180.0;
const SIZE_W: f32 = 110.0;
/// "Sep 30, 2026" at the small font.
const DATE_W: f32 = 180.0;
/// Left of the name: the icon slot.
const NAME_X: f32 = 42.0;
/// Right gutter (so the last column doesn't kiss the preview pane / window
/// edge), and the gap between the name and the column after it.
const GAP: f32 = 12.0;

/// List view columns (normal mode).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListCols {
    pub name_x: f32,
    /// Widest a name may be drawn.
    pub name_w: f32,
    /// Left edge of the Size column; `None` when there is no room for it.
    pub size_x: Option<f32>,
    /// Left edge of the Modified column; `None` when there is no room.
    pub date_x: Option<f32>,
    pub size_w: f32,
    pub date_w: f32,
}

/// Columns for a list `content_rect` wide. `m` is the list zoom multiplier.
///
/// The Size and Modified columns used to be reserved whatever the width:
/// in a narrow pane the name column went negative and every name vanished.
pub fn list_columns(content_rect: Rect, m: f32, s: f32) -> ListCols {
    let u = m * s;
    let name_x = content_rect.x + NAME_X * u;
    let right = content_rect.x + content_rect.w - GAP * u;
    let (size_w, date_w) = (SIZE_W * u, DATE_W * u);
    let name_min = NAME_MIN_W * u;
    // Room for the name when the columns from `left_edge` on are kept.
    let name_room = |left_edge: f32| left_edge - GAP * u - name_x;

    let date_x = right - date_w;
    let size_x = date_x - size_w;
    if name_room(size_x) >= name_min {
        return ListCols {
            name_x,
            name_w: name_room(size_x),
            size_x: Some(size_x),
            date_x: Some(date_x),
            size_w,
            date_w,
        };
    }
    // No Modified column: Size moves to the right edge.
    let size_x = right - size_w;
    if name_room(size_x) >= name_min {
        return ListCols {
            name_x,
            name_w: name_room(size_x),
            size_x: Some(size_x),
            date_x: None,
            size_w,
            date_w,
        };
    }
    ListCols {
        name_x,
        // Names only. Never zero: a cut name is clipped to the list, an
        // empty one is a row nobody can read.
        name_w: (right - name_x).max(2.0 * GAP * u),
        size_x: None,
        date_x: None,
        size_w,
        date_w,
    }
}

/// Height of the List view's fixed "Name / Size / Modified" header. The rows
/// start below it, so it is part of the scrollable height.
pub fn list_header_h(s: f32, zoom: f32) -> f32 {
    32.0 * list_zoom_multiplier(zoom) * s
}

/// The part of a list's content area its rows are drawn and clicked in:
/// everything below the fixed header. Rows used to be clipped to the whole
/// content area, so one scrolled part-way up painted its name, its stripe
/// and its icon over the header labels and could be clicked through them.
pub fn list_rows_rect(content: Rect, s: f32, zoom: f32) -> Rect {
    let hdr_h = list_header_h(s, zoom).min(content.h.max(0.0));
    Rect::new(content.x, content.y + hdr_h, content.w, content.h - hdr_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(w: f32, m: f32) -> ListCols {
        list_columns(Rect::new(240.0, 100.0, w, 600.0), m, 1.0)
    }

    #[test]
    fn a_wide_list_has_all_three_columns_where_they_always_were() {
        let c = cols(1000.0, 1.5);
        let right = 240.0 + 1000.0 - 18.0;
        assert_eq!(c.date_x, Some(right - 270.0));
        assert_eq!(c.size_x, Some(right - 270.0 - 165.0));
        assert_eq!(c.name_x, 240.0 + 63.0);
        // What the rows used as `size_x - name_x - 12*m*s`.
        assert_eq!(c.name_w, c.size_x.unwrap() - c.name_x - 18.0);
    }

    #[test]
    fn columns_go_before_the_name_does() {
        // Split view at the largest zoom: 666px for m = 2.2 used to leave
        // the name column 117px short of nothing.
        let c = cols(666.0, 2.2);
        assert!(c.name_w >= NAME_MIN_W * 2.2);
        assert_eq!(c.date_x, None);

        // Narrower still: names only.
        let c = cols(420.0, 2.2);
        assert_eq!((c.size_x, c.date_x), (None, None));
        assert!(c.name_w > 0.0);

        // Whatever the width, the name column is there and what is kept
        // does not start left of where the name ends.
        for w in (0..1600).step_by(7) {
            for m in [0.8_f32, 1.5, 2.2] {
                let c = cols(w as f32, m);
                assert!(c.name_w > 0.0, "no name column at {w}px, m {m}");
                if let Some(size_x) = c.size_x {
                    assert!(c.name_x + c.name_w <= size_x, "{w}px, m {m}");
                    assert!(c.name_w >= NAME_MIN_W * m - 0.01, "{w}px, m {m}");
                }
                if let Some(date_x) = c.date_x {
                    assert!(c.size_x.unwrap() + c.size_w <= date_x + 0.01);
                }
            }
        }
    }

    #[test]
    fn rows_live_below_the_header() {
        let content = Rect::new(240.0, 100.0, 800.0, 600.0);
        let rows = list_rows_rect(content, 1.0, 0.5);
        assert_eq!(rows.y, 100.0 + 48.0);
        assert_eq!(rows.y + rows.h, 700.0);
        assert_eq!((rows.x, rows.w), (240.0, 800.0));
        // A pane shorter than its header has no rows, not a negative rect.
        let rows = list_rows_rect(Rect::new(0.0, 0.0, 300.0, 20.0), 1.0, 0.5);
        assert_eq!(rows.h, 0.0);
    }
}
