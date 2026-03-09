use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Badge, BadgeVariant, Button, ButtonVariant, Checkbox, Dropdown, Fill, FontSize, FoxPalette,
    InteractionContext, Panel, ProgressBar, RadioGroup, ScrollArea, Scrollbar, Slider, TextInput,
    TextLabel, Toggle, Tooltip,
};

use crate::layout::*;

// ── Section background helper ───────────────────────────────────────────────

fn sec(
    p: &mut Painter,
    r: Rect,
    title: &str,
    t: &mut TextRenderer,
    f: &FoxPalette,
    sw: u32,
    sh: u32,
) {
    let border = f.muted.with_alpha(0.3);
    Panel::new(r)
        .fill(Fill::vertical(f.surface_2, f.surface))
        .radius(SUB_RADIUS)
        .draw(p);
    p.rect_stroke(r, SUB_RADIUS, 1.0, border);
    TextLabel::new(title, r.x + SECTION_PAD, r.y + 14.0)
        .size(FontSize::Small)
        .color(f.text)
        .draw(t, sw, sh);
}

/// Returns true if section rect is at least partially on screen.
fn visible(y: f32, h: f32, vp_top: f32, vp_bot: f32) -> bool {
    y + h > vp_top && y < vp_bot
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 1. Global Controls — transparency + text size sliders
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_GLOBAL_H: f32 = 170.0;

pub fn draw_global_controls(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    bg_alpha: f32, font_scale: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_GLOBAL_H, vt, vb) { return SEC_GLOBAL_H; }
    let r = Rect::new(x, y, w, SEC_GLOBAL_H);
    sec(p, r, "Global Controls", t, f, sw, sh);

    let pad = SECTION_PAD;
    let slider_w = (w - pad * 2.0) * 0.45;
    let slider_h = 28.0;

    // Transparency slider
    let ty = y + 48.0;
    TextLabel::new("Transparency", x + pad, ty)
        .size(FontSize::Label).color(f.text_secondary).draw(t, sw, sh);
    let sr = Rect::new(x + pad, ty + 26.0, slider_w, slider_h);
    let ss = ix.add_zone(Z_TRANSPARENCY, sr);
    Slider::new(sr).value(bg_alpha).hovered(ss.is_hovered()).active(ss.is_active()).draw(p, f);
    let pct = format!("{:.0}%", bg_alpha * 100.0);
    TextLabel::new(&pct, sr.x + sr.w + 14.0, ty + 26.0)
        .size(FontSize::Caption).color(f.accent).draw(t, sw, sh);

    // Text size slider
    let tx2 = x + pad + slider_w + 40.0;
    TextLabel::new("Text Scale", tx2, ty)
        .size(FontSize::Label).color(f.text_secondary).draw(t, sw, sh);
    let sr2 = Rect::new(tx2, ty + 26.0, slider_w, slider_h);
    let ss2 = ix.add_zone(Z_TEXT_SIZE, sr2);
    Slider::new(sr2).value((font_scale - 0.5) / 1.5).hovered(ss2.is_hovered()).active(ss2.is_active()).draw(p, f);
    let scale_text = format!("{:.1}x", font_scale);
    TextLabel::new(&scale_text, sr2.x + sr2.w + 14.0, ty + 26.0)
        .size(FontSize::Caption).color(f.accent).draw(t, sw, sh);

    // Demo text
    let demo_y = ty + 66.0;
    let demo_size = 24.0 * font_scale;
    TextLabel::new("The quick brown fox jumps over the lazy dog", x + pad, demo_y)
        .size(FontSize::Custom(demo_size))
        .color(f.text)
        .max_width(w - pad * 2.0)
        .draw(t, sw, sh);

    SEC_GLOBAL_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 2. Typography — font sizes + color palette
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_TYPO_H: f32 = 340.0;

pub fn draw_typography(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    x: f32, y: f32, w: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_TYPO_H, vt, vb) { return SEC_TYPO_H; }
    let r = Rect::new(x, y, w, SEC_TYPO_H);
    sec(p, r, "Typography", t, f, sw, sh);

    let pad = SECTION_PAD;
    let col_w = (w - pad * 3.0) * 0.5;

    // Left: font size samples
    let samples: &[(&str, FontSize)] = &[
        ("Heading 38px", FontSize::Heading),
        ("Subheading 32px", FontSize::Subheading),
        ("Body 28px", FontSize::Body),
        ("Small 26px", FontSize::Small),
        ("Caption 24px", FontSize::Caption),
        ("Label 22px", FontSize::Label),
    ];
    let mut sy = y + 48.0;
    for (label, size) in samples {
        TextLabel::new(label, x + pad, sy)
            .size(*size).color(f.text).max_width(col_w)
            .draw(t, sw, sh);
        sy += size.px() + 12.0;
    }

    // Right: color swatches
    let cx = x + pad + col_w + pad;
    let colors: &[(&str, Color)] = &[
        ("text", f.text), ("text_secondary", f.text_secondary),
        ("muted", f.muted), ("accent", f.accent),
        ("danger", f.danger), ("success", f.success),
        ("warning", f.warning), ("info", f.info),
        ("surface", f.surface), ("bg", f.bg),
    ];
    let swatch_w = 36.0;
    let row_h = 28.0;
    let mut sy = y + 48.0;
    for (label, color) in colors {
        let sw_r = Rect::new(cx, sy, swatch_w, row_h);
        p.rect_filled(sw_r, 6.0, *color);
        p.rect_stroke(sw_r, 6.0, 1.0, f.text.with_alpha(0.15));
        TextLabel::new(label, cx + swatch_w + 10.0, sy + 4.0)
            .size(FontSize::Label).color(f.text_secondary)
            .draw(t, sw, sh);
        sy += row_h + 2.0;
    }

    SEC_TYPO_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 3. Buttons
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_BUTTONS_H: f32 = 110.0;

pub fn draw_buttons(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_BUTTONS_H, vt, vb) { return SEC_BUTTONS_H; }
    sec(p, Rect::new(x, y, w, SEC_BUTTONS_H), "Buttons", t, f, sw, sh);

    let pad = SECTION_PAD;
    let btn_y = y + 52.0;
    let btn_w = 140.0;
    let btn_h = 38.0;
    let gap = 18.0;

    let variants = [
        (Z_BTN_DEFAULT, "Default", ButtonVariant::Default),
        (Z_BTN_PRIMARY, "Primary", ButtonVariant::Primary),
        (Z_BTN_GHOST, "Ghost", ButtonVariant::Ghost),
    ];
    for (i, (zid, label, variant)) in variants.iter().enumerate() {
        let bx = x + pad + i as f32 * (btn_w + gap);
        let br = Rect::new(bx, btn_y, btn_w, btn_h);
        let st = ix.add_zone(*zid, br);
        Button::new(br, label).variant(*variant)
            .hovered(st.is_hovered()).pressed(st.is_active())
            .draw(p, t, f, sw, sh);
    }
    SEC_BUTTONS_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 4. Slider
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_SLIDER_H: f32 = 100.0;

pub fn draw_slider(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    slider_value: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_SLIDER_H, vt, vb) { return SEC_SLIDER_H; }
    sec(p, Rect::new(x, y, w, SEC_SLIDER_H), "Slider", t, f, sw, sh);

    let pad = SECTION_PAD;
    let sr = Rect::new(x + pad, y + 52.0, (w - pad * 2.0) * 0.6, 28.0);
    let ss = ix.add_zone(Z_SLIDER, sr);
    Slider::new(sr).value(slider_value).hovered(ss.is_hovered()).active(ss.is_active()).draw(p, f);

    let pct = format!("{:.0}%", slider_value * 100.0);
    TextLabel::new(&pct, sr.x + sr.w + 14.0, y + 52.0)
        .size(FontSize::Caption).color(f.accent).draw(t, sw, sh);
    SEC_SLIDER_H
}

