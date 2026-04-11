use std::time::Instant;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::config::WindowMode;

// ── Constants ───────────────────────────────────────────────────────────────

const TAB_MAX_WIDTH: f32 = 500.0;
const TAB_MIN_WIDTH: f32 = 90.0;
const TAB_GAP: f32 = 6.0;
const TAB_PAD_H: f32 = 14.0;
const NEW_TAB_WIDTH: f32 = 48.0;
const TAB_CLOSE_SIZE: f32 = 24.0;
const TAB_FONT_SIZE: f32 = 22.0;
const PIN_WIDTH: f32 = 22.0;
const DOUBLE_CLICK_MS: u128 = 400;
const TAB_PAD_V: f32 = 8.0;
/// Internal left inset inside the tabs region so the first tab's rounded left
/// edge isn't clipped by `push_clip(bounds)`.
const TABS_INNER_LEFT: f32 = 6.0;

// ── Palettes ───────────────────────────────────────────────────────────────

struct TabPalette {
    surface: Color,
    tab_active: Color,
    tab_hover: Color,
    tab_inactive: Color,
    tab_rename: Color,
    text: Color,
    muted: Color,
    accent: Color,
    close_hover_bg: Color,
    close_hover_fg: Color,
    close_active_fg: Color,
    close_inactive_fg: Color,
    plus_bg: Color,
    plus_bg_hover: Color,
    plus_border: Color,
    plus_icon: Color,
    plus_icon_hover: Color,
    ctx_shadow: Color,
    ctx_bg: Color,
    ctx_highlight: Color,
    ctx_top_line: Color,
}

fn palette(mode: &WindowMode) -> TabPalette {
    match mode {
        WindowMode::Fox => TabPalette {
            surface: Color::from_rgba8(30, 30, 30, 255),
            tab_active: Color::from_rgba8(50, 50, 50, 255),
            tab_hover: Color::from_rgba8(45, 45, 45, 255),
            tab_inactive: Color::from_rgba8(35, 35, 35, 255),
            tab_rename: Color::from_rgba8(50, 50, 50, 255),
            text: Color::from_rgba8(236, 236, 236, 255),
            muted: Color::from_rgba8(144, 144, 144, 255),
            accent: Color::from_rgba8(200, 134, 10, 255),
            close_hover_bg: Color::from_rgba8(232, 50, 50, 40),
            close_hover_fg: Color::from_rgba8(232, 80, 80, 255),
            close_active_fg: Color::from_rgba8(180, 60, 60, 255),
            close_inactive_fg: Color::from_rgba8(120, 50, 50, 255),
            plus_bg: Color::from_rgba8(255, 255, 255, 12),
            plus_bg_hover: Color::from_rgba8(255, 255, 255, 35),
            plus_border: Color::from_rgba8(255, 255, 255, 40),
            plus_icon: Color::from_rgba8(190, 190, 190, 255),
            plus_icon_hover: Color::from_rgba8(236, 236, 236, 255),
            ctx_shadow: Color::from_rgba8(0, 0, 0, 60),
            ctx_bg: Color::from_rgba8(39, 39, 39, 255),
            ctx_highlight: Color::from_rgba8(255, 255, 255, 15),
            ctx_top_line: Color::from_rgba8(255, 255, 255, 15),
        },
        WindowMode::NightSky => TabPalette {
            surface: Color::rgba(0.005, 0.002, 0.020, 0.85),
            tab_active: Color::rgba(0.06, 0.03, 0.12, 0.9),
            tab_hover: Color::rgba(0.05, 0.025, 0.10, 0.8),
            tab_inactive: Color::rgba(0.025, 0.012, 0.06, 0.7),
            tab_rename: Color::rgba(0.06, 0.03, 0.12, 0.9),
            text: Color::rgb(0.80, 0.76, 0.90),
            muted: Color::rgb(0.45, 0.40, 0.58),
            accent: Color::from_rgba8(200, 134, 10, 255),
            close_hover_bg: Color::rgba(0.45, 0.02, 0.02, 0.35),
            close_hover_fg: Color::rgb(0.85, 0.25, 0.25),
            close_active_fg: Color::rgb(0.60, 0.18, 0.18),
            close_inactive_fg: Color::rgb(0.40, 0.15, 0.15),
            plus_bg: Color::rgba(0.30, 0.20, 0.50, 0.12),
            plus_bg_hover: Color::rgba(0.30, 0.20, 0.50, 0.30),
            plus_border: Color::rgba(0.30, 0.20, 0.50, 0.25),
            plus_icon: Color::rgb(0.55, 0.50, 0.68),
            plus_icon_hover: Color::rgb(0.80, 0.76, 0.90),
            ctx_shadow: Color::rgba(0.0, 0.0, 0.0, 0.35),
            ctx_bg: Color::rgba(0.04, 0.02, 0.08, 0.95),
            ctx_highlight: Color::rgba(0.30, 0.20, 0.50, 0.20),
            ctx_top_line: Color::rgba(0.30, 0.20, 0.50, 0.15),
        },
    }
}

