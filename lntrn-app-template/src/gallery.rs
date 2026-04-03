use lntrn_render::{Color, Painter, Rect, TextRenderer};

// ── Gallery palette ────────────────────────────────────────────────────────

const TEXT_PRIMARY: Color   = Color::rgb(0.88, 0.85, 0.95);
const TEXT_SECONDARY: Color = Color::rgb(0.50, 0.45, 0.62);
const ACCENT: Color         = Color::rgb(0.25, 0.65, 0.90);     // vibrant cyan-blue
const ACCENT_HOVER: Color   = Color::rgb(0.30, 0.72, 0.95);
const PINK: Color           = Color::rgb(0.90, 0.35, 0.55);     // hot pink
const GREEN: Color          = Color::rgb(0.30, 0.80, 0.50);     // mint green
const SURFACE: Color        = Color::rgba(0.10, 0.06, 0.18, 0.45);
const SURFACE_HOVER: Color  = Color::rgba(0.14, 0.08, 0.24, 0.55);
const WIDGET_BG: Color      = Color::rgba(0.06, 0.03, 0.12, 0.55);
const BORDER: Color         = Color::rgba(0.35, 0.25, 0.55, 0.25);

// ── Gallery state ──────────────────────────────────────────────────────────

pub struct GalleryState {
    pub toggle_on: bool,
    pub checkbox_checked: bool,
    pub slider_value: f32,
    pub radio_selected: u8,
    pub progress: f32,
    pub slider_dragging: bool,
    pub active_tab: usize,
    pub scroll_pos: f32, // 0.0 to 1.0
    pub scroll_dragging: bool,
}

impl GalleryState {
    pub fn new() -> Self {
        Self {
            toggle_on: false,
            checkbox_checked: false,
            slider_value: 0.4,
            radio_selected: 0,
            progress: 0.65,
            slider_dragging: false,
            active_tab: 0,
            scroll_pos: 0.3,
            scroll_dragging: false,
        }
    }
}

// ── Hit testing ────────────────────────────────────────────────────────────

fn in_rect(cx: f32, cy: f32, x: f32, y: f32, w: f32, h: f32) -> bool {
    cx >= x && cx <= x + w && cy >= y && cy <= y + h
}

// Layout constants
const TAB_NAMES: &[&str] = &["Widgets", "Colors", "Layout", "Settings"];

fn swatch_end_y(s: f32, title_h: f32) -> f32 {
    let swatch_y = title_h + 24.0 * s;
    let wy = 28.0 * s;
    let btn_h = 38.0 * s;
    let row1_y = swatch_y + wy;
    let row2_label_y = row1_y + btn_h + 16.0 * s;
    let row2_y = row2_label_y + wy;
    row2_y + btn_h + 14.0 * s
}

fn tabs_y(s: f32, title_h: f32) -> f32 { swatch_end_y(s, title_h) }
fn widgets_base_y(s: f32, title_h: f32) -> f32 { tabs_y(s, title_h) + 40.0 * s }

