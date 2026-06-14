use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext, MenuBar,
    MenuEvent, MenuItem,
};

use crate::config::{CursorStylePref, WindowMode};
use crate::night_sky;

// ── Constants ───────────────────────────────────────────────────────────────

/// Unified title bar height — accommodates inline tabs, menus, and window controls.
pub const TITLE_BAR_HEIGHT: f32 = 40.0;

/// Width reserved on the right for window controls (same in both modes — they
/// share the circular control style). Buttons sit at w-30 / w-60 / w-90 with
/// 10px radius, so the leftmost edge is at w-100 — reserve a bit beyond that.
const CONTROLS_W: f32 = 108.0;

/// Left margin for the menu bar.
const MENU_LEFT: f32 = 8.0;
/// Padding around the divider after the menus.
const DIVIDER_PAD: f32 = 14.0;
/// Extra inset between the divider and the first tab so the pill doesn't
/// kiss the divider.
const TABS_LEFT_INSET: f32 = 12.0;
/// Vertical line thickness for the divider.
const DIVIDER_W: f32 = 2.5;

// Replicated from lntrn_ui::gpu::menu_bar (private constants there).
// Compact menu font for the title bar. The MenuBar is constructed with this
// same value (see ChromeState::new) so our layout width calc lines up exactly
// with where the menu labels actually render. Smaller than the theme body font
// so the title bar can be slim.
const MENU_FONT_BODY: f32 = 22.0;
const MENU_LABEL_PAD_H: f32 = 8.0;
const MENU_LABEL_GAP: f32 = 12.0;

/// Title bar height for the given window mode (both modes share TITLE_BAR_HEIGHT).
pub fn title_bar_height(_mode: &WindowMode) -> f32 {
    TITLE_BAR_HEIGHT
}

// ── Menu action IDs ─────────────────────────────────────────────────────────

pub const MENU_FONT_SLIDER: u32 = 100;
pub const MENU_SPLIT_RIGHT: u32 = 200;
pub const MENU_SPLIT_DOWN: u32 = 201;
pub const MENU_CLOSE_PANE: u32 = 202;
pub const MENU_PREV_PANE: u32 = 203;
pub const MENU_NEXT_PANE: u32 = 204;
pub const MENU_TOGGLE_SIDEBAR: u32 = 300;
/// View menu toggle: open new windows with the title/tab bar hidden
/// (same "rice mode" Super+F11 toggles live).
pub const MENU_OPEN_BAR_HIDDEN: u32 = 301;

// Cursor style radio group + items
pub const CURSOR_STYLE_GROUP: u32 = 500;
pub const MENU_CURSOR_BLOCK: u32 = 501;
pub const MENU_CURSOR_UNDERLINE: u32 = 502;
pub const MENU_CURSOR_BEAM: u32 = 503;

// Context menu (right-click)
pub const CTX_COPY: u32 = 400;
pub const CTX_PASTE: u32 = 401;
pub const CTX_SELECT_ALL: u32 = 402;
pub const CTX_CLEAR_SCROLLBACK: u32 = 403;
pub const CTX_NEW_TAB: u32 = 404;
pub const CTX_CLOSE_TAB: u32 = 405;
// Mini title bar (window controls) at the top of the context menu
pub const CTX_MINIMIZE: u32 = 406;
pub const CTX_MAXIMIZE: u32 = 407;
pub const CTX_CLOSE_WINDOW: u32 = 408;
/// "lntrn" brand label in the mini title bar — types `lntrn` + Enter into
/// the active pane.
pub const CTX_LNTRN: u32 = 409;
// Tab-cycle chevrons in the mini title bar (only shown with >1 tab)
pub const CTX_PREV_TAB: u32 = 410;
pub const CTX_NEXT_TAB: u32 = 411;
pub const CTX_OPEN_SIDEBAR: u32 = 412;
// Sidebar (file tree) context menu — same ContextMenu widget as the
// terminal menu, different item set. Must stay below CTX_TAB_DOT_BASE.
pub const CTX_SB_NEW_FILE: u32 = 420;
pub const CTX_SB_NEW_FOLDER: u32 = 421;
pub const CTX_SB_RENAME: u32 = 422;
pub const CTX_SB_DELETE: u32 = 423;
pub const CTX_SB_OPEN_CODE: u32 = 424;
/// Base id for the tab-indicator dots floating above the context menu.
/// Dot for tab `i` fires `CTX_TAB_DOT_BASE + i` — this range is open-ended
/// (one id per tab), so keep it the HIGHEST id block in the menu.
pub const CTX_TAB_DOT_BASE: u32 = 500;

