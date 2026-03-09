use lntrn_render::Rect;

pub const TITLE_BAR_H: f32 = 52.0;
pub const PANEL_INSET: f32 = 28.0;
pub const CONTENT_PAD: f32 = 24.0;
pub const SECTION_GAP: f32 = 20.0;
pub const SECTION_PAD: f32 = 18.0;
pub const SUB_RADIUS: f32 = 14.0;

// ── Zone IDs ────────────────────────────────────────────────────────────────

pub const Z_CLOSE: u32 = 1;
pub const Z_MINIMIZE: u32 = 2;
pub const Z_MAXIMIZE: u32 = 3;
pub const Z_MAIN_SCROLL: u32 = 5;

pub const Z_TRANSPARENCY: u32 = 10;
pub const Z_TEXT_SIZE: u32 = 11;

pub const Z_BTN_DEFAULT: u32 = 20;
pub const Z_BTN_PRIMARY: u32 = 21;
pub const Z_BTN_GHOST: u32 = 22;
pub const Z_SLIDER: u32 = 25;

pub const Z_CB_BASE: u32 = 30; // 30, 31, 32
pub const Z_TOGGLE_BASE: u32 = 35; // 35, 36
pub const Z_RADIO_BASE: u32 = 40; // 40, 41, 42

pub const Z_INPUT_BASE: u32 = 50; // 50, 51, 52
pub const Z_DROPDOWN: u32 = 55;
pub const Z_DROPDOWN_OPT: u32 = 56; // 56..56+N

pub const Z_MODAL_OPEN: u32 = 70;
pub const Z_MODAL_CANCEL: u32 = 71;
pub const Z_MODAL_CONFIRM: u32 = 72;
pub const Z_TOAST_SPAWN: u32 = 75;

pub const Z_SCROLL_DEMO: u32 = 90;

// ── Rect helpers ────────────────────────────────────────────────────────────

pub fn panel_rect(size: (u32, u32)) -> Rect {
    Rect::new(
        PANEL_INSET,
        PANEL_INSET,
        (size.0 as f32 - PANEL_INSET * 2.0).max(120.0),
        (size.1 as f32 - PANEL_INSET * 2.0).max(120.0),
    )
}

pub fn title_bar_rect(panel: Rect) -> Rect {
    Rect::new(panel.x, panel.y, panel.w, TITLE_BAR_H)
}

/// The scrollable viewport below the title bar.
pub fn viewport_rect(panel: Rect) -> Rect {
    let top = panel.y + TITLE_BAR_H + 4.0;
    Rect::new(panel.x, top, panel.w, (panel.h - TITLE_BAR_H - 4.0).max(60.0))
}
