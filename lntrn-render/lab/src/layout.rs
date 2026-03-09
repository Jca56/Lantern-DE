use lntrn_render::Rect;

// ── Global layout constants ──────────────────────────────────────────────────

/// Height of the title bar area.
pub const TITLE_BAR_H: f32 = 52.0;
/// Height of the gradient accent strip.
pub const ACCENT_STRIP_H: f32 = 4.0;
/// Height of the tab bar.
pub const TAB_BAR_H: f32 = 44.0;
/// Inset from window edges to main panel.
pub const PANEL_INSET: f32 = 28.0;
/// Padding inside the content area.
pub const CONTENT_PAD: f32 = 18.0;
/// Footer height.
pub const FOOTER_H: f32 = 48.0;
/// Section gap between side-by-side panels.
pub const SECTION_GAP: f32 = 18.0;
/// Vertical gap between stacked sections.
pub const SECTION_V_GAP: f32 = 12.0;
/// Standard sub-panel corner radius.
pub const SUB_PANEL_RADIUS: f32 = 14.0;

// ── Scroll demo constants ────────────────────────────────────────────────────

pub const SCROLL_DEMO_ITEMS: usize = 15;
pub const SCROLL_DEMO_ITEM_H: f32 = 40.0;

// ── Checkerboard texture generation ──────────────────────────────────────────

pub fn generate_checkerboard(size: u32, cell: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let even = ((x / cell) + (y / cell)) % 2 == 0;
            let i = ((y * size + x) * 4) as usize;
            if even {
                rgba[i..i + 4].copy_from_slice(&[200, 134, 10, 255]);
            } else {
                rgba[i..i + 4].copy_from_slice(&[40, 40, 40, 255]);
            }
        }
    }
    rgba
}

// ── Main panel ───────────────────────────────────────────────────────────────

pub fn panel_rect(size: (u32, u32)) -> Rect {
    Rect::new(
        PANEL_INSET,
        PANEL_INSET,
        (size.0 as f32 - PANEL_INSET * 2.0).max(120.0),
        (size.1 as f32 - PANEL_INSET * 2.0).max(120.0),
    )
}

/// The title bar rect at the top of the panel.
pub fn title_bar_rect(panel: Rect) -> Rect {
    Rect::new(panel.x, panel.y, panel.w, TITLE_BAR_H)
}

/// The tab bar rect just below the title bar.
pub fn tab_bar_rect(panel: Rect) -> Rect {
    Rect::new(
        panel.x,
        panel.y + TITLE_BAR_H + ACCENT_STRIP_H,
        panel.w,
        TAB_BAR_H,
    )
}

/// The content area below tabs, above footer.
pub fn content_rect(panel: Rect) -> Rect {
    let top = panel.y + TITLE_BAR_H + ACCENT_STRIP_H + TAB_BAR_H;
    let bottom = panel.y + panel.h - FOOTER_H;
    Rect::new(
        panel.x + CONTENT_PAD,
        top + CONTENT_PAD,
        (panel.w - CONTENT_PAD * 2.0).max(60.0),
        (bottom - top - CONTENT_PAD * 2.0).max(60.0),
    )
}

// ── Typography tab layout ────────────────────────────────────────────────────

/// Left column: text scale reference (50%).
pub fn typo_text_scale_rect(content: Rect) -> Rect {
    let w = (content.w - SECTION_GAP) * 0.5;
    Rect::new(content.x, content.y, w, content.h)
}

/// Right column: color swatches (50%).
pub fn typo_color_swatch_rect(content: Rect) -> Rect {
    let left_w = (content.w - SECTION_GAP) * 0.5;
    let x = content.x + left_w + SECTION_GAP;
    let w = content.w - left_w - SECTION_GAP;
    Rect::new(x, content.y, w, content.h)
}

// ── Controls tab layout ──────────────────────────────────────────────────────

/// Top section: buttons row.
pub fn ctrl_buttons_rect(content: Rect) -> Rect {
    Rect::new(content.x, content.y, content.w, 100.0)
}

/// Middle section: slider panel (full width for the panel, slider inside is narrower).
pub fn ctrl_slider_panel_rect(content: Rect) -> Rect {
    let y = content.y + 100.0 + SECTION_V_GAP;
    Rect::new(content.x, y, content.w, 88.0)
}

/// The slider control itself: 60% width, left-aligned within the panel.
pub fn ctrl_slider_control_rect(content: Rect) -> Rect {
    let panel = ctrl_slider_panel_rect(content);
    let slider_w = (panel.w - 36.0) * 0.6;
    Rect::new(panel.x + 18.0, panel.y + 44.0, slider_w, 28.0)
}

/// Bottom section: checkboxes.
pub fn ctrl_checkboxes_rect(content: Rect) -> Rect {
    let y = content.y + 100.0 + SECTION_V_GAP + 88.0 + SECTION_V_GAP;
    let h = (content.h - 100.0 - 88.0 - SECTION_V_GAP * 2.0).max(60.0);
    Rect::new(content.x, y, content.w, h)
}

// ── Inputs tab layout ────────────────────────────────────────────────────────

/// Top section: text input demos (stacked vertically).
pub fn inputs_text_area_rect(content: Rect) -> Rect {
    let h = (content.h * 0.55).max(120.0);
    Rect::new(content.x, content.y, content.w, h)
}

/// Bottom section: nested tab bar demo.
pub fn inputs_nested_tabs_rect(content: Rect) -> Rect {
    let top_h = (content.h * 0.55).max(120.0);
    let y = content.y + top_h + SECTION_V_GAP;
    let h = (content.h - top_h - SECTION_V_GAP).max(60.0);
    Rect::new(content.x, y, content.w, h)
}

// ── Containers tab layout ────────────────────────────────────────────────────

/// Left column: panel demos + alpha gradient.
pub fn cont_panel_area_rect(content: Rect) -> Rect {
    let w = (content.w - SECTION_GAP) * 0.5;
    Rect::new(content.x, content.y, w, content.h)
}

/// Right column: scroll area + texture demo.
pub fn cont_scroll_area_rect(content: Rect) -> Rect {
    let left_w = (content.w - SECTION_GAP) * 0.5;
    let x = content.x + left_w + SECTION_GAP;
    let w = content.w - left_w - SECTION_GAP;
    Rect::new(x, content.y, w, content.h)
}

/// Slider value from cursor X position relative to the controls-tab slider.
pub fn slider_value_for_x(content: Rect, x: f32) -> f32 {
    let slider = ctrl_slider_control_rect(content);
    ((x - slider.x) / slider.w.max(1.0)).clamp(0.0, 1.0)
}
