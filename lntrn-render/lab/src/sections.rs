use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, Checkbox, Fill, FontSize, FoxPalette, InteractionContext, Panel, Slider,
    TabBar, TextInput, TextLabel,
};

use crate::layout::*;

// ── Zone ID ranges ───────────────────────────────────────────────────────────
// 1-9:    title bar
// 10-19:  tab bar tabs
// 20-49:  controls tab (slider, checkboxes, buttons)
// 50-79:  inputs tab (text inputs)
// 80-99:  containers tab (scrollbar, etc.)
// 100+:   orb, swatches, etc.

pub const ZONE_BTN_DEFAULT: u32 = 20;
pub const ZONE_BTN_PRIMARY: u32 = 21;
pub const ZONE_BTN_GHOST: u32 = 22;
pub const ZONE_SLIDER: u32 = 23;
pub const ZONE_CB_ONE: u32 = 30;
pub const ZONE_CB_TWO: u32 = 31;
pub const ZONE_CB_THREE: u32 = 32;

pub const ZONE_INPUT_EMPTY: u32 = 50;
pub const ZONE_INPUT_FILLED: u32 = 51;
pub const ZONE_INPUT_FOCUSED: u32 = 52;


// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tab 1: Typography
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn draw_typography_tab(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    content: Rect,
    size: (u32, u32),
) {
    let border = fox.muted.with_alpha(0.3);

    // ── Left column: Text scale reference ────────────────────────────────
    let text_rect = typo_text_scale_rect(content);
    Panel::new(text_rect)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(text_rect, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Text Scale Reference", text_rect.x + 18.0, text_rect.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let samples: &[(&str, FontSize)] = &[
        ("Heading 38px", FontSize::Heading),
        ("Subheading 32px", FontSize::Subheading),
        ("Body 28px", FontSize::Body),
        ("Small 26px", FontSize::Small),
        ("Caption 24px", FontSize::Caption),
        ("Label 22px", FontSize::Label),
    ];

    let mut y = text_rect.y + 50.0;
    for (label, font_size) in samples {
        TextLabel::new(label, text_rect.x + 18.0, y)
            .size(*font_size)
            .color(fox.text)
            .max_width(text_rect.w - 36.0)
            .draw(text_renderer, size.0, size.1);
        y += font_size.px() + 16.0;
    }

    // ── Right column: Color swatches ─────────────────────────────────────
    let swatch_rect = typo_color_swatch_rect(content);
    Panel::new(swatch_rect)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(swatch_rect, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Palette Colors", swatch_rect.x + 18.0, swatch_rect.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let color_samples: &[(&str, Color)] = &[
        ("text", fox.text),
        ("text_secondary", fox.text_secondary),
        ("muted", fox.muted),
        ("accent", fox.accent),
        ("danger", fox.danger),
        ("success", fox.success),
        ("surface", fox.surface),
        ("surface_2", fox.surface_2),
        ("bg", fox.bg),
        ("sidebar", fox.sidebar),
    ];

    let swatch_w = 44.0;
    let swatch_h = 40.0;
    // Distribute rows evenly across available height
    let swatch_top = swatch_rect.y + 50.0;
    let swatch_avail = swatch_rect.h - 58.0; // 50 top + 8 bottom
    let row_h = (swatch_avail / color_samples.len() as f32).min(52.0);
    let mut y = swatch_top;

    for (label, color) in color_samples {
        if y + swatch_h > swatch_rect.y + swatch_rect.h - 4.0 {
            break;
        }

        // Color swatch
        let sw = Rect::new(swatch_rect.x + 18.0, y, swatch_w, swatch_h);
        painter.rect_filled(sw, 8.0, *color);
        painter.rect_stroke(sw, 8.0, 1.0, fox.text.with_alpha(0.15));

        // Label at Caption size (24px)
        TextLabel::new(label, swatch_rect.x + 18.0 + swatch_w + 14.0, y + 8.0)
            .size(FontSize::Caption)
            .color(fox.text_secondary)
            .max_width(swatch_rect.w - swatch_w - 50.0)
            .draw(text_renderer, size.0, size.1);

        y += row_h;
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tab 2: Controls
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn draw_controls_tab(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    ix: &mut InteractionContext,
    content: Rect,
    slider_value: f32,
    checkbox_states: [bool; 3],
    size: (u32, u32),
) {
    let border = fox.muted.with_alpha(0.3);

    // ── Buttons row ──────────────────────────────────────────────────────
    let btn_area = ctrl_buttons_rect(content);
    Panel::new(btn_area)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(btn_area, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Button Variants", btn_area.x + 18.0, btn_area.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let btn_y = btn_area.y + 50.0;
    let btn_w = 140.0;
    let btn_h = 38.0;
    let btn_gap = 18.0;

    let variants = [
        (ZONE_BTN_DEFAULT, "Default", ButtonVariant::Default),
        (ZONE_BTN_PRIMARY, "Primary", ButtonVariant::Primary),
        (ZONE_BTN_GHOST, "Ghost", ButtonVariant::Ghost),
    ];

    for (i, (zone_id, label, variant)) in variants.iter().enumerate() {
        let bx = btn_area.x + 18.0 + i as f32 * (btn_w + btn_gap);
        let rect = Rect::new(bx, btn_y, btn_w, btn_h);
        let state = ix.add_zone(*zone_id, rect);

        Button::new(rect, label)
            .variant(*variant)
            .hovered(state.is_hovered())
            .pressed(state.is_active())
            .draw(painter, text_renderer, fox, size.0, size.1);
    }

    // ── Slider ───────────────────────────────────────────────────────────
    let slider_panel = ctrl_slider_panel_rect(content);
    let slider_rect = ctrl_slider_control_rect(content);
    let slider_state = ix.add_zone(ZONE_SLIDER, slider_rect);

    Panel::new(slider_panel)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(slider_panel, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Gold Slider", slider_panel.x + 18.0, slider_panel.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let pct_text = format!("{:.0}%", slider_value * 100.0);
    TextLabel::new(&pct_text, slider_rect.x + slider_rect.w + 14.0, slider_panel.y + 44.0)
        .size(FontSize::Caption)
        .color(fox.accent)
        .max_width(70.0)
        .draw(text_renderer, size.0, size.1);

    Slider::new(slider_rect)
        .value(slider_value)
        .hovered(slider_state.is_hovered())
        .active(slider_state.is_active())
        .draw(painter, fox);

    // ── Checkboxes ───────────────────────────────────────────────────────
    let cb_area = ctrl_checkboxes_rect(content);
    Panel::new(cb_area)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(cb_area, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Checkboxes", cb_area.x + 18.0, cb_area.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let cb_y_start = cb_area.y + 50.0;
    let cb_row_h = 48.0;
    let cb_w = 220.0;

    let checkboxes = [
        (ZONE_CB_ONE, "Enable feature", checkbox_states[0], false),
        (ZONE_CB_TWO, "Dark mode", checkbox_states[1], false),
        (ZONE_CB_THREE, "Disabled option", checkbox_states[2], true),
    ];

    for (i, (zone_id, label, checked, disabled)) in checkboxes.iter().enumerate() {
        let rect = Rect::new(cb_area.x + 18.0, cb_y_start + i as f32 * cb_row_h, cb_w, cb_row_h);
        let state = ix.add_zone(*zone_id, rect);

        Checkbox::new(rect, *checked)
            .label(label)
            .hovered(state.is_hovered())
            .disabled(*disabled)
            .draw(painter, text_renderer, fox, size.0, size.1);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tab 3: Inputs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub fn draw_inputs_tab(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    ix: &mut InteractionContext,
    content: Rect,
    text_input_value: &str,
    focused_input: Option<u32>,
    nested_tab: usize,
    nested_tab_hovered: Option<usize>,
    size: (u32, u32),
) {
    let border = fox.muted.with_alpha(0.3);

    // ── Text inputs ──────────────────────────────────────────────────────
    let input_area = inputs_text_area_rect(content);
    Panel::new(input_area)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(input_area, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Text Input", input_area.x + 18.0, input_area.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let input_w = (input_area.w - 36.0).min(400.0);
    let input_h = 48.0;
    let input_x = input_area.x + 18.0;
    let input_gap = 18.0;

    // Input 1: empty with placeholder
    let y1 = input_area.y + 48.0;
    let r1 = Rect::new(input_x, y1, input_w, input_h);
    let s1 = ix.add_zone(ZONE_INPUT_EMPTY, r1);
    TextInput::new(r1)
        .placeholder("Type something...")
        .focused(focused_input == Some(ZONE_INPUT_EMPTY))
        .hovered(s1.is_hovered())
        .draw(painter, text_renderer, fox, size.0, size.1);

    // Input 2: filled with text
    let y2 = y1 + input_h + input_gap;
    let r2 = Rect::new(input_x, y2, input_w, input_h);
    let s2 = ix.add_zone(ZONE_INPUT_FILLED, r2);
    TextInput::new(r2)
        .text(text_input_value)
        .placeholder("Editable field")
        .focused(focused_input == Some(ZONE_INPUT_FILLED))
        .hovered(s2.is_hovered())
        .draw(painter, text_renderer, fox, size.0, size.1);

    // Input 3: always focused (demo)
    let y3 = y2 + input_h + input_gap;
    let r3 = Rect::new(input_x, y3, input_w, input_h);
    let s3 = ix.add_zone(ZONE_INPUT_FOCUSED, r3);
    TextInput::new(r3)
        .text("Always focused")
        .focused(true)
        .hovered(s3.is_hovered())
        .draw(painter, text_renderer, fox, size.0, size.1);

    // ── Nested tab bar demo ──────────────────────────────────────────────
    let tabs_area = inputs_nested_tabs_rect(content);
    Panel::new(tabs_area)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(tabs_area, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Nested TabBar", tabs_area.x + 18.0, tabs_area.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let nested_bar_rect = Rect::new(
        tabs_area.x + 18.0,
        tabs_area.y + 48.0,
        tabs_area.w - 36.0,
        38.0,
    );

    let tab_labels: &[&str] = &["Alpha", "Beta", "Gamma"];
    TabBar::new(nested_bar_rect)
        .tabs(tab_labels)
        .selected(nested_tab)
        .hovered(nested_tab_hovered)
        .draw(painter, text_renderer, fox, size.0, size.1);

    // Content below nested tabs
    let nested_content_y = nested_bar_rect.y + nested_bar_rect.h + 14.0;
    let label = match nested_tab {
        0 => "Alpha content - first nested tab selected",
        1 => "Beta content - second nested tab selected",
        2 => "Gamma content - third nested tab selected",
        _ => "Unknown tab",
    };
    TextLabel::new(label, tabs_area.x + 36.0, nested_content_y)
        .size(FontSize::Caption)
        .color(fox.muted)
        .max_width(tabs_area.w - 72.0)
        .draw(text_renderer, size.0, size.1);
}