// Tab context menu
const CTX_MENU_WIDTH: f32 = 180.0;
const CTX_ITEM_HEIGHT: f32 = 36.0;

// ── Public info struct ──────────────────────────────────────────────────────

pub struct TabDisplay<'a> {
    pub title: &'a str,
    pub pinned: bool,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct TabBarState {
    // Rename
    pub renaming: Option<usize>,
    pub rename_buf: String,
    pub rename_cursor: usize,

    // Drag reorder
    pub dragging: Option<usize>,
    pub drag_start_x: f32,
    pub drag_offset_x: f32,
    drag_committed: bool,

    // Double-click detection
    last_click_time: Instant,
    last_click_tab: Option<usize>,

    // Tab right-click context menu
    pub context_menu: Option<(usize, f32, f32)>,
}

impl TabBarState {
    pub fn new() -> Self {
        Self {
            renaming: None,
            rename_buf: String::new(),
            rename_cursor: 0,
            dragging: None,
            drag_start_x: 0.0,
            drag_offset_x: 0.0,
            drag_committed: false,
            last_click_time: Instant::now(),
            last_click_tab: None,
            context_menu: None,
        }
    }

    pub fn has_overlay(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn start_rename(&mut self, idx: usize, current_title: &str) {
        self.renaming = Some(idx);
        self.rename_buf = current_title.to_string();
        self.rename_cursor = current_title.len();
    }

    pub fn cancel_rename(&mut self) {
        self.renaming = None;
        self.rename_buf.clear();
    }
}

// ── Actions ─────────────────────────────────────────────────────────────────

pub enum TabBarAction {
    None,
    SwitchTab(usize),
    CloseTab(usize),
    NewTab,
    ConfirmRename(usize, String),
    TogglePin(usize),
    Reorder { from: usize, to: usize },
}

// ── Layout helpers ──────────────────────────────────────────────────────────

/// Per-char width used for tab title measurement. Uses the ceiling of
/// `font * 0.6` to match the monospace grid cell width (see `render.rs`).
const CHAR_W: f32 = 14.0; // ceil(22 * 0.6)

/// Compute widths for each tab based on title length, fitting within available space.
fn calc_tab_widths(titles: &[&str], available: f32) -> Vec<f32> {
    let n = titles.len().max(1);
    let space = available
        - TABS_INNER_LEFT
        - TABS_INNER_LEFT // symmetric right-side breathing room
        - NEW_TAB_WIDTH
        - 4.0
        - (n.saturating_sub(1) as f32 * TAB_GAP);

    // Ideal width per tab: pad + text + close + pad
    let ideal: Vec<f32> = titles
        .iter()
        .map(|t| {
            let text_w = t.chars().count() as f32 * CHAR_W;
            (TAB_PAD_H * 2.0 + text_w + TAB_CLOSE_SIZE).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH)
        })
        .collect();

    let total_ideal: f32 = ideal.iter().sum();
    if total_ideal <= space {
        // All fit at ideal size
        return ideal;
    }

    // Scale down proportionally, respecting min
    let scale = space / total_ideal;
    ideal
        .iter()
        .map(|&w| (w * scale).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH))
        .collect()
}