/// Compute slider value from cursor X.
pub fn slider_value_for_x(x: f32, sec_x: f32, sec_w: f32) -> f32 {
    let pad = SECTION_PAD;
    let sr_x = sec_x + pad;
    let sr_w = (sec_w - pad * 2.0) * 0.6;
    ((x - sr_x) / sr_w.max(1.0)).clamp(0.0, 1.0)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 5. Selection Controls — Checkboxes, Toggles, Radios
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_SELECTION_H: f32 = 240.0;

pub fn draw_selection(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    checkbox_states: [bool; 3],
    toggle_states: [bool; 2],
    radio_selected: usize,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_SELECTION_H, vt, vb) { return SEC_SELECTION_H; }
    sec(p, Rect::new(x, y, w, SEC_SELECTION_H), "Selection Controls", t, f, sw, sh);

    let pad = SECTION_PAD;
    let col_w = (w - pad * 4.0) / 3.0;
    let top = y + 48.0;
    let row_h = 48.0;

    // Column 1: Checkboxes
    TextLabel::new("Checkboxes", x + pad, top)
        .size(FontSize::Label).color(f.muted).draw(t, sw, sh);
    let cbs = [
        (Z_CB_BASE, "Enable feature", checkbox_states[0], false),
        (Z_CB_BASE + 1, "Dark mode", checkbox_states[1], false),
        (Z_CB_BASE + 2, "Disabled", checkbox_states[2], true),
    ];
    for (i, (zid, label, checked, disabled)) in cbs.iter().enumerate() {
        let cr = Rect::new(x + pad, top + 28.0 + i as f32 * row_h, col_w, row_h);
        let st = ix.add_zone(*zid, cr);
        Checkbox::new(cr, *checked).label(label)
            .hovered(st.is_hovered()).disabled(*disabled)
            .draw(p, t, f, sw, sh);
    }

    // Column 2: Toggles
    let tx = x + pad + col_w + pad;
    TextLabel::new("Toggles", tx, top)
        .size(FontSize::Label).color(f.muted).draw(t, sw, sh);
    let tgs = [
        (Z_TOGGLE_BASE, "Wi-Fi", toggle_states[0]),
        (Z_TOGGLE_BASE + 1, "Bluetooth", toggle_states[1]),
    ];
    for (i, (zid, label, on)) in tgs.iter().enumerate() {
        let tr = Rect::new(tx, top + 28.0 + i as f32 * row_h, col_w, row_h);
        let st = ix.add_zone(*zid, tr);
        Toggle::new(tr, *on).label(label)
            .hovered(st.is_hovered())
            .draw(p, t, f, sw, sh);
    }

    // Column 3: Radio buttons
    let rx = tx + col_w + pad;
    TextLabel::new("Radio Group", rx, top)
        .size(FontSize::Label).color(f.muted).draw(t, sw, sh);
    let options = ["Small", "Medium", "Large"];
    // Register zones
    let mut hovered_radio = None;
    let rg = RadioGroup::new(rx, top + 28.0, &options, radio_selected).width(col_w);
    for i in 0..options.len() {
        let rr = rg.item_rect(i);
        let st = ix.add_zone(Z_RADIO_BASE + i as u32, rr);
        if st.is_hovered() { hovered_radio = Some(i); }
    }
    RadioGroup::new(rx, top + 28.0, &options, radio_selected)
        .width(col_w)
        .hovered_index(hovered_radio)
        .draw(p, t, f, sw, sh);

    SEC_SELECTION_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 6. Text Input & Dropdown
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_INPUTS_H: f32 = 290.0;
const DROPDOWN_OPTIONS: &[&str] = &["Fox Dark", "Fox Light", "Midnight", "Solarized"];

pub fn draw_inputs(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    text_value: &str, focused: Option<u32>,
    dd_open: bool, dd_selected: usize,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_INPUTS_H, vt, vb) { return SEC_INPUTS_H; }
    sec(p, Rect::new(x, y, w, SEC_INPUTS_H), "Text Input & Dropdown", t, f, sw, sh);

    let pad = SECTION_PAD;
    let input_w = (w - pad * 2.0).min(400.0);
    let input_h = 44.0;
    let gap = 14.0;
    let ix0 = x + pad;

    // Input 1: empty placeholder
    let iy = y + 48.0;
    let r1 = Rect::new(ix0, iy, input_w, input_h);
    let s1 = ix.add_zone(Z_INPUT_BASE, r1);
    TextInput::new(r1).placeholder("Type something...")
        .focused(focused == Some(Z_INPUT_BASE)).hovered(s1.is_hovered())
        .draw(p, t, f, sw, sh);

    // Input 2: filled
    let r2 = Rect::new(ix0, iy + input_h + gap, input_w, input_h);
    let s2 = ix.add_zone(Z_INPUT_BASE + 1, r2);
    TextInput::new(r2).text(text_value).placeholder("Editable")
        .focused(focused == Some(Z_INPUT_BASE + 1)).hovered(s2.is_hovered())
        .draw(p, t, f, sw, sh);

    // Input 3: always focused
    let r3 = Rect::new(ix0, iy + (input_h + gap) * 2.0, input_w, input_h);
    let s3 = ix.add_zone(Z_INPUT_BASE + 2, r3);
    TextInput::new(r3).text("Always focused").focused(true).hovered(s3.is_hovered())
        .draw(p, t, f, sw, sh);

    // Dropdown
    let dd_y = iy + (input_h + gap) * 3.0;
    let dd_rect = Rect::new(ix0, dd_y, 250.0, 42.0);
    let dd_state = ix.add_zone(Z_DROPDOWN, dd_rect);

    // Probe option zones if open
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

    SEC_INPUTS_H
}

