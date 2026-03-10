use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Badge, BadgeVariant, Button, ButtonVariant, Checkbox, Dropdown, Fill, FontSize, FoxPalette,
    InteractionContext, Panel, ProgressBar, RadioButton, ScrollArea, Scrollbar, Slider, TextInput,
    TextLabel, Toggle,
};

use crate::layout::*;

// ── Card background ─────────────────────────────────────────────────────────

fn card(
    p: &mut Painter, r: Rect, title: &str,
    t: &mut TextRenderer, f: &FoxPalette, sw: u32, sh: u32,
) {
    Panel::new(r)
        .fill(Fill::vertical(f.surface_2, f.surface))
        .radius(CARD_RADIUS)
        .draw(p);
    p.rect_stroke(r, CARD_RADIUS, 1.0, f.muted.with_alpha(0.2));
    TextLabel::new(title, r.x + CARD_PAD, r.y + 12.0)
        .size(FontSize::Caption).color(f.accent)
        .draw(t, sw, sh);
}

fn visible(y: f32, h: f32, vt: f32, vb: f32) -> bool {
    y + h > vt && y < vb
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 1 left: Buttons
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_BUTTONS_H: f32 = 180.0;

pub fn draw_buttons(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32, sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_BUTTONS_H, vt, vb) { return CARD_BUTTONS_H; }
    let r = Rect::new(x, y, w, CARD_BUTTONS_H);
    card(p, r, "Buttons", t, f, sw, sh);

    let pad = CARD_PAD;
    let btn_h = 38.0;
    let gap = 12.0;
    let btn_w = ((w - pad * 2.0 - gap) * 0.5).min(160.0);

    let variants = [
        (Z_BTN_DEFAULT, "Default", ButtonVariant::Default),
        (Z_BTN_PRIMARY, "Primary", ButtonVariant::Primary),
        (Z_BTN_GHOST, "Ghost", ButtonVariant::Ghost),
        (Z_BTN_DANGER, "Danger", ButtonVariant::Default),
    ];

    for (i, (zid, label, variant)) in variants.iter().enumerate() {
        let col = i % 2;
        let row = i / 2;
        let bx = x + pad + col as f32 * (btn_w + gap);
        let by = y + 44.0 + row as f32 * (btn_h + gap);
        let br = Rect::new(bx, by, btn_w, btn_h);
        let st = ix.add_zone(*zid, br);
        let mut btn = Button::new(br, label).variant(*variant)
            .hovered(st.is_hovered()).pressed(st.is_active());
        // Danger button: use default variant but we'll note it visually
        if *zid == Z_BTN_DANGER {
            btn = Button::new(br, label).variant(ButtonVariant::Default)
                .hovered(st.is_hovered()).pressed(st.is_active());
        }
        btn.draw(p, t, f, sw, sh);
    }
    CARD_BUTTONS_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 1 right: Controls (Toggle, Checkbox, Radio)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_CONTROLS_H: f32 = 180.0;

pub fn draw_controls(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    toggles: [bool; 2], checkboxes: [bool; 2], radio_sel: u32,
    sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_CONTROLS_H, vt, vb) { return CARD_CONTROLS_H; }
    let r = Rect::new(x, y, w, CARD_CONTROLS_H);
    card(p, r, "Controls", t, f, sw, sh);

    let pad = CARD_PAD;
    let row_h = 36.0;
    let half_w = (w - pad * 3.0) * 0.5;
    let cy = y + 44.0;

    // Toggles — left column
    let t1r = Rect::new(x + pad, cy, half_w, row_h);
    let t1s = ix.add_zone(Z_TOGGLE_A, t1r);
    Toggle::new(t1r, toggles[0]).label("Dark Mode")
        .hovered(t1s.is_hovered()).draw(p, t, f, sw, sh);

    let t2r = Rect::new(x + pad, cy + row_h + 8.0, half_w, row_h);
    let t2s = ix.add_zone(Z_TOGGLE_B, t2r);
    Toggle::new(t2r, toggles[1]).label("Animations")
        .hovered(t2s.is_hovered()).draw(p, t, f, sw, sh);

    // Checkboxes — right column
    let cx2 = x + pad + half_w + pad;
    let c1r = Rect::new(cx2, cy, half_w, row_h);
    let c1s = ix.add_zone(Z_CHECKBOX_A, c1r);
    Checkbox::new(c1r, checkboxes[0]).label("Show Grid")
        .hovered(c1s.is_hovered()).draw(p, t, f, sw, sh);

    let c2r = Rect::new(cx2, cy + row_h + 8.0, half_w, row_h);
    let c2s = ix.add_zone(Z_CHECKBOX_B, c2r);
    Checkbox::new(c2r, checkboxes[1]).label("Snap to Grid")
        .hovered(c2s.is_hovered()).draw(p, t, f, sw, sh);

    // Radio buttons — full width row
    let ry = cy + (row_h + 8.0) * 2.0;
    let labels = ["Small", "Medium", "Large"];
    let zones = [Z_RADIO_A, Z_RADIO_B, Z_RADIO_C];
    let radio_w = (w - pad * 2.0) / 3.0;
    for (i, (label, zid)) in labels.iter().zip(zones.iter()).enumerate() {
        let rr = Rect::new(x + pad + i as f32 * radio_w, ry, radio_w, row_h);
        let rs = ix.add_zone(*zid, rr);
        RadioButton::new(rr, radio_sel == i as u32)
            .label(label).hovered(rs.is_hovered())
            .draw(p, t, f, sw, sh);
    }

    CARD_CONTROLS_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 2 left: Text Input
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_INPUT_H: f32 = 120.0;

pub fn draw_text_input(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    text_value: &str, focused: bool,
    sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_INPUT_H, vt, vb) { return CARD_INPUT_H; }
    let r = Rect::new(x, y, w, CARD_INPUT_H);
    card(p, r, "Text Input", t, f, sw, sh);

    let pad = CARD_PAD;
    let input_w = w - pad * 2.0;
    let ir = Rect::new(x + pad, y + 48.0, input_w, 44.0);
    let is = ix.add_zone(Z_INPUT, ir);
    TextInput::new(ir).text(text_value).placeholder("Type here...")
        .focused(focused).hovered(is.is_hovered())
        .draw(p, t, f, sw, sh);

    CARD_INPUT_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 2 right: Dropdown
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_DROPDOWN_H: f32 = 120.0;
const DROPDOWN_OPTIONS: &[&str] = &["Fox Dark", "Fox Light", "Midnight", "Solarized"];

pub fn draw_dropdown(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    dd_open: bool, dd_selected: usize,
    sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_DROPDOWN_H, vt, vb) { return CARD_DROPDOWN_H; }
    let r = Rect::new(x, y, w, CARD_DROPDOWN_H);
    card(p, r, "Dropdown", t, f, sw, sh);

    let pad = CARD_PAD;
    let dd_w = (w - pad * 2.0).min(280.0);
    let dd_rect = Rect::new(x + pad, y + 48.0, dd_w, 42.0);
    let dd_state = ix.add_zone(Z_DROPDOWN, dd_rect);

    let temp = Dropdown::new(dd_rect, DROPDOWN_OPTIONS, dd_selected).open(dd_open);
    let mut hovered_opt = None;
    if dd_open {
        for i in 0..DROPDOWN_OPTIONS.len() {
            let or = temp.option_rect(i);
            let os = ix.add_zone(Z_DROPDOWN_OPT + i as u32, or);
            if os.is_hovered() { hovered_opt = Some(i); }
        }
    }
    Dropdown::new(dd_rect, DROPDOWN_OPTIONS, dd_selected)
        .open(dd_open).button_hovered(dd_state.is_hovered())
        .hovered_option(hovered_opt)
        .draw(p, t, f, sw, sh);

    CARD_DROPDOWN_H
}

