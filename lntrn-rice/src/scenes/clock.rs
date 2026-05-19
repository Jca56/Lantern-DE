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

        // Fill the window — measure once with the actual font, then scale so the
        // text width hits ~92% of the window width (or 80% of height, whichever
        // is tighter). Digital-7 has wide glyphs, so its width factor differs
        // from a generic sans — measuring with the right family keeps it tight.
        let probe = 100.0_f32;
        let probe_w = text.measure_width_family(&s, probe, FONT).max(1.0);
        let by_width = (ctx.wf * 0.92) / probe_w * probe;
        let by_height = ctx.hf * 0.80 / 1.2;
        let font_size = by_width.min(by_height).max(48.0 * ctx.scale);

        let w = text.measure_width_family(&s, font_size, FONT);
        let x = (ctx.wf - w) * 0.5;
        let y = (ctx.hf - font_size * 1.2) * 0.5;

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
