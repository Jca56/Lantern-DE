//! Right-click context menu for the editor — Copy / Cut / Paste / Select All.
//!
//! Uses the shared `lntrn_ui::gpu::ContextMenu` so it looks and behaves exactly
//! like the menus in the Terminal and File Manager: a near-black panel with an
//! accent-tinted border + hover that tracks the user's theme accent. More items
//! (and eventually a floating mini-toolbar) can be added to `build_items`.

use lntrn_render::Color;
use lntrn_ui::gpu::{ContextMenuStyle, FoxPalette, MenuEvent, MenuItem};

use crate::actions;
use crate::TextHandler;

// ── Menu action IDs ──────────────────────────────────────────────────────────
// Kept clear of the title-bar menu IDs in main.rs (100–202).
pub const CTX_COPY: u32 = 300;
pub const CTX_CUT: u32 = 301;
pub const CTX_PASTE: u32 = 302;
pub const CTX_SELECT_ALL: u32 = 303;

/// Context-menu style shared with the Terminal and File Manager: a near-black
/// panel with an accent-tinted border + hover. The #0A0A0A background is
/// Lantern's standard menu black — an intentional, allowed exception to the
/// no-hardcoded-colors rule (matches `lntrn-terminal` and `lntrn-file-manager`).
pub fn context_menu_style(palette: &FoxPalette) -> ContextMenuStyle {
    let mut style = ContextMenuStyle::from_palette(palette);
    style.bg = Color::from_rgba8(10, 10, 10, 255);
    style.bg_hover = palette.accent.with_alpha(0.16);
    style.border = palette.accent.with_alpha(0.35);
    style.border_width = 1.5;
    style.corner_radius = 12.0;
    style.separator = palette.accent.with_alpha(0.12);
    style
}

/// Build the editor right-click menu. Copy/Cut are greyed out without a
/// selection; Paste is always offered (the clipboard is read on demand).
pub fn build_items(has_selection: bool) -> Vec<MenuItem> {
    vec![
        MenuItem::action_with(CTX_COPY, "Copy", "Ctrl+C").enabled(has_selection),
        MenuItem::action_with(CTX_CUT, "Cut", "Ctrl+X").enabled(has_selection),
        MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+V"),
        MenuItem::separator(),
        MenuItem::action_with(CTX_SELECT_ALL, "Select All", "Ctrl+A"),
    ]
}

/// Dispatch a context-menu event to the matching editor action. Returns `true`
/// if it was one of our actions (so the caller closes the menu and redraws).
pub fn handle_event(handler: &mut TextHandler, event: &MenuEvent) -> bool {
    let MenuEvent::Action(id) = event else {
        return false;
    };
    match *id {
        CTX_COPY => actions::do_copy(handler),
        CTX_CUT => actions::do_cut(handler),
        CTX_PASTE => actions::do_paste(handler),
        CTX_SELECT_ALL => handler.editor_mut().select_all(),
        _ => return false,
    }
    true
}