pub fn dropdown_option_count() -> usize {
    DROPDOWN_OPTIONS.len()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 3 left: Slider & Progress
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_SLIDER_H: f32 = 160.0;

pub fn draw_slider_progress(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    slider_val: f32, anim_time: f32,
    sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_SLIDER_H, vt, vb) { return CARD_SLIDER_H; }
    let r = Rect::new(x, y, w, CARD_SLIDER_H);
    card(p, r, "Slider & Progress", t, f, sw, sh);

    let pad = CARD_PAD;
    let track_w = w - pad * 2.0;

    // Slider
    let sy = y + 48.0;
    let sr = Rect::new(x + pad, sy, track_w, 28.0);
    let ss = ix.add_zone(Z_SLIDER, sr);
    Slider::new(sr).value(slider_val)
        .hovered(ss.is_hovered()).active(ss.is_active())
        .draw(p, f);
    let pct = format!("{:.0}%", slider_val * 100.0);
    TextLabel::new(&pct, x + pad + track_w + 8.0, sy + 4.0)
        .size(FontSize::Label).color(f.accent).draw(t, sw, sh);

    // Animated progress bar
    let py = sy + 48.0;
    TextLabel::new("Loading", x + pad, py - 2.0)
        .size(FontSize::Label).color(f.text_secondary).draw(t, sw, sh);
    let prog = (anim_time % 4.0) / 4.0;
    ProgressBar::new(Rect::new(x + pad, py + 22.0, track_w, 24.0))
        .value(prog).label(true)
        .draw(p, t, f, sw, sh);

    CARD_SLIDER_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 3 right: Color Swatches
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_SWATCHES_H: f32 = 200.0;

pub fn draw_swatches(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    x: f32, y: f32, w: f32, sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_SWATCHES_H, vt, vb) { return CARD_SWATCHES_H; }
    let r = Rect::new(x, y, w, CARD_SWATCHES_H);
    card(p, r, "Palette", t, f, sw, sh);

    let pad = CARD_PAD;
    let colors: &[(&str, Color)] = &[
        ("bg", f.bg), ("surface", f.surface), ("surf 2", f.surface_2),
        ("accent", f.accent), ("text", f.text), ("muted", f.muted),
        ("danger", f.danger), ("success", f.success),
        ("warn", f.warning), ("info", f.info),
    ];

    let cols = 5;
    let avail = w - pad * 2.0;
    let gap = 8.0;
    let swatch_w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
    let swatch_h = 36.0;
    let start_y = y + 44.0;

    for (i, (label, color)) in colors.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let sx = x + pad + col as f32 * (swatch_w + gap);
        let sy = start_y + row as f32 * (swatch_h + 26.0);

        let sr = Rect::new(sx, sy, swatch_w, swatch_h);
        p.rect_filled(sr, 6.0, *color);
        p.rect_stroke(sr, 6.0, 1.0, f.text.with_alpha(0.12));

        TextLabel::new(label, sx + 2.0, sy + swatch_h + 4.0)
            .size(FontSize::Label).color(f.muted)
            .max_width(swatch_w)
            .draw(t, sw, sh);
    }

    CARD_SWATCHES_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 4 left: Badges
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_BADGES_H: f32 = 120.0;