/// Compute tab rect using pre-calculated dynamic widths inside the given bounds.
fn tab_rect_dynamic(idx: usize, widths: &[f32], bounds: Rect) -> Rect {
    let mut x = bounds.x + TABS_INNER_LEFT;
    for i in 0..idx {
        x += widths[i] + TAB_GAP;
    }
    let w = widths.get(idx).copied().unwrap_or(TAB_MIN_WIDTH);
    Rect::new(x, bounds.y + TAB_PAD_V, w, bounds.h - TAB_PAD_V * 2.0)
}

/// Compute the new-tab "+" button rect placed after the last tab, inside bounds.
fn new_tab_rect(widths: &[f32], bounds: Rect) -> Rect {
    let last_end = if widths.is_empty() {
        bounds.x + TABS_INNER_LEFT
    } else {
        let last = tab_rect_dynamic(widths.len() - 1, widths, bounds);
        last.x + last.w + TAB_GAP
    };
    Rect::new(
        last_end + 4.0,
        bounds.y + TAB_PAD_V,
        NEW_TAB_WIDTH,
        bounds.h - TAB_PAD_V * 2.0,
    )
}

fn tab_close_rect(tab: Rect) -> Rect {
    let x = tab.x + tab.w - TAB_PAD_H - TAB_CLOSE_SIZE + 4.0;
    let y = tab.y + (tab.h - TAB_CLOSE_SIZE) / 2.0;
    Rect::new(x, y, TAB_CLOSE_SIZE, TAB_CLOSE_SIZE)
}

