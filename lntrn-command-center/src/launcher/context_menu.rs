//! Right-click context menu for launcher entries (grid + pinned row).
//!
//! Keep this generic: a menu owns a target `app_id`, an anchor position
//! in physical pixels, and a list of `MenuItem`s. Future items (open
//! file location, copy app_id, uninstall, etc.) just add to the enum
//! and the dispatch in `app.rs`. Drawing happens on layer 1 so the menu
//! always sits above grid tiles and labels.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

/// Each variant is a discrete action the menu can dispatch back to the
/// app. The action runs through `AppState::run_menu_action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    TogglePin,
    ToggleHidden,
    Launch,
    /// Close the foreign-toplevel window stored in `ContextMenu.window_title`.
    WindowClose,
    /// Minimize the foreign-toplevel window stored in `ContextMenu.window_title`.
    WindowMinimize,
    /// Terminal: copy current selection to the Wayland clipboard.
    TerminalCopy,
    /// Terminal: paste clipboard contents into the PTY.
    TerminalPaste,
    /// Terminal: clear the visible selection highlight.
    TerminalClearSelection,
    /// Files: open the targeted path (folder = navigate; file = xdg-open).
    FilesOpen,
    /// Files: switch to Terminal view and `cd` to the targeted path.
    FilesOpenInTerminal,
    /// Files: launch `lntrn-file-manager` at the targeted path.
    FilesRevealInFM,
    /// Files: put the absolute path on the Wayland clipboard.
    FilesCopyPath,
    /// Files sort options. Re-selecting the active field flips direction.
    FilesSortByName,
    FilesSortBySize,
    FilesSortByDate,
    FilesSortByType,
    /// Dock: launch a new instance of the app in `ContextMenu.app_id`,
    /// regardless of whether it already has windows open.
    DockLaunchNew,
    /// Dock: open Firefox in a private window. Only added to the menu
    /// when the right-clicked app is Firefox.
    FirefoxPrivate,
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub action: MenuAction,
}

/// Menu state. Anchor is in physical pixels — the upper-left corner of
/// where the menu wants to draw. We clamp inside the panel rect at
/// draw-time so the menu never bleeds off the right/bottom edges.
#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub app_id: String,
    /// For window-targeted menus (Open section). Empty for app menus.
    pub window_title: String,
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub items: Vec<MenuItem>,
}

// ── Layout (logical px) ─────────────────────────────────────────────────────

const MENU_WIDTH: f32 = 220.0;
const ITEM_HEIGHT: f32 = 40.0;
const MENU_PAD_V: f32 = 8.0;
const ITEM_FONT: f32 = 18.0;
const ITEM_PAD_H: f32 = 16.0;
const CORNER_RADIUS: f32 = 10.0;

const BG_RGB: (u8, u8, u8) = (32, 32, 32);
const BORDER_ALPHA: f32 = 0.18;
const HOVER_ALPHA: f32 = 0.10;
const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);

fn text_color(alpha: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(alpha)
}

/// Compute the rect (in physical pixels) the menu will occupy, clamped
/// inside the panel. Used by both draw and hit-test.
pub fn menu_rect(menu: &ContextMenu, panel: Rect, scale: f32) -> Rect {
    let w = MENU_WIDTH * scale;
    let h = MENU_PAD_V * 2.0 * scale + (menu.items.len() as f32) * ITEM_HEIGHT * scale;

    let max_x = panel.x + panel.w - 8.0 * scale - w;
    let max_y = panel.y + panel.h - 8.0 * scale - h;
    let min_x = panel.x + 8.0 * scale;
    let min_y = panel.y + 8.0 * scale;

    let x = menu.anchor_x.min(max_x).max(min_x);
    let y = menu.anchor_y.min(max_y).max(min_y);
    Rect::new(x, y, w, h)
}

/// Returns Some(action) if the click landed on an item.
pub fn hit_test(
    menu: &ContextMenu,
    panel: Rect,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> Option<MenuAction> {
    let rect = menu_rect(menu, panel, scale);
    if phys_x < rect.x || phys_x > rect.x + rect.w
        || phys_y < rect.y || phys_y > rect.y + rect.h
    {
        return None;
    }
    let item_h = ITEM_HEIGHT * scale;
    let pad_v = MENU_PAD_V * scale;
    let local_y = phys_y - rect.y - pad_v;
    if local_y < 0.0 {
        return None;
    }
    let idx = (local_y / item_h) as usize;
    menu.items.get(idx).map(|it| it.action)
}

/// True if the click landed anywhere on the menu surface (including
/// padding). Used to decide whether an outside-click should dismiss.
#[allow(dead_code)]
pub fn contains(menu: &ContextMenu, panel: Rect, scale: f32, phys_x: f32, phys_y: f32) -> bool {
    let r = menu_rect(menu, panel, scale);
    phys_x >= r.x && phys_x <= r.x + r.w && phys_y >= r.y && phys_y <= r.y + r.h
}

/// Draw the menu. Caller is responsible for switching to overlay layer
/// before calling, and for restoring layer 0 afterward.
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    menu: &ContextMenu,
    panel: Rect,
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let rect = menu_rect(menu, panel, scale);
    let radius = CORNER_RADIUS * scale;

    // Soft drop shadow for separation from grid.
    painter.shadow(
        rect,
        radius,
        16.0 * scale,
        Color::BLACK.with_alpha(0.45),
        0.0,
        4.0 * scale,
    );

    let bg = Color::from_rgb8(BG_RGB.0, BG_RGB.1, BG_RGB.2).with_alpha(0.98);
    painter.rect_filled(rect, radius, bg);
    painter.rect_stroke_sdf(
        rect,
        radius,
        1.0 * scale,
        Color::rgba(1.0, 1.0, 1.0, BORDER_ALPHA),
    );

    let item_h = ITEM_HEIGHT * scale;
    let pad_v = MENU_PAD_V * scale;
    let pad_h = ITEM_PAD_H * scale;
    let font = ITEM_FONT * scale;

    for (i, item) in menu.items.iter().enumerate() {
        let row_y = rect.y + pad_v + (i as f32) * item_h;
        let row_rect = Rect::new(rect.x + 4.0 * scale, row_y, rect.w - 8.0 * scale, item_h);

        // Subtle hover-feel hint on every other row — keeps the list
        // scannable. Real hover styling would want pointer-tracking;
        // skip until we need it.
        let _ = row_rect;
        let _ = HOVER_ALPHA;

        text.queue(
            &item.label,
            font,
            rect.x + pad_h,
            row_y + (item_h - font) / 2.0,
            text_color(0.95),
            rect.w - pad_h * 2.0,
            surface_w,
            surface_h,
        );
    }
}