pub fn draw_badges(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    x: f32, y: f32, w: f32, sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_BADGES_H, vt, vb) { return CARD_BADGES_H; }
    let r = Rect::new(x, y, w, CARD_BADGES_H);
    card(p, r, "Badges", t, f, sw, sh);

    let pad = CARD_PAD;
    let badges = [
        ("NEW", BadgeVariant::Info),
        ("STABLE", BadgeVariant::Success),
        ("WARN", BadgeVariant::Warning),
        ("ERROR", BadgeVariant::Danger),
        ("TAG", BadgeVariant::Default),
    ];

    // Row 1
    let mut bx = x + pad;
    let by = y + 48.0;
    for (label, variant) in &badges {
        Badge::new(label, bx, by).variant(*variant).draw(p, t, f, sw, sh);
        bx += label.len() as f32 * 12.0 + 32.0;
    }

    // Row 2 — larger labels
    let by2 = by + 36.0;
    let mut bx = x + pad;
    let big_badges = [
        ("v2.1.0", BadgeVariant::Info),
        ("Production", BadgeVariant::Success),
        ("Deprecated", BadgeVariant::Danger),
    ];
    for (label, variant) in &big_badges {
        Badge::new(label, bx, by2).variant(*variant).draw(p, t, f, sw, sh);
        bx += label.len() as f32 * 12.0 + 32.0;
    }

    CARD_BADGES_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 4 right: Scroll Area
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_SCROLL_H: f32 = 220.0;
const SCROLL_ITEMS: &[&str] = &[
    "Display", "Network", "Audio", "Keyboard", "Touchpad",
    "Power", "Notifications", "Apps", "Accessibility",
    "About", "Accounts", "Privacy", "Date & Time", "Updates",
];
const SCROLL_ITEM_H: f32 = 36.0;

