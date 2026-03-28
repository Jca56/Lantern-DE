use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::claude_menu::CATEGORIES as CLAUDE_CATEGORIES;
use crate::help_menu::{HelpCategory, CATEGORIES};
use crate::terminal::Color8;

// ── Palette (FoxPalette dark) ────────────────────────────────────────────────

const SURFACE: Color8 = Color8::from_rgb(39, 39, 39);
const TEXT_COLOR: Color8 = Color8::from_rgb(236, 236, 236);
const MUTED: Color8 = Color8::from_rgb(144, 144, 144);
const ACCENT: Color8 = Color8::from_rgb(200, 134, 10);

pub const TITLE_BAR_HEIGHT: f32 = 54.0;
const GRADIENT_STRIP_HEIGHT: f32 = 4.0;
const USABLE_HEIGHT: f32 = TITLE_BAR_HEIGHT - GRADIENT_STRIP_HEIGHT;

// ── Window control button constants (VS Code style) ─────────────────────────

const CONTROL_BTN_WIDTH: f32 = 46.0;
const CONTROL_BTN_HEIGHT: f32 = USABLE_HEIGHT;

const TAB_FONT_SIZE: f32 = 20.0;

// ── Help menu layout ────────────────────────────────────────────────────────

const HELP_CAT_WIDTH: f32 = 260.0;
const HELP_CMD_WIDTH: f32 = 420.0;
const HELP_CAT_HEIGHT: f32 = 40.0;
const HELP_CMD_HEIGHT: f32 = 34.0;
const HELP_FONT: f32 = 18.0;
const HELP_CMD_FONT: f32 = 16.0;
const HELP_DESC_FONT: f32 = 14.0;

// ── Gradient strip colors ───────────────────────────────────────────────────

const GRADIENT_COLORS: [Color8; 5] = [
    Color8::from_rgb(255, 105, 180),
    Color8::from_rgb(59, 130, 246),
    Color8::from_rgb(34, 197, 94),
    Color8::from_rgb(250, 204, 21),
    Color8::from_rgb(239, 68, 68),
];

// ── State ────────────────────────────────────────────────────────────────────

pub struct ChromeState {
    pub view_menu_open: bool,
    pub slider_dragging: bool,
    pub opacity_slider_dragging: bool,
    pub help_menu_open: bool,
    pub help_expanded: Option<usize>,
    pub split_menu_open: bool,
    pub claude_menu_open: bool,
    pub claude_expanded: Option<usize>,
    pub context_menu: Option<(f32, f32)>,
}

impl ChromeState {
    pub fn new() -> Self {
        Self {
            view_menu_open: false,
            slider_dragging: false,
            opacity_slider_dragging: false,
            help_menu_open: false,
            help_expanded: None,
            split_menu_open: false,
            claude_menu_open: false,
            claude_expanded: None,
            context_menu: None,
        }
    }

    pub fn has_overlay(&self) -> bool {
        self.view_menu_open
            || self.help_menu_open
            || self.split_menu_open
            || self.claude_menu_open
            || self.context_menu.is_some()
    }

    pub fn close_all_menus(&mut self) {
        self.view_menu_open = false;
        self.help_menu_open = false;
        self.help_expanded = None;
        self.split_menu_open = false;
        self.claude_menu_open = false;
        self.claude_expanded = None;
        self.context_menu = None;
    }
}

// ── Click actions ────────────────────────────────────────────────────────────

