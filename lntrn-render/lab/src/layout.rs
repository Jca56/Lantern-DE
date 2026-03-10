use lntrn_render::Rect;

pub const TITLE_BAR_H: f32 = 48.0;
pub const CONTENT_PAD: f32 = 20.0;
pub const CARD_GAP: f32 = 16.0;
pub const CARD_PAD: f32 = 18.0;
pub const CARD_RADIUS: f32 = 14.0;

// ── Zone IDs ────────────────────────────────────────────────────────────────

pub const Z_MAIN_SCROLL: u32 = 5;

// Buttons
pub const Z_BTN_DEFAULT: u32 = 20;
pub const Z_BTN_PRIMARY: u32 = 21;
pub const Z_BTN_GHOST: u32 = 22;
pub const Z_BTN_DANGER: u32 = 23;

// Inputs
pub const Z_INPUT: u32 = 50;
pub const Z_DROPDOWN: u32 = 55;
pub const Z_DROPDOWN_OPT: u32 = 56; // 56..56+N

// Controls
pub const Z_TOGGLE_A: u32 = 60;
pub const Z_TOGGLE_B: u32 = 61;
pub const Z_CHECKBOX_A: u32 = 62;
pub const Z_CHECKBOX_B: u32 = 63;
pub const Z_RADIO_A: u32 = 64;
pub const Z_RADIO_B: u32 = 65;
pub const Z_RADIO_C: u32 = 66;
pub const Z_SLIDER: u32 = 67;

// Actions
pub const Z_MODAL_OPEN: u32 = 70;
pub const Z_MODAL_CANCEL: u32 = 71;
pub const Z_MODAL_CONFIRM: u32 = 72;

// Scroll demo
pub const Z_SCROLL_DEMO: u32 = 90;

// Title bar
pub const Z_TB_CLOSE: u32 = 100;
pub const Z_TB_MAXIMIZE: u32 = 101;
pub const Z_TB_MINIMIZE: u32 = 102;

// ── Grid helpers ────────────────────────────────────────────────────────────

/// Full viewport (content area below the title bar).
pub fn viewport_rect(w: f32, h: f32) -> Rect {
    Rect::new(0.0, TITLE_BAR_H, w, (h - TITLE_BAR_H).max(0.0))
}

/// Width of one column in a 2-column grid.
pub fn col_w(total_w: f32) -> f32 {
    (total_w - CONTENT_PAD * 2.0 - CARD_GAP) * 0.5
}

/// X position of the left column.
pub fn col_left() -> f32 {
    CONTENT_PAD
}

/// X position of the right column.
pub fn col_right(total_w: f32) -> f32 {
    CONTENT_PAD + col_w(total_w) + CARD_GAP
}

/// Full width (minus padding).
pub fn full_w(total_w: f32) -> f32 {
    total_w - CONTENT_PAD * 2.0
}
