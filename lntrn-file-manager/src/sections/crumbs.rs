//! The breadcrumb trail of the path bar.
//!
//! One layout, from measured text, that the click zones (render.rs) and the
//! drawing (nav.rs, and the unfocused pane in split.rs) are both made from.
//! There used to be two copies of the arithmetic, both sizing a segment as
//! its byte count times 0.45 em: "Home" and "Documents" ran over their `/`
//! separators, the clickable box ended before the word did, a long trail
//! the estimate believed in was drawn over the buttons right of the bar,
//! and when even the last segment did not fit every segment was skipped,
//! leaving a bare "... /" and nothing to click.
//!
//! The current folder is always shown: cut with an ellipsis if it must be,
//! but never dropped.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use super::{breadcrumb_segments, fit_label};

const FONT: f32 = 22.0;
/// Space before the first segment.
const LEAD: f32 = 4.0;
/// Space inside a segment's box, left and right of its text.
const PAD_X: f32 = 6.0;
/// Width of the gap a `/` sits in.
const SEP_W: f32 = 14.0;
/// Strip kept free at the bar's right end: clicking it starts editing the
/// path. (A trail filling the whole bar left no way to do that.)
const EDIT_STRIP: f32 = 28.0;
/// Shown in place of the segments left out at the front.
const ELLIPSIS: &str = "\u{2026}";
/// Extra layout width for a label queued at its own measured width.
const SLACK: f32 = 4.0;
/// Segments that get a click zone (the zone ids are a range of 100).
pub const MAX_ZONES: usize = 100;

/// One segment as shown.
pub struct Crumb {
    /// Index into the path's segments (`breadcrumb_segments`).
    pub index: usize,
    /// The name, cut to fit if it is the last one and the bar is short.
    pub label: String,
    /// Hover highlight and click zone.
    pub rect: Rect,
    text_w: f32,
}

pub struct Trail {
    /// Segments left out at the front. A zone's segment is its position
    /// among the shown ones plus this (`App::breadcrumb_skip`).
    pub skip: usize,
    pub crumbs: Vec<Crumb>,
    /// Everything in the bar that is not a segment: a click there starts
    /// editing the path. Up to two pieces (the ellipsis at the front, the
    /// free strip at the end).
    pub edit: Vec<Rect>,
    /// Left edge of the leading ellipsis' text, when segments are left out.
    ellipsis_x: Option<f32>,
    /// Left edge of each `/`.
    seps: Vec<f32>,
    text_y: f32,
    font: f32,
}

struct Metrics {
    lead: f32,
    pad: f32,
    sep: f32,
    /// Width the leading ellipsis takes, its separator included.
    ellipsis: f32,
    /// Text width under which the last segment says nothing: below it the
    /// ellipsis gives up its place.
    min_last: f32,
}

#[derive(Debug, PartialEq)]
struct Plan {
    skip: usize,
    ellipsis: bool,
    /// Width the last segment's text may take.
    last_text_w: f32,
}

/// Which segments are shown in `avail` px. `widths` are the measured text
/// widths of all segments, first to last.
fn plan(widths: &[f32], avail: f32, m: &Metrics) -> Plan {
    let n = widths.len();
    let Some(&last) = widths.last() else {
        return Plan {
            skip: 0,
            ellipsis: false,
            last_text_w: 0.0,
        };
    };
    // Width of the trail from segment `k` on.
    let tail = |k: usize| -> f32 {
        let boxes: f32 = widths[k..].iter().map(|w| w + 2.0 * m.pad).sum();
        m.lead + boxes + (n - 1 - k) as f32 * m.sep
    };
    if tail(0) <= avail {
        return Plan {
            skip: 0,
            ellipsis: false,
            last_text_w: last,
        };
    }
    for k in 1..n {
        if m.ellipsis + tail(k) <= avail {
            return Plan {
                skip: k,
                ellipsis: true,
                last_text_w: last,
            };
        }
    }
    // Not even the last segment fits whole: it stays, cut. The ellipsis
    // stays with it while that leaves the name something to say.
    let alone = (avail - m.lead - 2.0 * m.pad).max(0.0);
    let with_ellipsis = alone - m.ellipsis;
    if n > 1 && with_ellipsis >= last.min(m.min_last) {
        Plan {
            skip: n - 1,
            ellipsis: true,
            last_text_w: last.min(with_ellipsis),
        }
    } else {
        Plan {
            skip: n - 1,
            ellipsis: false,
            last_text_w: last.min(alone),
        }
    }
}

