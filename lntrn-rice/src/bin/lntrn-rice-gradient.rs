use lntrn_rice::{draw_theme_background, run, theme_accent, AppConfig, Color, FrameCtx, Painter};

fn main() {
    let cfg = AppConfig {
        app_id: "lntrn-rice-gradient",
        title: "Rice — Gradient",
        initial_width: 720,
        initial_height: 480,
    };
    if let Err(e) = run(cfg, scene) {
        eprintln!("[lntrn-rice-gradient] fatal: {e}");
        std::process::exit(1);
    }
}

fn scene(painter: &mut Painter, ctx: &FrameCtx) {
    // Base: honor user's background_color + opacity so the wallpaper bleeds
    // through when opacity < 1. The gradient layers on top tint it.
    draw_theme_background(painter, ctx);

    let t = ctx.elapsed_secs;
    let accent = theme_accent();
    let (ah, asat, aval) = rgb_to_hsv(accent.r, accent.g, accent.b);

    // Saturate + brighten a bit so the gradient pops even on muted accents
    let sat = asat.max(0.55);
    let val = aval.max(0.55);

    // Two slowly-rotating linear gradients, each oscillating in hue around
    // the accent. The cross-product of two rotating gradients gives the
    // "flowing color shift" look without ever repeating.
    let angle1 = t * 0.18;
    let angle2 = t * 0.13 + std::f32::consts::FRAC_PI_2;

    let h1a = ah + (t * 0.07).sin() * 0.18;
    let h1b = ah + 0.22 + (t * 0.11).cos() * 0.18;
    let h2a = ah - 0.18 + (t * 0.09).cos() * 0.20;
    let h2b = ah + 0.45 + (t * 0.06).sin() * 0.20;

    let (r, g, b) = hsv_to_rgb(h1a, sat, val);
    let c1a = Color::rgba(r, g, b, 0.55);
    let (r, g, b) = hsv_to_rgb(h1b, sat, val * 0.85);
    let c1b = Color::rgba(r, g, b, 0.55);
    let (r, g, b) = hsv_to_rgb(h2a, sat * 0.9, val);
    let c2a = Color::rgba(r, g, b, 0.45);
    let (r, g, b) = hsv_to_rgb(h2b, sat * 0.9, val * 0.8);
    let c2b = Color::rgba(r, g, b, 0.45);

    let rect = ctx.window_rect();
    let r = ctx.corner_radius;
    painter.rect_gradient_linear(rect, r, angle1, c1a, c1b);
    painter.rect_gradient_linear(rect, r, angle2, c2a, c2b);
}

// ── HSV helpers ─────────────────────────────────────────────────────────────

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let h = if d == 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        (((g - b) / d) + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor() as i32;
    let f = h - h.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}