pub enum ClickAction {
    None,
    Close,
    Minimize,
    Maximize,
    StartDrag,
    SliderDrag,
    OpacitySliderDrag,
    RunCommand(String),
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

// ── Layout helpers ───────────────────────────────────────────────────────────

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

fn close_rect(screen_w: u32) -> Rect {
    let x = screen_w as f32 - CONTROL_BTN_WIDTH;
    Rect::new(x, 0.0, CONTROL_BTN_WIDTH, CONTROL_BTN_HEIGHT)
}

fn maximize_rect(screen_w: u32) -> Rect {
    let x = screen_w as f32 - CONTROL_BTN_WIDTH * 2.0;
    Rect::new(x, 0.0, CONTROL_BTN_WIDTH, CONTROL_BTN_HEIGHT)
}

fn minimize_rect(screen_w: u32) -> Rect {
    let x = screen_w as f32 - CONTROL_BTN_WIDTH * 3.0;
    Rect::new(x, 0.0, CONTROL_BTN_WIDTH, CONTROL_BTN_HEIGHT)
}

fn view_button_rect() -> Rect {
    Rect::new(14.0, 0.0, 56.0, USABLE_HEIGHT)
}

fn help_button_rect() -> Rect {
    let vb = view_button_rect();
    Rect::new(vb.x + vb.w + 6.0, 0.0, 56.0, USABLE_HEIGHT)
}

fn claude_button_rect() -> Rect {
    let hb = help_button_rect();
    Rect::new(hb.x + hb.w + 6.0, 0.0, 74.0, USABLE_HEIGHT)
}

fn files_button_rect() -> Rect {
    let sb = split_button_rect();
    Rect::new(sb.x + sb.w + 6.0, 0.0, 62.0, USABLE_HEIGHT)
}

fn split_button_rect() -> Rect {
    let cb = claude_button_rect();
    Rect::new(cb.x + cb.w + 6.0, 0.0, 66.0, USABLE_HEIGHT)
}

const SPLIT_MENU_ITEM_HEIGHT: f32 = 38.0;

fn split_menu_rect() -> Rect {
    let sb = split_button_rect();
    Rect::new(
        sb.x,
        TITLE_BAR_HEIGHT + 2.0,
        280.0,
        12.0 + 5.0 * SPLIT_MENU_ITEM_HEIGHT + 12.0,
    )
}

fn view_menu_rect() -> Rect {
    let vb = view_button_rect();
    Rect::new(vb.x, TITLE_BAR_HEIGHT + 2.0, 280.0, 170.0)
}

pub fn slider_rect(_screen_w: u32) -> Rect {
    let menu = view_menu_rect();
    Rect::new(menu.x + 16.0, menu.y + 48.0, menu.w - 32.0, 20.0)
}

pub fn opacity_slider_rect(_screen_w: u32) -> Rect {
    let menu = view_menu_rect();
    Rect::new(menu.x + 16.0, menu.y + 124.0, menu.w - 32.0, 20.0)
}

fn help_cat_rect() -> Rect {
    let hb = help_button_rect();
    let cat_count = CATEGORIES.len();
    let h = 12.0 + cat_count as f32 * HELP_CAT_HEIGHT + 12.0;
    Rect::new(hb.x, TITLE_BAR_HEIGHT + 2.0, HELP_CAT_WIDTH, h)
}

fn claude_cat_rect() -> Rect {
    let cb = claude_button_rect();
    let cat_count = CLAUDE_CATEGORIES.len();
    let h = 12.0 + cat_count as f32 * HELP_CAT_HEIGHT + 12.0;
    Rect::new(cb.x, TITLE_BAR_HEIGHT + 2.0, HELP_CAT_WIDTH, h)
}

/// Full bounding rect for a two-panel category menu (covers both panels).
fn category_menu_full_rect(
    categories: &[HelpCategory],
    cat_panel: Rect,
    cmd_panel_w: f32,
    expanded: Option<usize>,
) -> Rect {
    if let Some(idx) = expanded {
        let cmd_count = categories[idx].commands.len();
        let header_h = 10.0 + HELP_FONT + 16.0;
        let h = header_h + cmd_count as f32 * HELP_CMD_HEIGHT + 12.0;
        let right = cat_panel.x + cat_panel.w + 4.0 + cmd_panel_w;
        let bottom = cat_panel.y + cat_panel.h.max(h);
        Rect::new(
            cat_panel.x,
            cat_panel.y,
            right - cat_panel.x,
            bottom - cat_panel.y,
        )
    } else {
        cat_panel
    }
}

fn hit(rect: Rect, pos: Option<(f32, f32)>) -> bool {
    if let Some((x, y)) = pos {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    } else {
        false
    }
}

// ── Drawing (base layer — no menus) ─────────────────────────────────────────

pub fn draw_chrome(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ChromeState,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    sidebar_visible: bool,
) {
    let w = screen_w as f32;

    // Gradient strip
    draw_gradient_strip(painter, w);

    // "View" button
    let vb = view_button_rect();
    if hit(vb, cursor_pos) || state.view_menu_open {
        painter.rect_filled(vb, 0.0, c(Color8::from_rgba(255, 255, 255, 20)));
    }
    let btn_text_y = vb.y + (vb.h - TAB_FONT_SIZE) / 2.0;
    text.queue(
        "View",
        TAB_FONT_SIZE,
        vb.x + 8.0,
        btn_text_y,
        c(TEXT_COLOR),
        50.0,
        screen_w,
        screen_h,
    );

    // "Help" button
    let hb = help_button_rect();
    if hit(hb, cursor_pos) || state.help_menu_open {
        painter.rect_filled(hb, 0.0, c(Color8::from_rgba(255, 255, 255, 20)));
    }
    text.queue(
        "Help",
        TAB_FONT_SIZE,
        hb.x + 8.0,
        btn_text_y,
        c(TEXT_COLOR),
        50.0,
        screen_w,
        screen_h,
    );

    // "Claude" button
    let cb = claude_button_rect();
    if hit(cb, cursor_pos) || state.claude_menu_open {
        painter.rect_filled(cb, 0.0, c(Color8::from_rgba(255, 255, 255, 20)));
    }
    text.queue(
        "Claude",
        TAB_FONT_SIZE,
        cb.x + 4.0,
        btn_text_y,
        c(TEXT_COLOR),
        70.0,
        screen_w,
        screen_h,
    );

    // "Split" button
    let sb = split_button_rect();
    if hit(sb, cursor_pos) || state.split_menu_open {
        painter.rect_filled(sb, 0.0, c(Color8::from_rgba(255, 255, 255, 20)));
    }
    text.queue(
        "Split",
        TAB_FONT_SIZE,
        sb.x + 4.0,
        btn_text_y,
        c(TEXT_COLOR),
        62.0,
        screen_w,
        screen_h,
    );

    // "Files" button
    let fb = files_button_rect();
    if hit(fb, cursor_pos) || sidebar_visible {
        painter.rect_filled(fb, 0.0, c(Color8::from_rgba(255, 255, 255, 20)));
    }
    let files_color = if sidebar_visible { c(ACCENT) } else { c(TEXT_COLOR) };
    text.queue(
        "Files",
        TAB_FONT_SIZE,
        fb.x + 4.0,
        btn_text_y,
        files_color,
        58.0,
        screen_w,
        screen_h,
    );

    // Window controls
    draw_close_button(painter, close_rect(screen_w), cursor_pos);
    draw_maximize_button(painter, maximize_rect(screen_w), cursor_pos);
    draw_minimize_button(painter, minimize_rect(screen_w), cursor_pos);
}

/// Draw menu overlays (called in second render pass so menus appear above terminal text).
pub fn draw_overlay(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ChromeState,
    font_size: f32,
    opacity: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    has_selection: bool,
) {
    if state.view_menu_open {
        draw_view_menu(painter, text, font_size, opacity, screen_w, screen_h);
    }
    if state.help_menu_open {
        draw_help_menu(
            painter,
            text,
            state.help_expanded,
            screen_w,
            screen_h,
            cursor_pos,
        );
    }
    if state.claude_menu_open {
        draw_claude_menu(
            painter,
            text,
            state.claude_expanded,
            screen_w,
            screen_h,
            cursor_pos,
        );
    }
    if state.split_menu_open {
        draw_split_menu(painter, text, screen_w, screen_h, cursor_pos);
    }
    if let Some(origin) = state.context_menu {
        draw_context_menu(
            painter,
            text,
            origin,
            screen_w,
            screen_h,
            cursor_pos,
            has_selection,
        );
    }
}

fn draw_gradient_strip(painter: &mut Painter, width: f32) {
    let strip_y = TITLE_BAR_HEIGHT - GRADIENT_STRIP_HEIGHT;
    let steps = width.ceil() as usize;
    for i in 0..steps {
        let t = i as f32 / width;
        let color = sample_gradient(t);
        painter.rect_filled(
            Rect::new(i as f32, strip_y, 1.5, GRADIENT_STRIP_HEIGHT),
            0.0,
            color,
        );
    }
}

fn sample_gradient(t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let n = GRADIENT_COLORS.len() - 1;
    let segment = t * n as f32;
    let idx = (segment as usize).min(n - 1);
    let local_t = (segment - idx as f32).clamp(0.0, 1.0);
    let a = c(GRADIENT_COLORS[idx]);
    let b = c(GRADIENT_COLORS[idx + 1]);
    Color::rgba(
        a.r + (b.r - a.r) * local_t,
        a.g + (b.g - a.g) * local_t,
        a.b + (b.b - a.b) * local_t,
        1.0,
    )
}

fn draw_close_button(painter: &mut Painter, rect: Rect, cursor_pos: Option<(f32, f32)>) {
    let hovered = hit(rect, cursor_pos);
    if hovered {
        painter.rect_filled(rect, 0.0, c(Color8::from_rgba(232, 50, 50, 255)));
    }
    let icon_size = 14.0;
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let half = icon_size / 2.0;
    let lc = if hovered {
        c(Color8::from_rgb(255, 255, 255))
    } else {
        c(Color8::from_rgba(236, 236, 236, 200))
    };
    let steps = 12;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = cx - half + t * icon_size;
        let y1 = cy - half + t * icon_size;
        let y2 = cy + half - t * icon_size;
        painter.rect_filled(Rect::new(x - 0.75, y1 - 0.75, 1.5, 1.5), 0.0, lc);
        painter.rect_filled(Rect::new(x - 0.75, y2 - 0.75, 1.5, 1.5), 0.0, lc);
    }
}