pub fn draw_scroll_demo(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32, scroll_offset: &mut f32,
    sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_SCROLL_H, vt, vb) { return CARD_SCROLL_H; }
    let r = Rect::new(x, y, w, CARD_SCROLL_H);
    card(p, r, "Scroll Area", t, f, sw, sh);

    let pad = CARD_PAD;
    let vp = Rect::new(x + pad, y + 44.0, w - pad * 2.0, CARD_SCROLL_H - 54.0);
    let content_h = SCROLL_ITEMS.len() as f32 * SCROLL_ITEM_H;

    if ix.is_hovered(&vp) {
        let delta = ix.scroll_delta() * 40.0;
        if delta != 0.0 {
            ScrollArea::apply_scroll(scroll_offset, delta, content_h, vp.h);
        }
    }

    let area = ScrollArea::new(vp, content_h, scroll_offset);
    let scrollbar = Scrollbar::new(&vp, content_h, *scroll_offset);
    let sb_state = ix.add_zone(Z_SCROLL_DEMO, scrollbar.thumb);
    if sb_state.is_active() {
        if let Some((_, sy)) = ix.cursor() {
            *scroll_offset = scrollbar.offset_for_thumb_y(sy, content_h, vp.h);
        }
    }

    let stripe_colors = [f.accent, f.danger, f.success, f.info];
    area.begin(p);
    let cy = area.content_y();
    for (i, label) in SCROLL_ITEMS.iter().enumerate() {
        let iy = cy + i as f32 * SCROLL_ITEM_H;
        if iy + SCROLL_ITEM_H < vp.y || iy > vp.y + vp.h { continue; }
        let ir = Rect::new(vp.x + 2.0, iy + 2.0, vp.w - 16.0, SCROLL_ITEM_H - 4.0);
        p.rect_filled(ir, 6.0, f.surface_2.with_alpha(0.4));
        let stripe = Rect::new(ir.x, ir.y + 4.0, 3.0, ir.h - 8.0);
        p.rect_filled(stripe, 1.5, stripe_colors[i % stripe_colors.len()].with_alpha(0.6));
        TextLabel::new(label, ir.x + 12.0, ir.y + 7.0)
            .size(FontSize::Label).color(f.text).max_width(ir.w - 24.0)
            .draw(t, sw, sh);
    }
    area.end(p);
    scrollbar.draw(p, sb_state, f);

    CARD_SCROLL_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Row 5: Modal trigger (full width)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const CARD_MODAL_H: f32 = 100.0;

pub fn draw_modal_trigger(
    p: &mut Painter, t: &mut TextRenderer, f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32, sw: u32, sh: u32, vt: f32, vb: f32,
) -> f32 {
    if !visible(y, CARD_MODAL_H, vt, vb) { return CARD_MODAL_H; }
    let r = Rect::new(x, y, w, CARD_MODAL_H);
    card(p, r, "Modal", t, f, sw, sh);

    let pad = CARD_PAD;
    let mr = Rect::new(x + pad, y + 48.0, 200.0, 38.0);
    let ms = ix.add_zone(Z_MODAL_OPEN, mr);
    Button::new(mr, "Open Modal").variant(ButtonVariant::Primary)
        .hovered(ms.is_hovered()).pressed(ms.is_active())
        .draw(p, t, f, sw, sh);

    CARD_MODAL_H
}