fn hit(rect: Rect, pos: Option<(f32, f32)>) -> bool {
    if let Some((x, y)) = pos {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    } else {
        false
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Draw the tabs row inside `bounds` (the region between the menu divider and
/// the window controls). Tabs and the "+" button are clipped to bounds so they
/// never overlap menus or window controls.
pub fn draw_tab_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &TabBarState,
    tabs: &[TabDisplay],
    active: usize,
    bounds: Rect,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    mode: &WindowMode,
) {
    if bounds.w <= 0.0 {
        return;
    }
    let tab_count = tabs.len();
    let pal = palette(mode);
    let _ = pal.surface; // bg now drawn by ui_chrome

    // Compute dynamic tab widths based on title lengths
    let titles: Vec<&str> = tabs.iter().map(|t| t.title).collect();
    let widths = calc_tab_widths(&titles, bounds.w);

    // Clip everything inside the tabs region so dragged or oversized tabs
    // don't bleed onto menus or window controls.
    painter.push_clip(bounds);

    for (i, tab) in tabs.iter().enumerate() {
        let mut rect = tab_rect_dynamic(i, &widths, bounds);

        // If dragging this tab, offset it
        if state.dragging == Some(i) && state.drag_committed {
            rect.x += state.drag_offset_x;
        }

        let is_active = i == active;
        let is_hovered = hit(rect, cursor_pos);
        let is_renaming = state.renaming == Some(i);

        // Tab background (pill shape)
        let pill_r = rect.h / 2.0;
        if is_renaming {
            painter.rect_filled(rect, pill_r, pal.tab_rename);
            // Gold border for rename mode
            painter.rect_stroke_sdf(rect, pill_r, 2.0, pal.accent);
        } else if is_active {
            painter.rect_filled(rect, pill_r, pal.tab_active);
            // Accent stroke around active pill
            painter.rect_stroke_sdf(rect, pill_r, 1.5, pal.accent);
        } else if is_hovered {
            painter.rect_filled(rect, pill_r, pal.tab_hover);
        } else {
            painter.rect_filled(rect, pill_r, pal.tab_inactive);
        }

        // Pin indicator
        let text_x = if tab.pinned {
            let pin_x = rect.x + 8.0;
            let pin_y = rect.y + (rect.h - TAB_FONT_SIZE) / 2.0;
            text.queue(
                "\u{1F4CC}",
                TAB_FONT_SIZE - 4.0,
                pin_x,
                pin_y,
                pal.accent,
                PIN_WIDTH,
                screen_w,
                screen_h,
            );
            rect.x + 8.0 + PIN_WIDTH
        } else {
            rect.x + TAB_PAD_H
        };

        // Close button (only if not pinned and multiple tabs)
        let has_close = !tab.pinned && tab_count > 1;
        let max_text_w = if has_close {
            rect.x + rect.w - TAB_PAD_H - TAB_CLOSE_SIZE - text_x
        } else {
            rect.x + rect.w - TAB_PAD_H - text_x
        };

        // Tab title (or rename buffer)
        let text_y = rect.y + (rect.h - TAB_FONT_SIZE) / 2.0;

        if is_renaming {
            draw_rename_field(
                painter, text, state, rect, text_x, text_y, max_text_w, screen_w, screen_h,
                &pal,
            );
        } else {
            let text_color = if is_active { pal.text } else { pal.muted };
            let display = truncate_title(tab.title, max_text_w);
            text.queue(
                &display,
                TAB_FONT_SIZE,
                text_x,
                text_y,
                text_color,
                max_text_w.max(10.0) + 1000.0, // give Glyphon room — we already truncated
                screen_w,
                screen_h,
            );
        }

        // Close X button
        if has_close && !is_renaming {
            draw_close_x(painter, tab_close_rect(rect), cursor_pos, is_active, &pal);
        }
    }

    // "+" new tab button (pill) — positioned after last dynamic tab
    let nb = new_tab_rect(&widths, bounds);
    let nb_r = nb.h / 2.0;
    let plus_hovered = hit(nb, cursor_pos);
    if plus_hovered {
        painter.rect_filled(nb, nb_r, pal.plus_bg_hover);
    } else {
        painter.rect_filled(nb, nb_r, pal.plus_bg);
        painter.rect_stroke_sdf(nb, nb_r, 1.5, pal.plus_border);
    }
    let plus_color = if plus_hovered { pal.plus_icon_hover } else { pal.plus_icon };
    let cx = nb.x + nb.w / 2.0;
    let cy = nb.y + nb.h / 2.0;
    let arm = 8.0;
    painter.line(cx - arm, cy, cx + arm, cy, 2.0, plus_color);
    painter.line(cx, cy - arm, cx, cy + arm, 2.0, plus_color);

    painter.pop_clip();
}

fn draw_rename_field(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &TabBarState,
    tab_rect: Rect,
    text_x: f32,
    text_y: f32,
    max_w: f32,
    screen_w: u32,
    screen_h: u32,
    pal: &TabPalette,
) {
    // Measure the actual rendered width of the text up to the cursor — using
    // `font * 0.6` constants would drift from Glyphon's real advance metrics.
    // We need a string slice up to `rename_cursor` chars, so handle multibyte
    // safely by walking chars rather than indexing bytes.
    let prefix: String = state
        .rename_buf
        .chars()
        .take(state.rename_cursor)
        .collect();
    let cursor_px = if prefix.is_empty() {
        0.0
    } else {
        text.measure_width(&prefix, TAB_FONT_SIZE)
    };
    // Per-char width estimate used for scroll bookkeeping (an exact value isn't
    // required here — just enough to keep the caret on screen).
    let char_w = (TAB_FONT_SIZE * 0.6).ceil();

    // Scroll offset so cursor stays visible within max_w
    let scroll = if cursor_px > max_w - char_w {
        cursor_px - max_w + char_w
    } else {
        0.0
    };

    // Clip to tab bounds so text doesn't overflow
    painter.push_clip(Rect::new(text_x, tab_rect.y, max_w.max(10.0), tab_rect.h));

    text.queue(
        &state.rename_buf,
        TAB_FONT_SIZE,
        text_x - scroll,
        text_y,
        pal.text,
        (max_w + scroll).max(10.0),
        screen_w,
        screen_h,
    );

    // Draw cursor
    let cursor_x = text_x + cursor_px - scroll;
    painter.rect_filled(
        Rect::new(cursor_x, text_y, 2.0, TAB_FONT_SIZE + 2.0),
        0.0,
        pal.text,
    );

    painter.pop_clip();
}

fn draw_close_x(
    painter: &mut Painter,
    cr: Rect,
    cursor_pos: Option<(f32, f32)>,
    is_active: bool,
    pal: &TabPalette,
) {
    let close_hovered = hit(cr, cursor_pos);
    if close_hovered {
        painter.rect_filled(cr, 3.0, pal.close_hover_bg);
    }
    let xc = if close_hovered {
        pal.close_hover_fg
    } else if is_active {
        pal.close_active_fg
    } else {
        pal.close_inactive_fg
    };
    let inset = 6.0;
    let x1 = cr.x + inset;
    let y1 = cr.y + inset;
    let x2 = cr.x + cr.w - inset;
    let y2 = cr.y + cr.h - inset;
    painter.line(x1, y1, x2, y2, 1.5, xc);
    painter.line(x2, y1, x1, y2, 1.5, xc);
}

/// Truncate the title to fit within `max_w` pixels, appending an ellipsis
/// if any characters were dropped. Uses the same per-char width assumption
/// as the layout / cursor code so the visual fit matches what we measured.
fn truncate_title(title: &str, max_w: f32) -> String {
    let chars: Vec<char> = title.chars().collect();
    // How many full characters fit, leaving space for the ellipsis if needed.
    let max_chars = (max_w / CHAR_W).floor() as usize;
    if chars.len() <= max_chars {
        return title.to_string();
    }
    // Need at least 1 char + ellipsis to be meaningful.
    if max_chars < 2 {
        return "\u{2026}".to_string();
    }
    let keep = max_chars.saturating_sub(1); // leave room for the ellipsis
    let mut t: String = chars[..keep].iter().collect();
    t.push('\u{2026}');
    t
}

// ── Tab context menu (right-click) ──────────────────────────────────────────

pub fn draw_tab_context_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &TabBarState,
    tabs: &[TabDisplay],
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    mode: &WindowMode,
) {
    let (tab_idx, mx, my) = match state.context_menu {
        Some(v) => v,
        None => return,
    };
    if tab_idx >= tabs.len() {
        return;
    }

    let pal = palette(mode);

    let is_pinned = tabs[tab_idx].pinned;
    let items: &[&str] = if is_pinned {
        &["Rename", "Unpin tab"]
    } else {
        &["Rename", "Pin tab", "Close tab"]
    };
    let item_count = items.len();
    let h = 12.0 + item_count as f32 * CTX_ITEM_HEIGHT + 12.0;
    let x = if mx + CTX_MENU_WIDTH > screen_w as f32 {
        mx - CTX_MENU_WIDTH
    } else {
        mx
    }
    .max(0.0);
    let y = if my + h > screen_h as f32 {
        my - h
    } else {
        my
    }
    .max(0.0);
    let menu = Rect::new(x, y, CTX_MENU_WIDTH, h);

    // Shadow + bg
    painter.rect_filled(
        Rect::new(menu.x + 2.0, menu.y + 2.0, menu.w, menu.h),
        6.0,
        pal.ctx_shadow,
    );
    painter.rect_filled(menu, 6.0, pal.ctx_bg);
    painter.rect_filled(
        Rect::new(menu.x + 3.0, menu.y, menu.w - 6.0, 1.0),
        0.0,
        pal.ctx_top_line,
    );

    let mut iy = menu.y + 8.0;
    let font = 18.0;
    for label in items {
        let item_rect = Rect::new(menu.x + 4.0, iy, menu.w - 8.0, CTX_ITEM_HEIGHT);
        let hovered = hit(item_rect, cursor_pos);
        if hovered {
            painter.rect_filled(item_rect, 4.0, pal.ctx_highlight);
        }
        let lc = if hovered { pal.text } else { pal.muted };
        text.queue(
            label,
            font,
            menu.x + 16.0,
            iy + (CTX_ITEM_HEIGHT - font) / 2.0,
            lc,
            CTX_MENU_WIDTH - 32.0,
            screen_w,
            screen_h,
        );
        iy += CTX_ITEM_HEIGHT;
    }
}

// ── Hit testing ─────────────────────────────────────────────────────────────

/// Handle a left click on the tabs region. Returns:
/// - `SwitchTab/CloseTab/NewTab/...` if a tab/button was clicked
/// - `None` if the click landed on empty tab area, on the context menu, or
///   outside the tabs region — the chrome `handle_click` should then decide
///   whether to drag the window or do nothing.
pub fn handle_click(
    state: &mut TabBarState,
    cursor_pos: Option<(f32, f32)>,
    tab_count: usize,
    tabs: &[TabDisplay],
    bounds: Rect,
    screen_w: u32,
) -> TabBarAction {
    let (px, _py) = match cursor_pos {
        Some(p) => p,
        None => return TabBarAction::None,
    };

    // Context menu takes priority — it can extend outside the tabs region.
    if let Some((tab_idx, mx, my)) = state.context_menu {
        let is_pinned = tabs.get(tab_idx).map_or(false, |t| t.pinned);
        let items: &[&str] = if is_pinned {
            &["Rename", "Unpin tab"]
        } else {
            &["Rename", "Pin tab", "Close tab"]
        };
        let h = 12.0 + items.len() as f32 * CTX_ITEM_HEIGHT + 12.0;
        let sw = screen_w as f32;
        let x = if mx + CTX_MENU_WIDTH > sw { mx - CTX_MENU_WIDTH } else { mx }.max(0.0);
        let y = if my + h > sw { my - h } else { my }.max(0.0);
        let menu = Rect::new(x, y, CTX_MENU_WIDTH, h);

        if hit(menu, cursor_pos) {
            let mut iy = menu.y + 8.0;
            for (i, _label) in items.iter().enumerate() {
                let item_rect = Rect::new(menu.x + 4.0, iy, menu.w - 8.0, CTX_ITEM_HEIGHT);
                if hit(item_rect, cursor_pos) {
                    state.context_menu = None;
                    return match (is_pinned, i) {
                        (_, 0) => {
                            let title = tabs.get(tab_idx).map_or("", |t| t.title);
                            state.start_rename(tab_idx, title);
                            TabBarAction::None
                        }
                        (true, 1) => TabBarAction::TogglePin(tab_idx),
                        (false, 1) => TabBarAction::TogglePin(tab_idx),
                        (false, 2) => TabBarAction::CloseTab(tab_idx),
                        _ => TabBarAction::None,
                    };
                }
                iy += CTX_ITEM_HEIGHT;
            }
        }
        state.context_menu = None;
        return TabBarAction::None;
    }

    // Click must be inside the tabs region for tab/button hit testing.
    if !hit(bounds, cursor_pos) {
        return TabBarAction::None;
    }

    let titles: Vec<&str> = tabs.iter().map(|t| t.title).collect();
    let widths = calc_tab_widths(&titles, bounds.w);

    // If renaming, click-away confirms
    if let Some(idx) = state.renaming {
        let rect = tab_rect_dynamic(idx, &widths, bounds);
        if !hit(rect, cursor_pos) {
            let name = state.rename_buf.clone();
            state.cancel_rename();
            return TabBarAction::ConfirmRename(idx, name);
        }
        return TabBarAction::None;
    }

    // New tab button
    let nb = new_tab_rect(&widths, bounds);
    if hit(nb, cursor_pos) {
        return TabBarAction::NewTab;
    }

    // Tab clicks
    for i in 0..tab_count {
        let rect = tab_rect_dynamic(i, &widths, bounds);
        if hit(rect, cursor_pos) {
            // Close button
            let is_pinned = tabs.get(i).map_or(false, |t| t.pinned);
            if !is_pinned && tab_count > 1 {
                if hit(tab_close_rect(rect), cursor_pos) {
                    return TabBarAction::CloseTab(i);
                }
            }

            // Double-click detection
            let now = Instant::now();
            let elapsed = now.duration_since(state.last_click_time).as_millis();
            if state.last_click_tab == Some(i) && elapsed < DOUBLE_CLICK_MS {
                state.last_click_tab = None;
                let title = tabs.get(i).map_or("", |t| t.title);
                state.start_rename(i, title);
                return TabBarAction::SwitchTab(i);
            }
            state.last_click_time = now;
            state.last_click_tab = Some(i);

            // Start potential drag
            state.dragging = Some(i);
            state.drag_start_x = px;
            state.drag_offset_x = 0.0;
            state.drag_committed = false;

            return TabBarAction::SwitchTab(i);
        }
    }

    TabBarAction::None
}

pub fn handle_right_click(
    state: &mut TabBarState,
    cursor_pos: Option<(f32, f32)>,
    tabs: &[TabDisplay],
    bounds: Rect,
) -> bool {
    let (px, py) = match cursor_pos {
        Some(p) => p,
        None => return false,
    };

    if !hit(bounds, cursor_pos) {
        return false;
    }

    let titles: Vec<&str> = tabs.iter().map(|t| t.title).collect();
    let widths = calc_tab_widths(&titles, bounds.w);
    for i in 0..tabs.len() {
        let rect = tab_rect_dynamic(i, &widths, bounds);
        if hit(rect, cursor_pos) {
            state.context_menu = Some((i, px, py));
            return true;
        }
    }
    false
}

/// Call on mouse move while dragging to potentially reorder.
pub fn handle_drag_move(
    state: &mut TabBarState,
    cursor_x: f32,
    tabs: &[TabDisplay],
    bounds: Rect,
) -> Option<TabBarAction> {
    let dragging_idx = state.dragging?;
    let delta = cursor_x - state.drag_start_x;

    if !state.drag_committed && delta.abs() < 8.0 {
        return None;
    }
    state.drag_committed = true;
    state.drag_offset_x = delta;

    let titles: Vec<&str> = tabs.iter().map(|t| t.title).collect();
    let widths = calc_tab_widths(&titles, bounds.w);
    let rect = tab_rect_dynamic(dragging_idx, &widths, bounds);
    let dragged_center = rect.x + delta + rect.w / 2.0;

    // Check if we crossed into an adjacent tab
    for i in 0..tabs.len() {
        if i == dragging_idx {
            continue;
        }
        let other = tab_rect_dynamic(i, &widths, bounds);
        let other_center = other.x + other.w / 2.0;
        if (i < dragging_idx && dragged_center < other_center)
            || (i > dragging_idx && dragged_center > other_center)
        {
            state.dragging = Some(i);
            state.drag_start_x = cursor_x;
            state.drag_offset_x = 0.0;
            return Some(TabBarAction::Reorder {
                from: dragging_idx,
                to: i,
            });
        }
    }
    None
}

/// Call on mouse release to end drag.
pub fn handle_drag_end(state: &mut TabBarState) {
    state.dragging = None;
    state.drag_offset_x = 0.0;
    state.drag_committed = false;
}

// ── Rename key handling ─────────────────────────────────────────────────────

/// Handle a key press while renaming. Returns Some(action) if rename completes.
pub fn handle_rename_key(state: &mut TabBarState, key: &str) -> Option<TabBarAction> {
    let idx = state.renaming?;

    match key {
        "Enter" | "Return" => {
            let name = state.rename_buf.clone();
            state.cancel_rename();
            Some(TabBarAction::ConfirmRename(idx, name))
        }
        "Escape" => {
            state.cancel_rename();
            Some(TabBarAction::None)
        }
        "Backspace" => {
            if state.rename_cursor > 0 {
                state.rename_cursor -= 1;
                state.rename_buf.remove(state.rename_cursor);
            }
            Some(TabBarAction::None)
        }
        "Delete" => {
            if state.rename_cursor < state.rename_buf.len() {
                state.rename_buf.remove(state.rename_cursor);
            }
            Some(TabBarAction::None)
        }
        "Left" => {
            state.rename_cursor = state.rename_cursor.saturating_sub(1);
            Some(TabBarAction::None)
        }
        "Right" => {
            state.rename_cursor = (state.rename_cursor + 1).min(state.rename_buf.len());
            Some(TabBarAction::None)
        }
        "Home" => {
            state.rename_cursor = 0;
            Some(TabBarAction::None)
        }
        "End" => {
            state.rename_cursor = state.rename_buf.len();
            Some(TabBarAction::None)
        }
        _ => None,
    }
}

/// Handle a character input during rename.
pub fn handle_rename_char(state: &mut TabBarState, ch: char) {
    if state.renaming.is_none() || ch.is_control() {
        return;
    }
    state.rename_buf.insert(state.rename_cursor, ch);
    state.rename_cursor += 1;
}

/// Returns true if the tab bar is currently capturing keyboard input (rename mode).
pub fn is_capturing_input(state: &TabBarState) -> bool {
    state.renaming.is_some()
}