// ── State ───────────────────────────────────────────────────────────────────

pub struct ChromeState {
    pub menu_bar: MenuBar,
    pub context_menu: ContextMenu,
    pub palette: FoxPalette,
}

impl ChromeState {
    pub fn new() -> Self {
        let palette = FoxPalette::current();
        Self {
            menu_bar: MenuBar::new(&palette).with_font_size(MENU_FONT_BODY),
            context_menu: ContextMenu::new(context_menu_style(&palette)),
            palette,
        }
    }

    /// Re-read the active theme/accent and restyle the context menu. Called
    /// right before opening so a theme change in System Settings shows up
    /// without restarting the terminal.
    pub fn refresh_theme(&mut self) {
        self.palette = FoxPalette::current();
        self.context_menu.set_style(context_menu_style(&self.palette));
    }

    pub fn has_overlay(&self) -> bool {
        self.menu_bar.is_open() || self.context_menu.is_open()
    }

    pub fn close_all_menus(&mut self) {
        self.menu_bar.close();
        self.context_menu.close();
    }
}

/// Terminal context-menu style: a black panel (instead of the stock flat
/// `surface_2` grey) with an accent-tinted border and hover so it follows
/// the user's theme + accent override. The bg is Lantern's standard black
/// (#0A0A0A) by explicit user preference — exempt from the no-hardcoded-
/// colors rule.
pub fn context_menu_style(palette: &FoxPalette) -> ContextMenuStyle {
    let mut style = ContextMenuStyle::from_palette(palette);
    style.bg = Color::from_rgba8(10, 10, 10, 255);
    style.bg_hover = palette.accent.with_alpha(0.16);
    style.border = palette.accent.with_alpha(0.35);
    style.border_width = 2.5;
    style.corner_radius = 12.0;
    style.separator = palette.accent.with_alpha(0.12);
    style
}

// ── Click actions (kept for event dispatch compatibility) ───────────────────

pub enum ClickAction {
    None,
    Close,
    Minimize,
    Maximize,
    StartDrag,
    SliderDrag,
    WindowModeChanged,
    SplitHorizontal,
    SplitVertical,
    ClosePane,
    /// Jump directly to tab `i` (from the context-menu tab dots).
    SelectTab(usize),
    FocusPrevPane,
    FocusNextPane,
    ToggleSidebar,
    Copy,
    Paste,
    SelectAll,
    NewTab,
    CloseTab,
    ClearScrollback,
    PrevTab,
    NextTab,
    /// Type `lntrn` + Enter into the active pane (context-menu brand label).
    RunLntrn,
    // Sidebar file-tree context menu actions — operate on the entry
    // remembered in `SidebarState::menu_target`.
    SidebarNewFile,
    SidebarNewFolder,
    SidebarRename,
    SidebarDelete,
    SidebarOpenCode,
}

// ── Menu definitions ────────────────────────────────────────────────────────