/// Lay the trail for `dir` out in `bar`.
pub fn layout(text: &mut TextRenderer, dir: &std::path::Path, bar: Rect, s: f32) -> Trail {
    let font = FONT * s;
    let mut trail = Trail {
        skip: 0,
        crumbs: Vec::new(),
        edit: Vec::new(),
        ellipsis_x: None,
        seps: Vec::new(),
        text_y: bar.y + (bar.h - font) * 0.5,
        font,
    };
    if bar.w <= 0.0 {
        return trail;
    }
    // No room for even a letter in its box (what a ROOT badge left of a
    // narrow strip): the sliver is just a place to click into the path.
    if bar.w < (LEAD + 2.0 * PAD_X + FONT) * s {
        trail.edit.push(bar);
        return trail;
    }
    let segments = breadcrumb_segments(dir, s);
    // Memoised: the same few names are asked for every frame.
    let widths: Vec<f32> = segments
        .iter()
        .map(|(name, _)| fit_label(text, name, f32::MAX, font).1)
        .collect();
    let pad = PAD_X * s;
    let sep = SEP_W * s;
    let ellipsis_w = fit_label(text, ELLIPSIS, f32::MAX, font).1;
    let metrics = Metrics {
        lead: LEAD * s,
        pad,
        sep,
        ellipsis: ellipsis_w + 2.0 * pad + sep,
        min_last: 3.0 * font,
    };
    // A narrow bar gives the strip less, not the trail nothing.
    let strip = (EDIT_STRIP * s).min(bar.w * 0.2);
    let plan = plan(&widths, bar.w - strip, &metrics);
    trail.skip = plan.skip;

    let mut cx = bar.x + metrics.lead;
    if plan.ellipsis {
        trail.ellipsis_x = Some(cx + pad);
        let w = ellipsis_w + 2.0 * pad;
        trail
            .edit
            .push(Rect::new(bar.x, bar.y, cx + w - bar.x, bar.h));
        cx += w;
        trail.seps.push(cx);
        cx += sep;
    }
    for (index, (name, _)) in segments.iter().enumerate().skip(plan.skip) {
        if index > plan.skip {
            trail.seps.push(cx);
            cx += sep;
        }
        let is_last = index + 1 == segments.len();
        let (label, text_w) = if is_last && plan.last_text_w < widths[index] {
            fit_label(text, name, plan.last_text_w, font)
        } else {
            (name.clone(), widths[index])
        };
        let w = text_w + 2.0 * pad;
        trail.crumbs.push(Crumb {
            index,
            label,
            rect: Rect::new(cx, bar.y + 2.0 * s, w, bar.h - 4.0 * s),
            text_w,
        });
        cx += w;
    }
    let free = bar.x + bar.w - cx;
    if free > 0.0 {
        trail.edit.push(Rect::new(cx, bar.y, free, bar.h));
    }
    trail
}