fn draw_maximize_button(painter: &mut Painter, rect: Rect, cursor_pos: Option<(f32, f32)>) {
    if hit(rect, cursor_pos) {
        painter.rect_filled(rect, 0.0, c(Color8::from_rgba(255, 255, 255, 30)));
    }
    let s = 14.0;
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    let h = s / 2.0;
    let lc = c(Color8::from_rgba(236, 236, 236, 200));
    let t = 1.5;
    painter.rect_filled(Rect::new(cx - h, cy - h, s, t), 0.0, lc);
    painter.rect_filled(Rect::new(cx - h, cy + h - t, s, t), 0.0, lc);
    painter.rect_filled(Rect::new(cx - h, cy - h, t, s), 0.0, lc);
    painter.rect_filled(Rect::new(cx + h - t, cy - h, t, s), 0.0, lc);
}

fn draw_minimize_button(painter: &mut Painter, rect: Rect, cursor_pos: Option<(f32, f32)>) {
    if hit(rect, cursor_pos) {
        painter.rect_filled(rect, 0.0, c(Color8::from_rgba(255, 255, 255, 30)));
    }
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    painter.rect_filled(
        Rect::new(cx - 7.0, cy - 0.75, 14.0, 1.5),
        0.0,
        c(Color8::from_rgba(236, 236, 236, 200)),
    );
}