pub fn build_menus(
    font_size: f32,
    _sidebar_visible: bool,
    cursor_style: CursorStylePref,
    open_bar_hidden: bool,
    _mode: &WindowMode,
) -> Vec<(&'static str, Vec<MenuItem>)> {
    let block_sel = matches!(cursor_style, CursorStylePref::Block);
    let underline_sel = matches!(cursor_style, CursorStylePref::Underline);
    let beam_sel = matches!(cursor_style, CursorStylePref::Beam);
    vec![
        ("Files", vec![
            MenuItem::action(MENU_TOGGLE_SIDEBAR, "Toggle Sidebar"),
        ]),
        ("View", vec![
            MenuItem::slider(MENU_FONT_SLIDER, "Text Size", ((font_size - 6.0) / 24.0).clamp(0.0, 1.0)),
            MenuItem::separator(),
            MenuItem::header("Cursor Style"),
            MenuItem::radio(MENU_CURSOR_BLOCK, CURSOR_STYLE_GROUP, "Block", block_sel),
            MenuItem::radio(MENU_CURSOR_UNDERLINE, CURSOR_STYLE_GROUP, "Underline", underline_sel),
            MenuItem::radio(MENU_CURSOR_BEAM, CURSOR_STYLE_GROUP, "Beam", beam_sel),
            MenuItem::separator(),
            MenuItem::toggle(MENU_OPEN_BAR_HIDDEN, "Open with Bar Hidden", open_bar_hidden),
        ]),
        ("Split", vec![
            MenuItem::action_with(MENU_SPLIT_RIGHT, "Split Right", "Ctrl+Shift+D"),
            MenuItem::action_with(MENU_SPLIT_DOWN, "Split Down", "Ctrl+Shift+E"),
            MenuItem::separator(),
            MenuItem::action_with(MENU_CLOSE_PANE, "Close Pane", "Ctrl+Shift+W"),
            MenuItem::separator(),
            MenuItem::action_with(MENU_PREV_PANE, "Prev Pane", "Ctrl+Shift+["),
            MenuItem::action_with(MENU_NEXT_PANE, "Next Pane", "Ctrl+Shift+]"),
        ]),
    ]
}

pub fn build_context_menu(
    has_selection: bool,
    tab_count: usize,
    active_tab: usize,
    pane_count: usize,
    sidebar_visible: bool,
    font_size: f32,
) -> Vec<MenuItem> {
    let copy = if has_selection {
        MenuItem::action_with(CTX_COPY, "Copy", "Ctrl+Shift+C")
    } else {
        MenuItem::action_disabled(CTX_COPY, "Copy")
    };
    // Mini title bar: window controls up top, handy in rice mode where
    // the real title bar is hidden. The "lntrn" brand label runs the
    // `lntrn` command; tab-cycle chevrons appear with more than one tab.
    let mut controls = MenuItem::window_controls(CTX_MINIMIZE, CTX_MAXIMIZE, CTX_CLOSE_WINDOW)
        .controls_title(CTX_LNTRN, "lntrn");
    if tab_count > 1 {
        controls = controls
            .controls_nav(CTX_PREV_TAB, CTX_NEXT_TAB)
            .controls_tabs(CTX_TAB_DOT_BASE, tab_count, active_tab);
    }
    let sidebar_label = if sidebar_visible { "Close Sidebar" } else { "Open Sidebar" };
    vec![
        controls,
        MenuItem::separator(),
        MenuItem::slider(MENU_FONT_SLIDER, "Text Size", ((font_size - 6.0) / 24.0).clamp(0.0, 1.0)),
        MenuItem::action(CTX_OPEN_SIDEBAR, sidebar_label),
        MenuItem::separator(),
        copy,
        MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+Shift+V"),
        MenuItem::action(CTX_SELECT_ALL, "Select All"),
        MenuItem::separator(),
        MenuItem::action_with(CTX_NEW_TAB, "New Tab", "Ctrl+Shift+T"),
        MenuItem::action(CTX_CLOSE_TAB, "Close Tab"),
        MenuItem::separator(),
        MenuItem::action_with(MENU_SPLIT_RIGHT, "Split Right", "Ctrl+Shift+D"),
        MenuItem::action_with(MENU_SPLIT_DOWN, "Split Down", "Ctrl+Shift+E"),
        MenuItem::action_with(MENU_CLOSE_PANE, "Close Pane", "Ctrl+Shift+W").enabled(pane_count > 1),
    ]
}

