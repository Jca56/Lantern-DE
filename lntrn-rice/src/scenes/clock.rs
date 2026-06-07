use std::time::{SystemTime, UNIX_EPOCH};

use lntrn_render::{Color, Painter, TextRenderer};

use crate::app::FrameCtx;
use super::{draw_theme_background, Scene};

const FONT: &str = "Square Sans Serif 7";

pub struct Clock;

impl Scene for Clock {
    fn draw(&mut self, painter: &mut Painter, text: &mut TextRenderer, ctx: &FrameCtx) {
        draw_theme_background(painter, ctx);

        let (h, m, _) = local_hms();
        let s = format!("{:02}:{:02}", h, m);

        // Fill the window. Measure at a probe size, then scale. Width uses the
        // laid-out advance; HEIGHT uses the actual ink bounds (the lit digits),
        // not the padded line box — "Square Sans Serif 7" has lots of
        // ascent/descent air, so fitting to the line box leaves the digits short
        // and floating in a short/wide window.
        let probe = 100.0_f32;
        let probe_w = text.measure_width_family(&s, probe, FONT).max(1.0);
        let (ink_h, ink_top) = text.measure_ink_height_family(&s, probe, FONT);

        let by_width = (ctx.wf * 0.95) / probe_w * probe;
        let have_ink = ink_h >= 1.0;
        let font_size = if have_ink {
            // Scale so the visible ink fills ~92% of the window height.
            by_width.min((ctx.hf * 0.92) / ink_h * probe)
        } else {
            by_width.min(ctx.hf * 0.90 / 1.2) // fallback if ink couldn't be measured
        }
        .max(48.0 * ctx.scale);

        let scale_f = font_size / probe;
        let w = text.measure_width_family(&s, font_size, FONT);
        let x = (ctx.wf - w) * 0.5;
        // Center the visible ink vertically (not the padded line box).
        let y = if have_ink {
            (ctx.hf - ink_h * scale_f) * 0.5 - ink_top * scale_f
        } else {
            (ctx.hf - font_size * 1.2) * 0.5
        };

        let screen_w = ctx.wf.round().max(1.0) as u32;
        let screen_h = ctx.hf.round().max(1.0) as u32;
        text.queue_family(&s, font_size, x, y, Color::WHITE, ctx.wf, FONT, screen_w, screen_h);
    }
}

fn local_hms() -> (u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
}