// ── View menu overlay ───────────────────────────────────────────────────────

fn draw_view_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    font_size: f32,
    opacity: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let menu = view_menu_rect();
    draw_menu_bg(painter, menu);

    let label_font = 17.0;
    let value_font = 17.0;

    text.queue(
        "Text Size",
        label_font,
        menu.x + 16.0,
        menu.y + 14.0,
        c(MUTED),
        160.0,
        screen_w,
        screen_h,
    );
    let size_str = format!("{:.0}", font_size);
    text.queue(
        &size_str,
        value_font,
        menu.x + menu.w - 44.0,
        menu.y + 14.0,
        c(TEXT_COLOR),
        50.0,
        screen_w,
        screen_h,
    );

    let sr = slider_rect(screen_w);
    draw_slider(painter, sr, ((font_size - 6.0) / 24.0).clamp(0.0, 1.0));

    painter.rect_filled(
        Rect::new(menu.x + 16.0, menu.y + 78.0, menu.w - 32.0, 1.0),
        0.0,
        c(Color8::from_rgba(255, 255, 255, 15)),
    );

    text.queue(
        "Opacity",
        label_font,
        menu.x + 16.0,
        menu.y + 92.0,
        c(MUTED),
        160.0,
        screen_w,
        screen_h,
    );
    let opacity_str = format!("{}%", (opacity * 100.0).round() as u32);
    text.queue(
        &opacity_str,
        value_font,
        menu.x + menu.w - 54.0,
        menu.y + 92.0,
        c(TEXT_COLOR),
        60.0,
        screen_w,
        screen_h,
    );

    let osr = opacity_slider_rect(screen_w);
    draw_slider(painter, osr, ((opacity - 0.1) / 0.9).clamp(0.0, 1.0));
}

// ── Two-panel category menu (shared by Help & Claude) ────────────────────

fn draw_help_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    expanded: Option<usize>,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    draw_category_menu(
        painter,
        text,
        &CATEGORIES,
        help_cat_rect(),
        HELP_CMD_WIDTH,
        220.0,
        false,
        expanded,
        screen_w,
        screen_h,
        cursor_pos,
    );
}

fn draw_claude_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    expanded: Option<usize>,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    draw_category_menu(
        painter,
        text,
        &CLAUDE_CATEGORIES,
        claude_cat_rect(),
        HELP_CMD_WIDTH,
        220.0,
        true,
        expanded,
        screen_w,
        screen_h,
        cursor_pos,
    );
}