impl Trail {
    /// Draw the trail. `hovered[i]` goes with `crumbs[i]`; `muted` is the
    /// unfocused pane's look (one colour, no highlight).
    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        palette: &FoxPalette,
        hovered: &[bool],
        muted: bool,
        screen: (u32, u32),
        s: f32,
    ) {
        let size = FontSize::Custom(self.font);
        let faint = palette.muted.with_alpha(0.3);
        if let Some(x) = self.ellipsis_x {
            TextLabel::new(ELLIPSIS, x, self.text_y)
                .size(size)
                .color(palette.muted)
                .draw(text, screen.0, screen.1);
        }
        if !self.seps.is_empty() {
            let slash_w = fit_label(text, "/", f32::MAX, self.font).1;
            for x in &self.seps {
                TextLabel::new("/", x + (SEP_W * s - slash_w) * 0.5, self.text_y)
                    .size(size)
                    .color(faint)
                    .draw(text, screen.0, screen.1);
            }
        }
        let last = self.crumbs.len().saturating_sub(1);
        for (i, crumb) in self.crumbs.iter().enumerate() {
            if !muted && hovered.get(i).copied().unwrap_or(false) {
                painter.rect_filled(crumb.rect, 4.0 * s, palette.surface_2.with_alpha(0.4));
            }
            let color = if i == last && !muted {
                palette.text
            } else {
                palette.text_secondary
            };
            TextLabel::new(&crumb.label, crumb.rect.x + PAD_X * s, self.text_y)
                .size(size)
                .color(color)
                .max_width(crumb.text_w + SLACK * s)
                .draw(text, screen.0, screen.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Metrics = Metrics {
        lead: 4.0,
        pad: 6.0,
        sep: 14.0,
        ellipsis: 40.0,
        min_last: 66.0,
    };

    /// Width the plan's trail takes.
    fn used(widths: &[f32], p: &Plan) -> f32 {
        let n = widths.len();
        let mut w = M.lead + if p.ellipsis { M.ellipsis } else { 0.0 };
        for (i, text_w) in widths.iter().enumerate().skip(p.skip) {
            if i > p.skip {
                w += M.sep;
            }
            w += if i + 1 == n { p.last_text_w } else { *text_w } + 2.0 * M.pad;
        }
        w
    }

    #[test]
    fn a_trail_that_fits_is_shown_whole() {
        // Home / Documents / Projects, as Inter measures them at 22px.
        let widths = [61.6, 118.6, 92.0];
        let p = plan(&widths, 400.0, &M);
        assert_eq!(
            p,
            Plan {
                skip: 0,
                ellipsis: false,
                last_text_w: 92.0
            }
        );
        assert!(used(&widths, &p) <= 400.0);
    }

    #[test]
    fn leading_segments_go_first_and_only_as_many_as_needed() {
        let widths = [61.6, 118.6, 92.0, 140.0];
        // Whole: 4 + 412.2 + 48 + 42 = 506.2.
        assert_eq!(plan(&widths, 507.0, &M).skip, 0);
        let p = plan(&widths, 500.0, &M);
        assert_eq!((p.skip, p.ellipsis), (1, true));
        assert!(used(&widths, &p) <= 500.0);
        let p = plan(&widths, 330.0, &M);
        assert_eq!((p.skip, p.ellipsis), (2, true));
        assert!(used(&widths, &p) <= 330.0);
        assert_eq!(p.last_text_w, 140.0, "the current folder is still whole");
    }

    #[test]
    fn the_current_folder_is_never_skipped() {
        let widths = [61.6, 118.6, 400.0];
        // The audit's case: a bar too short for "… / <long name>". Every
        // segment used to be skipped (skip == len) and only "... /" drawn.
        for avail in [0.0_f32, 30.0, 100.0, 166.0, 250.0, 420.0, 455.0] {
            let p = plan(&widths, avail, &M);
            assert!(p.skip < widths.len(), "nothing shown at {avail}px");
            assert!(p.last_text_w >= 0.0);
            assert!(
                used(&widths, &p) <= avail.max(M.lead + 2.0 * M.pad),
                "{avail}px: the trail is wider than the bar"
            );
        }
        // Room for the ellipsis and a readable piece of the name: both.
        let p = plan(&widths, 250.0, &M);
        assert_eq!((p.skip, p.ellipsis), (2, true));
        assert_eq!(p.last_text_w, 250.0 - 4.0 - 12.0 - 40.0);
        // Too little for both: the name gets all of it.
        let p = plan(&widths, 100.0, &M);
        assert_eq!((p.skip, p.ellipsis), (2, false));
        assert_eq!(p.last_text_w, 100.0 - 4.0 - 12.0);
    }

    #[test]
    fn a_single_segment_has_no_ellipsis_in_front_of_it() {
        let p = plan(&[300.0], 120.0, &M);
        assert_eq!(
            p,
            Plan {
                skip: 0,
                ellipsis: false,
                last_text_w: 104.0
            }
        );
        // No path at all (never the case for a real directory): no panic.
        assert_eq!(plan(&[], 120.0, &M).skip, 0);
    }
}