/// Context menu for a file-tree sidebar entry. Same widget + style as the
/// terminal menu; the header names the target so it's obvious what the
/// actions apply to. `is_root` = right-click on empty space (root dir).
pub fn build_sidebar_context_menu(name: &str, is_root: bool, is_dir: bool) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::header(name), MenuItem::separator()];
    if is_root || is_dir {
        items.push(MenuItem::action(CTX_SB_NEW_FILE, "New File"));
        items.push(MenuItem::action(CTX_SB_NEW_FOLDER, "New Folder"));
    } else {
        items.push(MenuItem::action(CTX_SB_OPEN_CODE, "Open with Lantern Code"));
    }
    if !is_root {
        items.push(MenuItem::separator());
        items.push(MenuItem::action(CTX_SB_RENAME, "Rename"));
        items.push(MenuItem::action_danger(CTX_SB_DELETE, "Delete"));
    }
    items
}

// ── Layout ──────────────────────────────────────────────────────────────────

/// Resolved horizontal regions inside the title bar for the current frame.
/// All values are in screen pixels.
#[derive(Clone, Copy)]
pub struct ChromeLayout {
    /// Total bar height (logical = screen since scale is 1.0).
    pub bar_h: f32,
    /// Menus start x.
    pub menu_left: f32,
    /// Right edge of the menu labels (before divider).
    pub menu_right: f32,
    /// X of the vertical divider line.
    pub divider_x: f32,
    /// Tabs region — left edge.
    pub tabs_left: f32,
    /// Tabs region — right edge (where window controls begin).
    pub tabs_right: f32,
}