fn draw_category_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    categories: &[HelpCategory],
    cat_panel: Rect,
    cmd_panel_w: f32,
    cmd_col_w: f32,
    show_basename: bool,
    expanded: Option<usize>,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    // ── Left panel: categories ──────────────────────────────────────────
    draw_menu_bg(painter, cat_panel);

    let mut y = cat_panel.y + 8.0;
    for (cat_idx, cat) in categories.iter().enumerate() {
        let is_expanded = expanded == Some(cat_idx);
        let cat_rect = Rect::new(cat_panel.x, y, cat_panel.w, HELP_CAT_HEIGHT);
        let cat_hovered = hit(cat_rect, cursor_pos);

        if is_expanded {
            painter.rect_filled(
                Rect::new(cat_panel.x + 4.0, y, cat_panel.w - 8.0, HELP_CAT_HEIGHT),
                4.0,
                c(Color8::from_rgba(200, 134, 10, 35)),
            );
        } else if cat_hovered {
            painter.rect_filled(
                Rect::new(cat_panel.x + 4.0, y, cat_panel.w - 8.0, HELP_CAT_HEIGHT),
                4.0,
                c(Color8::from_rgba(255, 255, 255, 15)),
            );
        }

        let arrow = if is_expanded { "▸" } else { "▸" };
        let arrow_color = if is_expanded { c(ACCENT) } else { c(MUTED) };
        text.queue(
            arrow,
            HELP_FONT,
            cat_panel.x + 14.0,
            y + (HELP_CAT_HEIGHT - HELP_FONT) / 2.0,
            arrow_color,
            20.0,
            screen_w,
            screen_h,
        );

        let name_color = if is_expanded { c(TEXT_COLOR) } else { c(MUTED) };
        text.queue(
            cat.name,
            HELP_FONT,
            cat_panel.x + 36.0,
            y + (HELP_CAT_HEIGHT - HELP_FONT) / 2.0,
            name_color,
            cat_panel.w - 80.0,
            screen_w,
            screen_h,
        );

        let count_str = format!("{}", cat.commands.len());
        text.queue(
            &count_str,
            HELP_DESC_FONT,
            cat_panel.x + cat_panel.w - 36.0,
            y + (HELP_CAT_HEIGHT - HELP_DESC_FONT) / 2.0,
            c(MUTED),
            24.0,
            screen_w,
            screen_h,
        );

        y += HELP_CAT_HEIGHT;
    }

    // ── Right panel: commands (only when a category is selected) ────────
    if let Some(cat_idx) = expanded {
        let cat = &categories[cat_idx];
        let cmd_count = cat.commands.len();
        let header_h = 10.0 + HELP_FONT + 16.0;
        let h = header_h + cmd_count as f32 * HELP_CMD_HEIGHT + 12.0;
        let cmd_panel = Rect::new(cat_panel.x + cat_panel.w + 4.0, cat_panel.y, cmd_panel_w, h);
        draw_menu_bg(painter, cmd_panel);

        let title_y = cmd_panel.y + 10.0;
        text.queue(
            cat.name,
            HELP_FONT + 1.0,
            cmd_panel.x + 14.0,
            title_y,
            c(ACCENT),
            cmd_panel.w - 28.0,
            screen_w,
            screen_h,
        );

        painter.rect_filled(
            Rect::new(
                cmd_panel.x + 10.0,
                title_y + HELP_FONT + 8.0,
                cmd_panel.w - 20.0,
                1.0,
            ),
            0.0,
            c(Color8::from_rgba(255, 255, 255, 15)),
        );

        let mut cy = title_y + HELP_FONT + 16.0;
        let desc_x = cmd_panel.x + cmd_col_w + 20.0;
        let desc_max_w = cmd_panel.w - cmd_col_w - 40.0;
        for cmd in cat.commands {
            let cmd_rect = Rect::new(cmd_panel.x + 6.0, cy, cmd_panel.w - 12.0, HELP_CMD_HEIGHT);
            let cmd_hovered = hit(cmd_rect, cursor_pos);

            if cmd_hovered {
                painter.rect_filled(cmd_rect, 3.0, c(Color8::from_rgba(200, 134, 10, 30)));
            }

            let cmd_color = if cmd_hovered {
                c(ACCENT)
            } else {
                c(Color8::from_rgb(180, 220, 255))
            };
            let text_y = cy + (HELP_CMD_HEIGHT - HELP_CMD_FONT) / 2.0;
            let display = if show_basename {
                cmd.cmd.rsplit('/').next().unwrap_or(cmd.cmd)
            } else {
                cmd.cmd
            };
            text.queue(
                display,
                HELP_CMD_FONT,
                cmd_panel.x + 16.0,
                text_y,
                cmd_color,
                cmd_col_w,
                screen_w,
                screen_h,
            );
            text.queue(
                cmd.desc,
                HELP_DESC_FONT,
                desc_x,
                text_y + 1.0,
                c(MUTED),
                desc_max_w,
                screen_w,
                screen_h,
            );

            cy += HELP_CMD_HEIGHT;
        }
    }
}

// ── Split menu overlay ──────────────────────────────────────────────────────