pub fn handle_click(
    cx: f32, cy: f32, s: f32, title_h: f32, wf: f32, hf: f32, gs: &mut GalleryState,
) -> bool {
    let col1 = 32.0 * s;
    let col2 = 520.0 * s;
    let base_y = widgets_base_y(s, title_h);
    let row_h = 90.0 * s;

    // Tabs
    let ty = tabs_y(s, title_h);
    let tab_h = 34.0 * s;
    let mut tx = col1;
    for i in 0..TAB_NAMES.len() {
        let tw = TAB_NAMES[i].len() as f32 * 10.0 * s + 24.0 * s;
        if in_rect(cx, cy, tx, ty, tw, tab_h) {
            gs.active_tab = i;
            return true;
        }
        tx += tw + 6.0 * s;
    }

    // Scrollbar (right edge)
    let sb_x = wf - 20.0 * s;
    let sb_w = 8.0 * s;
    let sb_top = base_y;
    let sb_h = hf - sb_top - 16.0 * s;
    if in_rect(cx, cy, sb_x - 6.0 * s, sb_top, sb_w + 12.0 * s, sb_h) {
        gs.scroll_pos = ((cy - sb_top) / sb_h).clamp(0.0, 1.0);
        gs.scroll_dragging = true;
        return true;
    }

    // Toggle (col2, row 0)
    let tog_y = base_y + 28.0 * s;
    if in_rect(cx, cy, col2, tog_y, 48.0 * s, 26.0 * s) {
        gs.toggle_on = !gs.toggle_on;
        return true;
    }

    // Checkbox (col2, row 1)
    let cb_y = base_y + row_h + 28.0 * s;
    if in_rect(cx, cy, col2, cb_y, 24.0 * s, 24.0 * s) {
        gs.checkbox_checked = !gs.checkbox_checked;
        return true;
    }

    // Radio buttons (col2, row 2)
    let radio_y = base_y + row_h * 2.0 + 28.0 * s;
    for i in 0..3u8 {
        let ry = radio_y + i as f32 * 34.0 * s;
        if in_rect(cx, cy, col2, ry - 12.0 * s, 200.0 * s, 28.0 * s) {
            gs.radio_selected = i;
            return true;
        }
    }

    // Slider (col1, row 2)
    let slider_y = base_y + row_h * 2.0 + 34.0 * s;
    let track_x = col1;
    let track_w = 340.0 * s;
    if in_rect(cx, cy, track_x - 12.0 * s, slider_y - 16.0 * s, track_w + 24.0 * s, 32.0 * s) {
        gs.slider_value = ((cx - track_x) / track_w).clamp(0.0, 1.0);
        gs.slider_dragging = true;
        return true;
    }

    false
}

pub fn handle_drag(cx: f32, cy: f32, s: f32, title_h: f32, hf: f32, gs: &mut GalleryState) {
    if gs.slider_dragging {
        let track_x = 32.0 * s;
        let track_w = 340.0 * s;
        gs.slider_value = ((cx - track_x) / track_w).clamp(0.0, 1.0);
    }
    if gs.scroll_dragging {
        let base_y = widgets_base_y(s, title_h);
        let sb_h = hf - base_y - 16.0 * s;
        gs.scroll_pos = ((cy - base_y) / sb_h).clamp(0.0, 1.0);
    }
}

pub fn handle_release(gs: &mut GalleryState) {
    gs.slider_dragging = false;
    gs.scroll_dragging = false;
}

// ── Drawing ────────────────────────────────────────────────────────────────

