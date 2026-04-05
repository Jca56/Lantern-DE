use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, GradientStrip, InteractionContext, MenuBar,
    MenuEvent, MenuItem, TitleBar,
};

use crate::config::WindowMode;
use crate::night_sky;

// ── Constants ───────────────────────────────────────────────────────────────

pub const TITLE_BAR_HEIGHT: f32 = 54.0;
pub const GRADIENT_STRIP_HEIGHT: f32 = 4.0;

/// Title bar height for the given window mode.
pub fn title_bar_height(mode: &WindowMode) -> f32 {
    match mode {
        WindowMode::Fox => TITLE_BAR_HEIGHT,
        WindowMode::NightSky => night_sky::TITLE_BAR_HEIGHT,
    }
}

// Zone IDs for title bar buttons
const ZONE_CLOSE: u32 = 10;
const ZONE_MAXIMIZE: u32 = 11;
const ZONE_MINIMIZE: u32 = 12;

// ── Menu action IDs ─────────────────────────────────────────────────────────

pub const MENU_FONT_SLIDER: u32 = 100;
pub const MENU_OPACITY_SLIDER: u32 = 101;
pub const MENU_SPLIT_RIGHT: u32 = 200;
pub const MENU_SPLIT_DOWN: u32 = 201;
pub const MENU_CLOSE_PANE: u32 = 202;
pub const MENU_PREV_PANE: u32 = 203;
pub const MENU_NEXT_PANE: u32 = 204;
pub const MENU_TOGGLE_SIDEBAR: u32 = 300;

// Context menu (right-click)
pub const CTX_COPY: u32 = 400;
pub const CTX_PASTE: u32 = 401;
pub const CTX_SELECT_ALL: u32 = 402;

// ── State ───────────────────────────────────────────────────────────────────

pub struct ChromeState {
    pub menu_bar: MenuBar,
    pub context_menu: ContextMenu,
    pub palette: FoxPalette,
}

impl ChromeState {
    pub fn new() -> Self {
        let palette = FoxPalette::dark();
        Self {
            menu_bar: MenuBar::new(&palette),
            context_menu: ContextMenu::new(ContextMenuStyle::from_palette(&palette)),
            palette,
        }
    }

    pub fn has_overlay(&self) -> bool {
        self.menu_bar.is_open() || self.context_menu.is_open()
    }

    pub fn close_all_menus(&mut self) {
        self.menu_bar.close();
        self.context_menu.close();
    }
}

// ── Click actions (kept for event dispatch compatibility) ───────────────────

pub enum ClickAction {
    None,
    Close,
    Minimize,
    Maximize,
    StartDrag,
    SliderDrag,
    OpacitySliderDrag,
    SplitHorizontal,
    SplitVertical,
    ClosePane,
    FocusPrevPane,
    FocusNextPane,
    ToggleSidebar,
    Copy,
    Paste,
    SelectAll,
}

// ── Menu definitions ────────────────────────────────────────────────────────

pub fn build_menus(font_size: f32, opacity: f32, sidebar_visible: bool) -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        ("View", vec![
            MenuItem::slider(MENU_FONT_SLIDER, "Text Size", ((font_size - 6.0) / 24.0).clamp(0.0, 1.0)),
            MenuItem::separator(),
            MenuItem::slider(MENU_OPACITY_SLIDER, "Opacity", ((opacity - 0.1) / 0.9).clamp(0.0, 1.0)),
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
        ("Files", vec![
            MenuItem::toggle(MENU_TOGGLE_SIDEBAR, "Show Sidebar", sidebar_visible),
        ]),
    ]
}