fn draw_split_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let menu = split_menu_rect();
    draw_menu_bg(painter, menu);

    let items: &[(&str, &str)] = &[
        ("Split Right", "Ctrl+Shift+D"),
        ("Split Down", "Ctrl+Shift+E"),
        ("Close Pane", "Ctrl+Shift+W"),
        ("Prev Pane", "Ctrl+Shift+["),
        ("Next Pane", "Ctrl+Shift+]"),
    ];

    let mut y = menu.y + 8.0;
    for (label, shortcut) in items {
        let item_rect = Rect::new(menu.x + 4.0, y, menu.w - 8.0, SPLIT_MENU_ITEM_HEIGHT);
        let hovered = hit(item_rect, cursor_pos);

        if hovered {
            painter.rect_filled(item_rect, 4.0, c(Color8::from_rgba(255, 255, 255, 15)));
        }

        let text_y = y + (SPLIT_MENU_ITEM_HEIGHT - HELP_FONT) / 2.0;
        let label_color = if hovered { c(TEXT_COLOR) } else { c(MUTED) };
        text.queue(
            label,
            HELP_FONT,
            menu.x + 16.0,
            text_y,
            label_color,
            160.0,
            screen_w,
            screen_h,
        );

        let shortcut_y = y + (SPLIT_MENU_ITEM_HEIGHT - HELP_DESC_FONT) / 2.0;
        text.queue(
            shortcut,
            HELP_DESC_FONT,
            menu.x + menu.w - 120.0,
            shortcut_y,
            c(Color8::from_rgba(120, 120, 120, 255)),
            110.0,
            screen_w,
            screen_h,
        );

        y += SPLIT_MENU_ITEM_HEIGHT;
    }
}

// ── Context menu (right-click) ──────────────────────────────────────────────

const CTX_MENU_WIDTH: f32 = 200.0;
const CTX_ITEM_HEIGHT: f32 = 36.0;
const CTX_ITEMS: &[(&str, &str)] = &[
    ("Copy", "Ctrl+Shift+C"),
    ("Paste", "Ctrl+Shift+V"),
    ("Select All", ""),
];

fn context_menu_rect(origin: (f32, f32), screen_w: u32, screen_h: u32) -> Rect {
    let h = 12.0 + CTX_ITEMS.len() as f32 * CTX_ITEM_HEIGHT + 12.0;
    // Keep menu on screen
    let x = if origin.0 + CTX_MENU_WIDTH > screen_w as f32 {
        origin.0 - CTX_MENU_WIDTH
    } else {
        origin.0
    };
    let y = if origin.1 + h > screen_h as f32 {
        origin.1 - h
    } else {
        origin.1
    };
    Rect::new(x.max(0.0), y.max(0.0), CTX_MENU_WIDTH, h)
}

fn draw_context_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    origin: (f32, f32),
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    has_selection: bool,
) {
    let menu = context_menu_rect(origin, screen_w, screen_h);
    draw_menu_bg(painter, menu);

    let mut y = menu.y + 8.0;
    for (label, shortcut) in CTX_ITEMS {
        let item_rect = Rect::new(menu.x + 4.0, y, menu.w - 8.0, CTX_ITEM_HEIGHT);
        let hovered = hit(item_rect, cursor_pos);
        let is_copy = *label == "Copy";
        let dimmed = is_copy && !has_selection;

        if hovered && !dimmed {
            painter.rect_filled(item_rect, 4.0, c(Color8::from_rgba(255, 255, 255, 15)));
        }

        let text_y = y + (CTX_ITEM_HEIGHT - HELP_FONT) / 2.0;
        let label_color = if dimmed {
            c(Color8::from_rgba(100, 100, 100, 255))
        } else if hovered {
            c(TEXT_COLOR)
        } else {
            c(MUTED)
        };
        text.queue(
            label,
            HELP_FONT,
            menu.x + 16.0,
            text_y,
            label_color,
            120.0,
            screen_w,
            screen_h,
        );

        if !shortcut.is_empty() {
            let sc_y = y + (CTX_ITEM_HEIGHT - HELP_DESC_FONT) / 2.0;
            text.queue(
                shortcut,
                HELP_DESC_FONT,
                menu.x + menu.w - 110.0,
                sc_y,
                c(Color8::from_rgba(100, 100, 100, 255)),
                100.0,
                screen_w,
                screen_h,
            );
        }

        y += CTX_ITEM_HEIGHT;
    }
}

fn context_menu_click(
    origin: (f32, f32),
    cursor_pos: Option<(f32, f32)>,
    screen_w: u32,
    screen_h: u32,
) -> ClickAction {
    let menu = context_menu_rect(origin, screen_w, screen_h);
    let mut y = menu.y + 8.0;

    let actions = [
        ClickAction::Copy,
        ClickAction::Paste,
        ClickAction::SelectAll,
    ];

    for action in actions {
        let item_rect = Rect::new(menu.x + 4.0, y, menu.w - 8.0, CTX_ITEM_HEIGHT);
        if hit(item_rect, cursor_pos) {
            return action;
        }
        y += CTX_ITEM_HEIGHT;
    }

    ClickAction::None
}

