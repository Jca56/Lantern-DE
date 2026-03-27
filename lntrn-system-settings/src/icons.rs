use lntrn_render::Color;

// ── SVG path commands ───────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum PathCmd {
    Move(f32, f32),
    Line(f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32), // cp1x, cp1y, cp2x, cp2y, x, y
    Close,
}

// ── Tiny path rasterizer ────────────────────────────────────────────────────
// Scanline fill with 4x supersampling for antialiasing.
// Input paths are defined in a normalized [0..viewbox] coordinate space,
// scaled to the target pixel buffer at rasterization time.

pub fn rasterize_path(
    cmds: &[PathCmd],
    viewbox_w: f32,
    viewbox_h: f32,
    buf_w: u32,
    buf_h: u32,
    color: Color,
) -> Vec<u8> {
    let sx = buf_w as f32 / viewbox_w;
    let sy = buf_h as f32 / viewbox_h;
    let ss = 4u32; // supersampling factor
    let ss_w = buf_w * ss;
    let ss_h = buf_h * ss;
    let ssx = sx * ss as f32;
    let ssy = sy * ss as f32;

    // Flatten path to line segments in supersampled space
    let segments = flatten_path(cmds, ssx, ssy);

    // Scanline fill using even-odd rule on supersampled grid
    let mut coverage = vec![0u32; (buf_w * buf_h) as usize];

    for y_ss in 0..ss_h {
        let y_f = y_ss as f32 + 0.5;
        // Find all x-intersections at this scanline
        let mut xs = Vec::new();
        for seg in &segments {
            if let Some(x) = intersect_scanline(seg.0, seg.1, seg.2, seg.3, y_f) {
                xs.push(x);
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Fill between pairs (even-odd)
        let out_y = y_ss / ss;
        if out_y >= buf_h { continue; }
        let mut i = 0;
        while i + 1 < xs.len() {
            let x0 = (xs[i].max(0.0) as u32).min(ss_w);
            let x1 = (xs[i + 1].max(0.0).ceil() as u32).min(ss_w);
            for x_ss in x0..x1 {
                let out_x = x_ss / ss;
                if out_x < buf_w {
                    coverage[(out_y * buf_w + out_x) as usize] += 1;
                }
            }
            i += 2;
        }
    }

    // Convert coverage to RGBA
    let max_samples = ss * ss;
    let srgb = color.to_srgb8();
    let mut rgba = vec![0u8; (buf_w * buf_h * 4) as usize];
    for i in 0..(buf_w * buf_h) as usize {
        let alpha = ((coverage[i] as f32 / max_samples as f32) * srgb[3] as f32).round() as u8;
        if alpha > 0 {
            let idx = i * 4;
            rgba[idx] = srgb[0];
            rgba[idx + 1] = srgb[1];
            rgba[idx + 2] = srgb[2];
            rgba[idx + 3] = alpha;
        }
    }
    rgba
}

// ── Flatten path to line segments ───────────────────────────────────────────

fn flatten_path(cmds: &[PathCmd], sx: f32, sy: f32) -> Vec<(f32, f32, f32, f32)> {
    let mut segs = Vec::new();
    let mut cx = 0.0f32;
    let mut cy = 0.0f32;
    let mut start_x = 0.0f32;
    let mut start_y = 0.0f32;

    for cmd in cmds {
        match *cmd {
            PathCmd::Move(x, y) => {
                cx = x * sx;
                cy = y * sy;
                start_x = cx;
                start_y = cy;
            }
            PathCmd::Line(x, y) => {
                let nx = x * sx;
                let ny = y * sy;
                segs.push((cx, cy, nx, ny));
                cx = nx;
                cy = ny;
            }
            PathCmd::Cubic(c1x, c1y, c2x, c2y, x, y) => {
                let p0x = cx;
                let p0y = cy;
                let p1x = c1x * sx;
                let p1y = c1y * sy;
                let p2x = c2x * sx;
                let p2y = c2y * sy;
                let p3x = x * sx;
                let p3y = y * sy;
                flatten_cubic(&mut segs, p0x, p0y, p1x, p1y, p2x, p2y, p3x, p3y, 0);
                cx = p3x;
                cy = p3y;
            }
            PathCmd::Close => {
                if (cx - start_x).abs() > 0.01 || (cy - start_y).abs() > 0.01 {
                    segs.push((cx, cy, start_x, start_y));
                }
                cx = start_x;
                cy = start_y;
            }
        }
    }
    segs
}

fn flatten_cubic(
    segs: &mut Vec<(f32, f32, f32, f32)>,
    x0: f32, y0: f32, x1: f32, y1: f32,
    x2: f32, y2: f32, x3: f32, y3: f32,
    depth: u32,
) {
    // Flatness test: are control points close to the line from p0 to p3?
    let dx = x3 - x0;
    let dy = y3 - y0;
    let d1 = ((x1 - x3) * dy - (y1 - y3) * dx).abs();
    let d2 = ((x2 - x3) * dy - (y2 - y3) * dx).abs();
    let tolerance = 0.5; // half pixel in supersampled space

    if (d1 + d2) * (d1 + d2) < tolerance * tolerance * (dx * dx + dy * dy) || depth > 8 {
        segs.push((x0, y0, x3, y3));
        return;
    }

    // De Casteljau subdivision at t=0.5
    let m01x = (x0 + x1) * 0.5;
    let m01y = (y0 + y1) * 0.5;
    let m12x = (x1 + x2) * 0.5;
    let m12y = (y1 + y2) * 0.5;
    let m23x = (x2 + x3) * 0.5;
    let m23y = (y2 + y3) * 0.5;
    let m012x = (m01x + m12x) * 0.5;
    let m012y = (m01y + m12y) * 0.5;
    let m123x = (m12x + m23x) * 0.5;
    let m123y = (m12y + m23y) * 0.5;
    let mx = (m012x + m123x) * 0.5;
    let my = (m012y + m123y) * 0.5;

    flatten_cubic(segs, x0, y0, m01x, m01y, m012x, m012y, mx, my, depth + 1);
    flatten_cubic(segs, mx, my, m123x, m123y, m23x, m23y, x3, y3, depth + 1);
}

// ── Scanline intersection ───────────────────────────────────────────────────

fn intersect_scanline(x0: f32, y0: f32, x1: f32, y1: f32, y: f32) -> Option<f32> {
    // Does this segment cross scanline y?
    if (y0 <= y && y1 > y) || (y1 <= y && y0 > y) {
        let t = (y - y0) / (y1 - y0);
        Some(x0 + t * (x1 - x0))
    } else {
        None
    }
}

// ── Icon path data ──────────────────────────────────────────────────────────
// All icons defined in a 24x24 viewbox (standard icon grid).

/// Appearance — paintbrush icon
pub fn icon_appearance() -> Vec<PathCmd> {
    use PathCmd::*;
    vec![
        // Brush body (angled rectangle)
        Move(6.0, 18.0),
        Cubic(6.0, 16.0, 7.0, 14.0, 9.0, 12.0),
        Line(12.0, 9.0),
        Cubic(14.0, 7.0, 16.0, 6.0, 18.0, 6.0),
        Line(18.0, 6.0),
        Cubic(19.5, 6.0, 20.5, 7.0, 20.0, 8.5),
        Line(15.0, 15.0),
        Cubic(14.0, 16.5, 12.5, 17.5, 11.0, 18.0),
        // Brush tip (rounded bottom)
        Cubic(9.0, 19.0, 7.0, 20.0, 5.0, 20.0),
        Cubic(4.0, 20.0, 4.0, 19.0, 5.0, 18.5),
        Close,
    ]
}

/// Window Manager — overlapping windows icon
pub fn icon_window_manager() -> Vec<PathCmd> {
    use PathCmd::*;
    vec![
        // Back window
        Move(7.0, 5.0),
        Line(20.0, 5.0),
        Cubic(20.5, 5.0, 21.0, 5.5, 21.0, 6.0),
        Line(21.0, 16.0),
        Cubic(21.0, 16.5, 20.5, 17.0, 20.0, 17.0),
        Line(13.0, 17.0),
        Line(13.0, 11.0),
        Line(7.0, 11.0),
        Line(7.0, 6.0),
        Cubic(7.0, 5.5, 7.0, 5.0, 7.0, 5.0),
        Close,
        // Front window
        Move(4.0, 9.0),
        Line(15.0, 9.0),
        Cubic(15.5, 9.0, 16.0, 9.5, 16.0, 10.0),
        Line(16.0, 19.0),
        Cubic(16.0, 19.5, 15.5, 20.0, 15.0, 20.0),
        Line(4.0, 20.0),
        Cubic(3.5, 20.0, 3.0, 19.5, 3.0, 19.0),
        Line(3.0, 10.0),
        Cubic(3.0, 9.5, 3.5, 9.0, 4.0, 9.0),
        Close,
        // Front window title bar
        Move(3.0, 10.0),
        Line(16.0, 10.0),
        Line(16.0, 12.0),
        Line(3.0, 12.0),
        Close,
    ]
}

/// Input — keyboard icon
pub fn icon_input() -> Vec<PathCmd> {
    use PathCmd::*;
    vec![
        // Keyboard body (rounded rectangle)
        Move(3.0, 7.0),
        Cubic(3.0, 6.0, 3.8, 5.0, 5.0, 5.0),
        Line(19.0, 5.0),
        Cubic(20.2, 5.0, 21.0, 6.0, 21.0, 7.0),
        Line(21.0, 17.0),
        Cubic(21.0, 18.0, 20.2, 19.0, 19.0, 19.0),
        Line(5.0, 19.0),
        Cubic(3.8, 19.0, 3.0, 18.0, 3.0, 17.0),
        Close,
        // Key row 1 - three keys
        Move(5.5, 7.5),
        Line(8.0, 7.5),
        Line(8.0, 9.5),
        Line(5.5, 9.5),
        Close,
        Move(10.0, 7.5),
        Line(14.0, 7.5),
        Line(14.0, 9.5),
        Line(10.0, 9.5),
        Close,
        Move(16.0, 7.5),
        Line(18.5, 7.5),
        Line(18.5, 9.5),
        Line(16.0, 9.5),
        Close,
        // Key row 2 - two keys
        Move(5.5, 11.0),
        Line(9.0, 11.0),
        Line(9.0, 13.0),
        Line(5.5, 13.0),
        Close,
        Move(11.0, 11.0),
        Line(18.5, 11.0),
        Line(18.5, 13.0),
        Line(11.0, 13.0),
        Close,
        // Spacebar
        Move(7.0, 14.5),
        Line(17.0, 14.5),
        Line(17.0, 16.5),
        Line(7.0, 16.5),
        Close,
    ]
}

/// Display — monitor icon
pub fn icon_display() -> Vec<PathCmd> {
    use PathCmd::*;
    vec![
        // Monitor bezel
        Move(3.0, 4.0),
        Cubic(3.0, 3.5, 3.5, 3.0, 4.0, 3.0),
        Line(20.0, 3.0),
        Cubic(20.5, 3.0, 21.0, 3.5, 21.0, 4.0),
        Line(21.0, 15.0),
        Cubic(21.0, 15.5, 20.5, 16.0, 20.0, 16.0),
        Line(4.0, 16.0),
        Cubic(3.5, 16.0, 3.0, 15.5, 3.0, 15.0),
        Close,
        // Screen area (inner)
        Move(5.0, 5.0),
        Line(19.0, 5.0),
        Line(19.0, 14.0),
        Line(5.0, 14.0),
        Close,
        // Stand neck
        Move(10.5, 16.0),
        Line(13.5, 16.0),
        Line(13.5, 18.5),
        Line(10.5, 18.5),
        Close,
        // Stand base
        Move(8.0, 18.5),
        Line(16.0, 18.5),
        Cubic(16.5, 18.5, 17.0, 19.0, 17.0, 19.5),
        Line(17.0, 20.0),
        Line(7.0, 20.0),
        Line(7.0, 19.5),
        Cubic(7.0, 19.0, 7.5, 18.5, 8.0, 18.5),
        Close,
    ]
}

/// Power — battery with lightning bolt icon
pub fn icon_power() -> Vec<PathCmd> {
    use PathCmd::*;
    // Battery scaled up to fill more of the 24x24 viewbox.
    // Original Y: 5.5–18.5 (13px). Scaled to Y: 3.0–21.0 (18px).
    // Scale: 18/13 ≈ 1.385, offset to center vertically.
    vec![
        // Battery body (rounded rectangle, horizontal)
        Move(2.0, 5.1),
        Cubic(2.0, 3.7, 3.1, 3.0, 4.7, 3.0),
        Line(17.3, 3.0),
        Cubic(18.9, 3.0, 19.5, 3.7, 19.5, 5.1),
        Line(19.5, 18.9),
        Cubic(19.5, 20.3, 18.9, 21.0, 17.3, 21.0),
        Line(4.7, 21.0),
        Cubic(3.1, 21.0, 2.0, 20.3, 2.0, 18.9),
        Close,
        // Battery terminal (right nub)
        Move(19.5, 7.8),
        Line(21.0, 7.8),
        Cubic(21.7, 7.8, 22.0, 8.5, 22.0, 9.2),
        Line(22.0, 14.8),
        Cubic(22.0, 15.5, 21.7, 16.2, 21.0, 16.2),
        Line(19.5, 16.2),
        Close,
        // Lightning bolt
        Move(12.8, 4.4),
        Line(7.2, 12.7),
        Line(10.7, 12.7),
        Line(8.6, 19.6),
        Line(14.2, 11.3),
        Line(10.7, 11.3),
        Close,
    ]
}

/// App Icons panel icon — a grid of squares representing app icons.
pub fn icon_app_icons() -> Vec<PathCmd> {
    use PathCmd::*;
    // 2x2 grid of rounded squares in a 24x24 viewbox
    let mut cmds = Vec::new();
    for &(x, y) in &[(2.0, 2.0), (13.0, 2.0), (2.0, 13.0), (13.0, 13.0)] {
        let (w, h) = (9.0, 9.0);
        cmds.extend([
            Move(x, y + 1.0), Cubic(x, y, x + 1.0, y, x + 1.0, y),
            Line(x + w - 1.0, y), Cubic(x + w, y, x + w, y + 1.0, x + w, y + 1.0),
            Line(x + w, y + h - 1.0), Cubic(x + w, y + h, x + w - 1.0, y + h, x + w - 1.0, y + h),
            Line(x + 1.0, y + h), Cubic(x, y + h, x, y + h - 1.0, x, y + h - 1.0),
            Close,
        ]);
    }
    cmds
}