pub fn draw(
    p: &mut Painter, t: &mut TextRenderer,
    cx: f32, cy: f32, s: f32, title_h: f32,
    gs: &GalleryState, wf: f32, hf: f32, sw: u32, sh: u32,
) {
    let col1 = 32.0 * s;
    let col2 = 520.0 * s;
    let label_sz = 16.0 * s;
    let wy = 28.0 * s;
    let btn_h = 38.0 * s;
    let btn_r = 8.0 * s;

    // ── Accent color swatches (full width, 2 rows) ──────────────

    let swatch_y = title_h + 24.0 * s;
    if swatch_y > hf { return; }
    t.queue("Soft", label_sz, col1, swatch_y, TEXT_SECONDARY, wf, sw, sh);
    let row1_y = swatch_y + wy;
    let soft: &[(&str, Color)] = &[
        ("Cyan",      Color::rgb(0.25, 0.65, 0.90)),
        ("Pink",      Color::rgb(0.90, 0.35, 0.55)),
        ("Mint",      Color::rgb(0.30, 0.80, 0.50)),
        ("Peach",     Color::rgb(0.92, 0.55, 0.35)),
        ("Lavender",  Color::rgb(0.62, 0.48, 0.88)),
        ("Gold",      Color::rgb(0.85, 0.72, 0.25)),
        ("Rose",      Color::rgb(0.82, 0.42, 0.68)),
    ];
    let gap = 10.0 * s;
    let total_gap = gap * (soft.len() - 1) as f32;
    let btn_w = (wf - col1 * 2.0 - total_gap) / soft.len() as f32;
    for (i, (name, color)) in soft.iter().enumerate() {
        let bx = col1 + i as f32 * (btn_w + gap);
        let hov = in_rect(cx, cy, bx, row1_y, btn_w, btn_h);
        let c = if hov { color.lighten(0.15) } else { *color };
        p.rect_filled(Rect::new(bx, row1_y, btn_w, btn_h), btn_r, c);
        t.queue(name, 14.0 * s, bx + 8.0 * s, row1_y + 10.0 * s,
            Color::rgb(0.98, 0.98, 1.0), wf, sw, sh);
    }

    let row2_label_y = row1_y + btn_h + 16.0 * s;
    if row2_label_y > hf { return; }
    t.queue("Deep", label_sz, col1, row2_label_y, TEXT_SECONDARY, wf, sw, sh);
    let row2_y = row2_label_y + wy;
    let deep: [(&str, Color); 7] = [
        ("Navy",      Color::from_rgb8(20, 40, 120)),
        ("Blood",     Color::from_rgb8(140, 15, 15)),
        ("Forest",    Color::from_rgb8(15, 120, 30)),
        ("Ember",     Color::from_rgb8(160, 60, 10)),
        ("Plum",      Color::from_rgb8(100, 20, 120)),
        ("Amber",     Color::from_rgb8(180, 140, 15)),
        ("Slate",     Color::from_rgb8(45, 55, 80)),
    ];
    for (i, (name, color)) in deep.iter().enumerate() {
        let bx = col1 + i as f32 * (btn_w + gap);
        let hov = in_rect(cx, cy, bx, row2_y, btn_w, btn_h);
        let c = if hov { color.lighten(0.15) } else { *color };
        p.rect_filled(Rect::new(bx, row2_y, btn_w, btn_h), btn_r, c);
        t.queue(name, 14.0 * s, bx + 6.0 * s, row2_y + 10.0 * s,
            Color::rgb(0.98, 0.98, 1.0), wf, sw, sh);
    }

    // ── Tabs ────────────────────────────────────────────────────────

    let tabs_y = row2_y + btn_h + 14.0 * s;
    if tabs_y > hf { return; }
    let tab_h = 34.0 * s;
    let mut tx = col1;
    for i in 0..TAB_NAMES.len() {
        let tw = TAB_NAMES[i].len() as f32 * 10.0 * s + 24.0 * s;
        let active = gs.active_tab == i;
        let hov = in_rect(cx, cy, tx, tabs_y, tw, tab_h);
        if active {
            p.rect_filled(Rect::new(tx, tabs_y, tw, tab_h), 8.0 * s, SURFACE);
            // Active underline
            p.rect_filled(
                Rect::new(tx + 8.0 * s, tabs_y + tab_h - 2.5 * s, tw - 16.0 * s, 2.5 * s),
                1.0 * s, ACCENT,
            );
        } else if hov {
            p.rect_filled(Rect::new(tx, tabs_y, tw, tab_h), 8.0 * s,
                WIDGET_BG.with_alpha(0.3));
        }
        let tc = if active { TEXT_PRIMARY } else { TEXT_SECONDARY };
        t.queue(TAB_NAMES[i], 15.0 * s, tx + 12.0 * s, tabs_y + 8.0 * s, tc, wf, sw, sh);
        tx += tw + 6.0 * s;
    }
    // Tab bar bottom line
    p.rect_filled(
        Rect::new(col1, tabs_y + tab_h, wf - col1 * 2.0, 1.0 * s), 0.0,
        BORDER.with_alpha(0.15),
    );

    let base_y = tabs_y + tab_h + 12.0 * s;
    let row_h = 90.0 * s;

    // Row 1: Text Input
    let r1 = base_y + row_h;
    if r1 > hf { return; }
    t.queue("Text Input", label_sz, col1, r1, TEXT_SECONDARY, wf, sw, sh);
    let inp_y = r1 + wy;
    let inp_w = 340.0 * s;
    let inp_h = 38.0 * s;
    p.rect_filled(Rect::new(col1, inp_y, inp_w, inp_h), 6.0 * s, WIDGET_BG);
    p.rect_stroke_sdf(Rect::new(col1, inp_y, inp_w, inp_h), 6.0 * s, 1.0 * s, BORDER);
    t.queue("Text Input", 16.0 * s, col1 + 14.0 * s, inp_y + 9.0 * s,
        TEXT_SECONDARY, wf, sw, sh);

    // Row 2: Slider
    let r2 = base_y + row_h * 2.0;
    if r2 > hf { return; }
    t.queue("Slider", label_sz, col1, r2, TEXT_SECONDARY, wf, sw, sh);
    let sl_y = r2 + 34.0 * s;
    let track_w = 340.0 * s;
    let track_h = 5.0 * s;
    p.rect_filled(Rect::new(col1, sl_y, track_w, track_h), 3.0 * s, WIDGET_BG);
    let fill_w = track_w * gs.slider_value;
    p.rect_filled(Rect::new(col1, sl_y, fill_w, track_h), 3.0 * s, ACCENT);
    let tx = col1 + fill_w;
    let ty = sl_y + track_h * 0.5;
    p.circle_filled(tx, ty, 9.0 * s, ACCENT);
    p.circle_filled(tx, ty, 5.0 * s, Color::rgb(0.95, 0.95, 1.0));
    let val_str = format!("{:.0}%", gs.slider_value * 100.0);
    t.queue(&val_str, 14.0 * s, col1 + track_w + 14.0 * s, r2 + 28.0 * s,
        TEXT_SECONDARY, wf, sw, sh);

    // Row 3: Progress Bar
    let r3 = base_y + row_h * 3.0;
    if r3 > hf { return; }
    t.queue("Progress", label_sz, col1, r3, TEXT_SECONDARY, wf, sw, sh);
    let pg_y = r3 + 32.0 * s;
    let pg_h = 8.0 * s;
    p.rect_filled(Rect::new(col1, pg_y, track_w, pg_h), 4.0 * s, WIDGET_BG);
    p.rect_gradient_linear(
        Rect::new(col1, pg_y, track_w * gs.progress, pg_h), 4.0 * s,
        0.0, ACCENT, GREEN,
    );

    // ── Column 2 ────────────────────────────────────────────────────

    // Row 0: Toggle
    t.queue("Toggle", label_sz, col2, base_y, TEXT_SECONDARY, wf, sw, sh);
    let tog_y = base_y + wy;
    let tog_w = 48.0 * s;
    let tog_h = 26.0 * s;
    let tog_r = tog_h * 0.5;
    let tsz = 20.0 * s;
    let tog_bg = if gs.toggle_on { GREEN } else { WIDGET_BG };
    p.rect_filled(Rect::new(col2, tog_y, tog_w, tog_h), tog_r, tog_bg);
    if !gs.toggle_on {
        p.rect_stroke_sdf(Rect::new(col2, tog_y, tog_w, tog_h), tog_r, 1.0 * s, BORDER);
    }
    let toff = (tog_h - tsz) * 0.5;
    let tx = if gs.toggle_on { col2 + tog_w - tsz - toff } else { col2 + toff };
    p.rect_filled(Rect::new(tx, tog_y + toff, tsz, tsz), tsz * 0.5,
        Color::rgb(0.95, 0.95, 1.0));
    t.queue(if gs.toggle_on { "On" } else { "Off" }, 16.0 * s,
        col2 + tog_w + 14.0 * s, tog_y + 3.0 * s, TEXT_PRIMARY, wf, sw, sh);

    // Row 1: Checkbox
    let r1 = base_y + row_h;
    t.queue("Checkbox", label_sz, col2, r1, TEXT_SECONDARY, wf, sw, sh);
    let cb_y = r1 + wy;
    let cb = 24.0 * s;
    if gs.checkbox_checked {
        p.rect_filled(Rect::new(col2, cb_y, cb, cb), 5.0 * s, ACCENT);
        let x1 = col2 + 5.0 * s; let y1 = cb_y + 12.0 * s;
        let x2 = col2 + 10.0 * s; let y2 = cb_y + 18.0 * s;
        let x3 = col2 + 19.0 * s; let y3 = cb_y + 6.0 * s;
        p.line(x1, y1, x2, y2, 2.5 * s, Color::WHITE);
        p.line(x2, y2, x3, y3, 2.5 * s, Color::WHITE);
    } else {
        p.rect_filled(Rect::new(col2, cb_y, cb, cb), 5.0 * s, WIDGET_BG);
        p.rect_stroke_sdf(Rect::new(col2, cb_y, cb, cb), 5.0 * s, 1.0 * s, BORDER);
    }
    t.queue("Checkbox", 16.0 * s, col2 + cb + 12.0 * s, cb_y + 2.0 * s,
        TEXT_PRIMARY, wf, sw, sh);

    // Row 2: Radio Buttons
    let r2 = base_y + row_h * 2.0;
    t.queue("Radio", label_sz, col2, r2, TEXT_SECONDARY, wf, sw, sh);
    let ry0 = r2 + wy;
    let labels = ["Radio A", "Radio B", "Radio C"];
    for i in 0..3u8 {
        let ry = ry0 + i as f32 * 34.0 * s;
        let sel = gs.radio_selected == i;
        let rcx = col2 + 11.0 * s;
        let rcy = ry + 1.0 * s;
        p.circle_stroke(rcx, rcy, 10.0 * s, 1.5 * s,
            if sel { ACCENT } else { BORDER.with_alpha(0.5) });
        if sel { p.circle_filled(rcx, rcy, 5.0 * s, ACCENT); }
        t.queue(labels[i as usize], 16.0 * s, col2 + 28.0 * s, ry - 7.0 * s,
            TEXT_PRIMARY, wf, sw, sh);
    }

    // Row 3: Badges (below radio, with proper spacing)
    let r3 = base_y + row_h * 3.0 + 20.0 * s;
    t.queue("Badge", label_sz, col2, r3, TEXT_SECONDARY, wf, sw, sh);
    let by = r3 + wy;
    let bh = 28.0 * s;
    let br = bh * 0.5;
    let badges: &[(&str, Color)] = &[
        ("New", ACCENT),
        ("Beta", PINK),
        ("v1.0", SURFACE),
    ];
    let mut bx = col2;
    for (label, color) in badges {
        let bw = label.len() as f32 * 10.0 * s + 24.0 * s;
        p.rect_filled(Rect::new(bx, by, bw, bh), br, *color);
        t.queue(label, 14.0 * s, bx + 12.0 * s, by + 5.0 * s,
            Color::rgb(0.95, 0.95, 1.0), wf, sw, sh);
        bx += bw + 10.0 * s;
    }

    // ── Scrollbar (right edge) ──────────────────────────────────────
    let hf = sh as f32; // physical height
    let sb_x = wf - 20.0 * s;
    let sb_w = 6.0 * s;
    let sb_top = base_y;
    let sb_h = hf - sb_top - 16.0 * s;
    let thumb_h = 60.0 * s;
    let thumb_y = sb_top + (sb_h - thumb_h) * gs.scroll_pos;

    // Track
    p.rect_filled(Rect::new(sb_x, sb_top, sb_w, sb_h), 3.0 * s, WIDGET_BG);
    // Thumb
    let thumb_hov = in_rect(cx, cy, sb_x - 4.0 * s, thumb_y, sb_w + 8.0 * s, thumb_h);
    let tc = if gs.scroll_dragging || thumb_hov {
        TEXT_SECONDARY
    } else {
        BORDER.with_alpha(0.5)
    };
    p.rect_filled(Rect::new(sb_x, thumb_y, sb_w, thumb_h), 3.0 * s, tc);
}