// ── Shared menu drawing ─────────────────────────────────────────────────────

fn draw_menu_bg(painter: &mut Painter, menu: Rect) {
    // Shadow
    painter.rect_filled(
        Rect::new(menu.x + 2.0, menu.y + 2.0, menu.w, menu.h),
        6.0,
        c(Color8::from_rgba(0, 0, 0, 60)),
    );
    // Background
    painter.rect_filled(menu, 6.0, c(SURFACE));
    // Top highlight
    painter.rect_filled(
        Rect::new(menu.x + 3.0, menu.y, menu.w - 6.0, 1.0),
        0.0,
        c(Color8::from_rgba(255, 255, 255, 15)),
    );
}

fn draw_slider(painter: &mut Painter, sr: Rect, t: f32) {
    let track_h = 6.0;
    let track_y = sr.y + (sr.h - track_h) / 2.0;
    painter.rect_filled(
        Rect::new(sr.x, track_y, sr.w, track_h),
        3.0,
        c(Color8::from_rgba(51, 51, 51, 242)),
    );

    let fill_w = (t * sr.w).max(0.0);
    if fill_w > 0.5 {
        painter.rect_filled(Rect::new(sr.x, track_y, fill_w, track_h), 3.0, c(ACCENT));
    }

    let thumb_x = sr.x + fill_w;
    let thumb_y = sr.y + sr.h / 2.0;
    painter.circle_filled(
        thumb_x,
        thumb_y,
        10.0,
        c(Color8::from_rgba(250, 204, 21, 56)),
    );
    painter.circle_filled(thumb_x, thumb_y, 7.0, c(Color8::from_rgb(240, 240, 240)));
    painter.circle_stroke(
        thumb_x,
        thumb_y,
        7.0,
        1.0,
        c(Color8::from_rgba(0, 0, 0, 30)),
    );
}

// ── Hit testing ──────────────────────────────────────────────────────────────

pub fn handle_click(
    state: &mut ChromeState,
    cursor_pos: Option<(f32, f32)>,
    _font_size: f32,
    screen_w: u32,
    screen_h: u32,
) -> ClickAction {
    let pos = match cursor_pos {
        Some(p) => p,
        None => return ClickAction::None,
    };

    // Context menu takes priority
    if let Some(origin) = state.context_menu {
        let menu = context_menu_rect(origin, screen_w, screen_h);
        if hit(menu, cursor_pos) {
            let action = context_menu_click(origin, cursor_pos, screen_w, screen_h);
            state.context_menu = None;
            return action;
        }
        state.context_menu = None;
        return ClickAction::None;
    }

    if hit(close_rect(screen_w), cursor_pos) {
        return ClickAction::Close;
    }
    if hit(maximize_rect(screen_w), cursor_pos) {
        return ClickAction::Maximize;
    }
    if hit(minimize_rect(screen_w), cursor_pos) {
        return ClickAction::Minimize;
    }

    if hit(view_button_rect(), cursor_pos) {
        let opening = !state.view_menu_open;
        state.close_all_menus();
        state.view_menu_open = opening;
        return ClickAction::None;
    }

    if hit(help_button_rect(), cursor_pos) {
        let opening = !state.help_menu_open;
        state.close_all_menus();
        state.help_menu_open = opening;
        return ClickAction::None;
    }

    if hit(claude_button_rect(), cursor_pos) {
        let opening = !state.claude_menu_open;
        state.close_all_menus();
        state.claude_menu_open = opening;
        return ClickAction::None;
    }

    if hit(split_button_rect(), cursor_pos) {
        let opening = !state.split_menu_open;
        state.close_all_menus();
        state.split_menu_open = opening;
        return ClickAction::None;
    }

    if hit(files_button_rect(), cursor_pos) {
        state.close_all_menus();
        return ClickAction::ToggleSidebar;
    }

    // View menu interactions
    if state.view_menu_open {
        let sr = slider_rect(screen_w);
        let expanded = Rect::new(sr.x - 8.0, sr.y - 4.0, sr.w + 16.0, sr.h + 8.0);
        if hit(expanded, cursor_pos) {
            state.slider_dragging = true;
            return ClickAction::SliderDrag;
        }

        let osr = opacity_slider_rect(screen_w);
        let osr_expanded = Rect::new(osr.x - 8.0, osr.y - 4.0, osr.w + 16.0, osr.h + 8.0);
        if hit(osr_expanded, cursor_pos) {
            state.opacity_slider_dragging = true;
            return ClickAction::OpacitySliderDrag;
        }

        if hit(view_menu_rect(), cursor_pos) {
            return ClickAction::None;
        }

        state.view_menu_open = false;
    }

    // Help menu interactions
    if state.help_menu_open {
        let menu = category_menu_full_rect(
            &CATEGORIES,
            help_cat_rect(),
            HELP_CMD_WIDTH,
            state.help_expanded,
        );
        if hit(menu, cursor_pos) {
            return help_menu_click(state, cursor_pos);
        }
        state.help_menu_open = false;
        state.help_expanded = None;
    }

    // Claude menu interactions
    if state.claude_menu_open {
        let menu = category_menu_full_rect(
            &CLAUDE_CATEGORIES,
            claude_cat_rect(),
            HELP_CMD_WIDTH,
            state.claude_expanded,
        );
        if hit(menu, cursor_pos) {
            return claude_menu_click(state, cursor_pos);
        }
        state.claude_menu_open = false;
        state.claude_expanded = None;
    }

    // Split menu interactions
    if state.split_menu_open {
        let menu = split_menu_rect();
        if hit(menu, cursor_pos) {
            return split_menu_click(state, cursor_pos);
        }
        state.split_menu_open = false;
    }

    // Title bar drag
    if pos.1 <= TITLE_BAR_HEIGHT {
        return ClickAction::StartDrag;
    }

    ClickAction::None
}