/// Compute the horizontal layout of menus / divider / tabs / controls without
/// any drawing. Used by both `draw_chrome` and `tabs_bounds` (for hit testing).
pub fn compute_layout(
    menus: &[(&'static str, Vec<MenuItem>)],
    screen_w: f32,
    scale: f32,
    _mode: &WindowMode,
) -> ChromeLayout {
    let bar_h = TITLE_BAR_HEIGHT * scale;
    let menu_left = MENU_LEFT * scale;
    let menu_right = menu_bar_right_edge(menus, menu_left, scale);
    let divider_pad = DIVIDER_PAD * scale;
    let divider_x = menu_right + divider_pad;
    let tabs_left = divider_x + DIVIDER_W * scale + TABS_LEFT_INSET * scale;
    let controls_w = CONTROLS_W * scale;
    let controls_left = screen_w - controls_w;
    let tabs_right = (controls_left - 8.0 * scale).max(tabs_left);
    ChromeLayout {
        bar_h,
        menu_left,
        menu_right,
        divider_x,
        tabs_left,
        tabs_right,
    }
}

/// Replicates the menu bar's internal label-rect calculation so we know where
/// the menus end (no public accessor on MenuBar).
fn menu_bar_right_edge(menus: &[(&'static str, Vec<MenuItem>)], rect_x: f32, scale: f32) -> f32 {
    let font = MENU_FONT_BODY * scale;
    let pad_h = MENU_LABEL_PAD_H * scale;
    let gap = MENU_LABEL_GAP * scale;
    let mut x = rect_x + pad_h * 0.5;
    for (label, _) in menus.iter() {
        let text_w = label.len() as f32 * font * 0.52;
        let w = text_w + pad_h * 2.0;
        x += w + gap;
    }
    // The loop adds a trailing gap after the last label — strip it so we get
    // the actual right edge of the last menu label.
    if menus.is_empty() { x } else { x - gap }
}

/// Convenience: compute just the tabs region as a Rect for hit testing.
pub fn tabs_bounds(
    menus: &[(&'static str, Vec<MenuItem>)],
    screen_w: f32,
    scale: f32,
    mode: &WindowMode,
) -> Rect {
    let l = compute_layout(menus, screen_w, scale, mode);
    Rect::new(l.tabs_left, 0.0, (l.tabs_right - l.tabs_left).max(0.0), l.bar_h)
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Draw the title bar contents: menus, divider, and circular window controls.
/// No background bar is drawn — we let the window background (Fox: solid bg,
/// Night Sky: gradient) flow through so the title bar is seamless with the
/// terminal area below.
///
/// Returns the layout so the caller can position the tab bar.
pub fn draw_chrome(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &mut ChromeState,
    input: &mut InteractionContext,
    screen_w: u32,
    screen_h: u32,
    font_size: f32,
    sidebar_visible: bool,
    cursor_style: CursorStylePref,
    open_bar_hidden: bool,
    _maximized: bool,
    scale: f32,
    mode: &WindowMode,
    cursor_pos: Option<(f32, f32)>,
) -> ChromeLayout {
    let s = scale;
    let pal = &state.palette;
    let wf = screen_w as f32;

    let menus = build_menus(font_size, sidebar_visible, cursor_style, open_bar_hidden, mode);
    let layout = compute_layout(&menus, wf, s, mode);

    // ── Window controls — same circular style, mode-specific palette ────
    let ctrl_pal = match mode {
        WindowMode::Lantern => night_sky::ControlPalette::lantern(),
        _ => night_sky::ControlPalette::default_palette(),
    };
    night_sky::draw_controls(painter, cursor_pos, wf, &ctrl_pal, s);

    // ── Menu bar in the menu region ─────────────────────────────────────
    let menu_area = Rect::new(layout.menu_left, 0.0, layout.menu_right - layout.menu_left, layout.bar_h);
    state.menu_bar.update(input, &menus, menu_area, s);
    let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
    state.menu_bar.draw_with_labels(painter, text, pal, &labels, screen_w, screen_h, s);

    // ── Divider between menus and tabs ──────────────────────────────────
    draw_divider(painter, layout.divider_x, layout.bar_h, s, mode);

    layout
}

/// Draw the vertical divider between the menu bar and the tabs.
fn draw_divider(painter: &mut Painter, x: f32, bar_h: f32, scale: f32, mode: &WindowMode) {
    let color = match mode {
        WindowMode::Fox => Color::from_rgba8(255, 255, 255, 70),
        WindowMode::Lantern => Color::from_rgba8(240, 230, 210, 50),
    };
    let inset = bar_h * 0.20;
    let h = bar_h - inset * 2.0;
    painter.rect_filled(
        Rect::new(x, inset, DIVIDER_W * scale, h),
        0.0,
        color,
    );
}

/// Draw menu overlays (second render pass).
pub fn draw_overlay(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &mut ChromeState,
    input: &mut InteractionContext,
    screen_w: u32,
    screen_h: u32,
) -> Option<MenuEvent> {
    let mut event = None;

    // Menu bar dropdown
    state.menu_bar.context_menu.update(0.016);
    if let Some(e) = state.menu_bar.context_menu.draw(painter, text, input, screen_w, screen_h) {
        event = Some(e);
    }

    // Right-click context menu
    state.context_menu.update(0.016);
    if let Some(e) = state.context_menu.draw(painter, text, input, screen_w, screen_h) {
        event = Some(e);
    }

    event
}

// ── Hit testing ─────────────────────────────────────────────────────────────

/// Handle a left click on the title bar area. Returns a ClickAction.
pub fn handle_click(
    state: &mut ChromeState,
    input: &mut InteractionContext,
    menus: &[(&str, Vec<MenuItem>)],
    scale: f32,
    mode: &WindowMode,
    screen_w: f32,
) -> ClickAction {
    let (x, y) = match input.cursor() {
        Some(pos) => pos,
        None => return ClickAction::None,
    };

    // Context menu takes priority
    if state.context_menu.is_open() {
        if state.context_menu.contains(x, y) {
            return ClickAction::None;
        }
        state.context_menu.close();
        return ClickAction::None;
    }

    // Menu bar dropdown click
    if state.menu_bar.is_open() && state.menu_bar.context_menu.contains(x, y) {
        return ClickAction::None;
    }

    // "Files" label acts as a direct sidebar toggle — intercept before menu bar
    {
        let title_h = title_bar_height(mode) * scale;
        if y <= title_h {
            let font = MENU_FONT_BODY * scale;
            let pad_h = MENU_LABEL_PAD_H * scale;
            let gap = MENU_LABEL_GAP * scale;
            let ml = MENU_LEFT * scale + pad_h * 0.5;
            let mut lx = ml;
            for (label, _) in menus.iter() {
                let tw = label.len() as f32 * font * 0.52;
                let w = tw + pad_h * 2.0;
                if *label == "Files" && x >= lx && x <= lx + w {
                    state.menu_bar.close();
                    return ClickAction::ToggleSidebar;
                }
                lx += w + gap;
            }
        }
    }

    // Menu bar label click (open/close/switch)
    if state.menu_bar.on_click(input, menus, scale) {
        return ClickAction::None;
    }

    // Window control buttons (shared circular controls for both modes)
    if let Some(zone) = night_sky::hit_test_controls((x, y), screen_w, scale) {
        return match zone {
            10 => ClickAction::Close,
            11 => ClickAction::Maximize,
            12 => ClickAction::Minimize,
            _ => ClickAction::None,
        };
    }

    // Title bar drag
    let title_h = title_bar_height(mode) * scale;
    if y <= title_h {
        return ClickAction::StartDrag;
    }

    ClickAction::None
}

/// Convert a MenuEvent from the overlay draw into a ClickAction.
pub fn menu_event_to_action(event: &MenuEvent) -> ClickAction {
    match event {
        MenuEvent::Action(id) => match *id {
            MENU_SPLIT_RIGHT => ClickAction::SplitHorizontal,
            MENU_SPLIT_DOWN => ClickAction::SplitVertical,
            MENU_CLOSE_PANE => ClickAction::ClosePane,
            MENU_PREV_PANE => ClickAction::FocusPrevPane,
            MENU_NEXT_PANE => ClickAction::FocusNextPane,
            MENU_TOGGLE_SIDEBAR => ClickAction::ToggleSidebar,
            CTX_COPY => ClickAction::Copy,
            CTX_PASTE => ClickAction::Paste,
            CTX_SELECT_ALL => ClickAction::SelectAll,
            CTX_NEW_TAB => ClickAction::NewTab,
            CTX_CLOSE_TAB => ClickAction::CloseTab,
            CTX_CLEAR_SCROLLBACK => ClickAction::ClearScrollback,
            CTX_MINIMIZE => ClickAction::Minimize,
            CTX_MAXIMIZE => ClickAction::Maximize,
            CTX_CLOSE_WINDOW => ClickAction::Close,
            CTX_LNTRN => ClickAction::RunLntrn,
            CTX_PREV_TAB => ClickAction::PrevTab,
            CTX_NEXT_TAB => ClickAction::NextTab,
            CTX_OPEN_SIDEBAR => ClickAction::ToggleSidebar,
            CTX_SB_NEW_FILE => ClickAction::SidebarNewFile,
            CTX_SB_NEW_FOLDER => ClickAction::SidebarNewFolder,
            CTX_SB_RENAME => ClickAction::SidebarRename,
            CTX_SB_DELETE => ClickAction::SidebarDelete,
            CTX_SB_OPEN_CODE => ClickAction::SidebarOpenCode,
            id if id >= CTX_TAB_DOT_BASE => {
                ClickAction::SelectTab((id - CTX_TAB_DOT_BASE) as usize)
            }
            _ => ClickAction::None,
        },
        MenuEvent::Toggled { id, .. } => match *id {
            _ => ClickAction::None,
        },
        MenuEvent::SliderChanged { id, .. } => match *id {
            MENU_FONT_SLIDER => ClickAction::SliderDrag,
            _ => ClickAction::None,
        },
        _ => ClickAction::None,
    }
}

/// Quick hit-test: is (x) on the "Files" menu label? Used by events.rs to
/// intercept the click before InteractionContext sees it.
pub fn is_files_label_hit(x: f32, _mode: &WindowMode, scale: f32) -> bool {
    let font = MENU_FONT_BODY * scale;
    let pad_h = MENU_LABEL_PAD_H * scale;
    let ml = MENU_LEFT * scale + pad_h * 0.5;
    // "Files" is the first menu label
    let tw = 5.0 * font * 0.52; // "Files".len() = 5
    let w = tw + pad_h * 2.0;
    x >= ml && x <= ml + w
}

/// Get the new font size from a slider event value.
pub fn font_size_from_slider(value: f32) -> f32 {
    let raw = 6.0 + value * 24.0;
    (raw * 2.0).round() / 2.0
}