pub fn build_context_menu(has_selection: bool) -> Vec<MenuItem> {
    let copy = if has_selection {
        MenuItem::action_with(CTX_COPY, "Copy", "Ctrl+Shift+C")
    } else {
        MenuItem::action_disabled(CTX_COPY, "Copy")
    };
    vec![
        copy,
        MenuItem::action_with(CTX_PASTE, "Paste", "Ctrl+Shift+V"),
        MenuItem::action(CTX_SELECT_ALL, "Select All"),
    ]
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Draw the title bar, menu labels, and gradient strip (base layer).
pub fn draw_chrome(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &mut ChromeState,
    input: &mut InteractionContext,
    screen_w: u32,
    screen_h: u32,
    font_size: f32,
    opacity: f32,
    sidebar_visible: bool,
    maximized: bool,
    scale: f32,
    mode: &WindowMode,
    cursor_pos: Option<(f32, f32)>,
) {
    let s = scale;
    let pal = &state.palette;
    let wf = screen_w as f32;

    match mode {
        WindowMode::Fox => {
            // ── Fox: TitleBar widget + gradient strip ─────────────
            let bar_h = (TITLE_BAR_HEIGHT - GRADIENT_STRIP_HEIGHT) * s;
            let title_rect = Rect::new(0.0, 0.0, wf, bar_h);
            let tb = TitleBar::new(title_rect).scale(s);

            let close_state = input.add_zone(ZONE_CLOSE, tb.close_button_rect());
            let max_state = input.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
            let min_state = input.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());
            let content = tb.content_rect();

            tb.close_hovered(close_state.is_hovered())
                .maximize_hovered(max_state.is_hovered())
                .minimize_hovered(min_state.is_hovered())
                .maximized(maximized)
                .draw(painter, pal);

            // Menu bar inside title bar content area
            let menus = build_menus(font_size, opacity, sidebar_visible);
            state.menu_bar.update(input, &menus, content, s);
            let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
            state.menu_bar.draw_with_labels(painter, text, pal, &labels, screen_w, screen_h, s);

            // Gradient strip below title bar
            let strip_y = bar_h;
            let mut strip = GradientStrip::new(0.0, strip_y, wf);
            strip.height = GRADIENT_STRIP_HEIGHT * s;
            strip.draw(painter);
        }
        WindowMode::NightSky => {
            // ── Night Sky: custom controls + menu bar ─────────────
            let bar_h = night_sky::TITLE_BAR_HEIGHT;
            night_sky::draw_controls(painter, cursor_pos, wf);

            // Menu bar positioned in the left side of the title bar
            let content = Rect::new(8.0, 0.0, wf - 120.0, bar_h);
            let menus = build_menus(font_size, opacity, sidebar_visible);
            state.menu_bar.update(input, &menus, content, s);
            let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
            state.menu_bar.draw_with_labels(painter, text, pal, &labels, screen_w, screen_h, s);
        }
    }
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

    // Menu bar label click (open/close/switch)
    if state.menu_bar.on_click(input, menus, scale) {
        return ClickAction::None;
    }

    // Window control buttons
    match mode {
        WindowMode::Fox => {
            match input.active_zone_id() {
                Some(id) if id == ZONE_CLOSE => return ClickAction::Close,
                Some(id) if id == ZONE_MAXIMIZE => return ClickAction::Maximize,
                Some(id) if id == ZONE_MINIMIZE => return ClickAction::Minimize,
                _ => {}
            }
        }
        WindowMode::NightSky => {
            if let Some(zone) = night_sky::hit_test_controls((x, y), screen_w) {
                return match zone {
                    10 => ClickAction::Close,
                    11 => ClickAction::Maximize,
                    12 => ClickAction::Minimize,
                    _ => ClickAction::None,
                };
            }
        }
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
            CTX_COPY => ClickAction::Copy,
            CTX_PASTE => ClickAction::Paste,
            CTX_SELECT_ALL => ClickAction::SelectAll,
            _ => ClickAction::None,
        },
        MenuEvent::Toggled { id, .. } => match *id {
            MENU_TOGGLE_SIDEBAR => ClickAction::ToggleSidebar,
            _ => ClickAction::None,
        },
        MenuEvent::SliderChanged { id, .. } => match *id {
            MENU_FONT_SLIDER => ClickAction::SliderDrag,
            MENU_OPACITY_SLIDER => ClickAction::OpacitySliderDrag,
            _ => ClickAction::None,
        },
        _ => ClickAction::None,
    }
}

/// Get the new font size from a slider event value.
pub fn font_size_from_slider(value: f32) -> f32 {
    let raw = 6.0 + value * 24.0;
    (raw * 2.0).round() / 2.0
}

/// Get the new opacity from a slider event value.
pub fn opacity_from_slider(value: f32) -> f32 {
    let raw = 0.1 + value * 0.9;
    (raw * 20.0).round() / 20.0
}