fn help_menu_click(state: &mut ChromeState, cursor_pos: Option<(f32, f32)>) -> ClickAction {
    match category_menu_click(
        &CATEGORIES,
        help_cat_rect(),
        HELP_CMD_WIDTH,
        &mut state.help_expanded,
        cursor_pos,
    ) {
        Some(cmd) => {
            state.help_menu_open = false;
            ClickAction::RunCommand(cmd)
        }
        None => ClickAction::None,
    }
}

fn claude_menu_click(state: &mut ChromeState, cursor_pos: Option<(f32, f32)>) -> ClickAction {
    match category_menu_click(
        &CLAUDE_CATEGORIES,
        claude_cat_rect(),
        HELP_CMD_WIDTH,
        &mut state.claude_expanded,
        cursor_pos,
    ) {
        Some(cmd) => {
            state.claude_menu_open = false;
            ClickAction::RunCommand(cmd)
        }
        None => ClickAction::None,
    }
}

fn category_menu_click(
    categories: &[HelpCategory],
    cat_panel: Rect,
    cmd_panel_w: f32,
    expanded: &mut Option<usize>,
    cursor_pos: Option<(f32, f32)>,
) -> Option<String> {
    if hit(cat_panel, cursor_pos) {
        let mut y = cat_panel.y + 8.0;
        for (cat_idx, _) in categories.iter().enumerate() {
            let cat_rect = Rect::new(cat_panel.x, y, cat_panel.w, HELP_CAT_HEIGHT);
            if hit(cat_rect, cursor_pos) {
                if *expanded == Some(cat_idx) {
                    *expanded = None;
                } else {
                    *expanded = Some(cat_idx);
                }
                return None;
            }
            y += HELP_CAT_HEIGHT;
        }
        return None;
    }

    if let Some(cat_idx) = *expanded {
        let cat = &categories[cat_idx];
        let cmd_count = cat.commands.len();
        let header_h = 10.0 + HELP_FONT + 16.0;
        let h = header_h + cmd_count as f32 * HELP_CMD_HEIGHT + 12.0;
        let cmd_panel = Rect::new(cat_panel.x + cat_panel.w + 4.0, cat_panel.y, cmd_panel_w, h);

        if hit(cmd_panel, cursor_pos) {
            let mut cy = cmd_panel.y + 10.0 + HELP_FONT + 16.0;
            for cmd in cat.commands {
                let cmd_rect =
                    Rect::new(cmd_panel.x + 6.0, cy, cmd_panel.w - 12.0, HELP_CMD_HEIGHT);
                if hit(cmd_rect, cursor_pos) {
                    *expanded = None;
                    return Some(cmd.cmd.to_string());
                }
                cy += HELP_CMD_HEIGHT;
            }
            return None;
        }
    }

    None
}

fn split_menu_click(state: &mut ChromeState, cursor_pos: Option<(f32, f32)>) -> ClickAction {
    let menu = split_menu_rect();
    let mut y = menu.y + 8.0;

    let actions = [
        ClickAction::SplitHorizontal,
        ClickAction::SplitVertical,
        ClickAction::ClosePane,
        ClickAction::FocusPrevPane,
        ClickAction::FocusNextPane,
    ];

    for action in actions {
        let item_rect = Rect::new(menu.x + 4.0, y, menu.w - 8.0, SPLIT_MENU_ITEM_HEIGHT);
        if hit(item_rect, cursor_pos) {
            state.split_menu_open = false;
            return action;
        }
        y += SPLIT_MENU_ITEM_HEIGHT;
    }

    ClickAction::None
}