pub fn dropdown_option_count() -> usize {
    DROPDOWN_OPTIONS.len()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 7. Badges, Progress, Tooltip
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_BADGES_H: f32 = 200.0;

pub fn draw_badges_progress(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    x: f32, y: f32, w: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_BADGES_H, vt, vb) { return SEC_BADGES_H; }
    sec(p, Rect::new(x, y, w, SEC_BADGES_H), "Badge · Progress · Tooltip", t, f, sw, sh);

    let pad = SECTION_PAD;
    let by = y + 52.0;

    // Badge row
    let badges = [
        ("NEW", BadgeVariant::Info),
        ("STABLE", BadgeVariant::Success),
        ("WARN", BadgeVariant::Warning),
        ("ERROR", BadgeVariant::Danger),
        ("DEFAULT", BadgeVariant::Default),
    ];
    let mut bx = x + pad;
    for (label, variant) in badges {
        Badge::new(label, bx, by).variant(variant).draw(p, t, f, sw, sh);
        bx += label.len() as f32 * 14.0 * 0.55 + 34.0;
    }

    // Progress bar
    let py = by + 44.0;
    let bar_w = (w - pad * 2.0) * 0.7;
    ProgressBar::new(Rect::new(x + pad, py, bar_w, 28.0))
        .value(0.65).label(true)
        .draw(p, t, f, sw, sh);

    // Tooltip
    let ty = py + 48.0;
    TextLabel::new("Hover target →", x + pad, ty + 6.0)
        .size(FontSize::Label).color(f.muted).draw(t, sw, sh);
    Tooltip::new("I'm a tooltip!", x + pad + 180.0, ty)
        .draw(p, t, f, sw, sh);

    SEC_BADGES_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 8. Scroll Area Demo
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_SCROLL_H: f32 = 280.0;
const SCROLL_ITEMS: &[&str] = &[
    "Display Settings", "Network Config", "Audio Output",
    "Keyboard Layout", "Mouse & Touchpad", "Power Management",
    "Notifications", "Default Apps", "Accessibility",
    "About System", "User Accounts", "Privacy",
    "Date & Time", "Language & Region", "Software Updates",
];
const SCROLL_ITEM_H: f32 = 40.0;

pub fn draw_scroll_demo(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    scroll_offset: &mut f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_SCROLL_H, vt, vb) { return SEC_SCROLL_H; }
    sec(p, Rect::new(x, y, w, SEC_SCROLL_H), "Scroll Area", t, f, sw, sh);

    let pad = SECTION_PAD;
    let viewport = Rect::new(x + pad, y + 48.0, w - pad * 2.0, SEC_SCROLL_H - 58.0);
    let content_h = SCROLL_ITEMS.len() as f32 * SCROLL_ITEM_H;

    // Wheel scroll inside this area (check if cursor is inside)
    if ix.is_hovered(&viewport) {
        let delta = ix.scroll_delta() * 40.0;
        if delta != 0.0 {
            ScrollArea::apply_scroll(scroll_offset, delta, content_h, viewport.h);
        }
    }

    let area = ScrollArea::new(viewport, content_h, scroll_offset);
    let scrollbar = Scrollbar::new(&viewport, content_h, *scroll_offset);
    let sb_state = ix.add_zone(Z_SCROLL_DEMO, scrollbar.thumb);

    if sb_state.is_active() {
        if let Some((_, sy)) = ix.cursor() {
            *scroll_offset = scrollbar.offset_for_thumb_y(sy, content_h, viewport.h);
        }
    }

    let item_colors = [f.accent, f.danger, f.success, f.info];
    area.begin(p);
    let cy = area.content_y();
    for (i, label) in SCROLL_ITEMS.iter().enumerate() {
        let iy = cy + i as f32 * SCROLL_ITEM_H;
        if iy + SCROLL_ITEM_H < viewport.y || iy > viewport.y + viewport.h { continue; }
        let ir = Rect::new(viewport.x + 4.0, iy + 2.0, viewport.w - 20.0, SCROLL_ITEM_H - 4.0);
        p.rect_filled(ir, 8.0, f.surface_2.with_alpha(0.5));
        let stripe = Rect::new(ir.x, ir.y + 4.0, 3.0, ir.h - 8.0);
        p.rect_filled(stripe, 1.5, item_colors[i % item_colors.len()].with_alpha(0.7));
        TextLabel::new(label, ir.x + 14.0, ir.y + 8.0)
            .size(FontSize::Caption).color(f.text).max_width(ir.w - 28.0)
            .draw(t, sw, sh);
    }
    area.end(p);
    scrollbar.draw(p, sb_state, f);

    SEC_SCROLL_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 9. Modal & Toast Triggers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_ACTIONS_H: f32 = 110.0;

pub fn draw_actions(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    ix: &mut InteractionContext,
    x: f32, y: f32, w: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_ACTIONS_H, vt, vb) { return SEC_ACTIONS_H; }
    sec(p, Rect::new(x, y, w, SEC_ACTIONS_H), "Modal & Toast", t, f, sw, sh);

    let pad = SECTION_PAD;
    let btn_y = y + 52.0;
    let btn_w = 180.0;
    let btn_h = 38.0;

    let mr = Rect::new(x + pad, btn_y, btn_w, btn_h);
    let ms = ix.add_zone(Z_MODAL_OPEN, mr);
    Button::new(mr, "Open Modal").variant(ButtonVariant::Primary)
        .hovered(ms.is_hovered()).pressed(ms.is_active())
        .draw(p, t, f, sw, sh);

    let tr = Rect::new(x + pad + btn_w + 18.0, btn_y, btn_w, btn_h);
    let ts = ix.add_zone(Z_TOAST_SPAWN, tr);
    Button::new(tr, "Spawn Toast").variant(ButtonVariant::Default)
        .hovered(ts.is_hovered()).pressed(ts.is_active())
        .draw(p, t, f, sw, sh);

    SEC_ACTIONS_H
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 10. Animations — auto-looping demos
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub const SEC_ANIMS_H: f32 = 190.0;

pub fn draw_animations(
    p: &mut Painter,
    t: &mut TextRenderer,
    f: &FoxPalette,
    x: f32, y: f32, w: f32,
    anim_time: f32,
    sw: u32, sh: u32,
    vt: f32, vb: f32,
) -> f32 {
    if !visible(y, SEC_ANIMS_H, vt, vb) { return SEC_ANIMS_H; }
    sec(p, Rect::new(x, y, w, SEC_ANIMS_H), "Animations", t, f, sw, sh);

    let pad = SECTION_PAD;

    // Auto-toggling toggle
    let cycle = anim_time % 3.0;
    let toggle_on = cycle > 1.5;
    // Smooth transition over 0.3s
    let transition = if toggle_on {
        ((cycle - 1.5) / 0.3).clamp(0.0, 1.0)
    } else {
        1.0 - (cycle / 0.3).clamp(0.0, 1.0)
    };

    let ty = y + 52.0;
    Toggle::new(Rect::new(x + pad, ty, 200.0, 36.0), toggle_on)
        .label("Auto-toggle")
        .transition(transition)
        .draw(p, t, f, sw, sh);

    // Auto-filling progress bar
    let prog = (anim_time % 4.0) / 4.0;
    let py = ty + 50.0;
    TextLabel::new("Progress animation", x + pad, py)
        .size(FontSize::Label).color(f.muted).draw(t, sw, sh);
    ProgressBar::new(Rect::new(x + pad, py + 26.0, (w - pad * 2.0) * 0.6, 28.0))
        .value(prog).label(true)
        .draw(p, t, f, sw, sh);

    // Info text
    let iy = py + 66.0;
    TextLabel::new("Toasts auto-spawn every 5s — check bottom-right corner!", x + pad, iy)
        .size(FontSize::Label).color(f.text_secondary).max_width(w - pad * 2.0)
        .draw(t, sw, sh);

    SEC_ANIMS_H
}
