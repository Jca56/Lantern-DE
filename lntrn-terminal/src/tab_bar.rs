use std::time::Instant;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::terminal::Color8;

// ── Constants ───────────────────────────────────────────────────────────────

pub const TAB_BAR_HEIGHT: f32 = 40.0;
const TAB_MAX_WIDTH: f32 = 200.0;
const TAB_MIN_WIDTH: f32 = 90.0;
const TAB_GAP: f32 = 2.0;
const TAB_PAD_H: f32 = 14.0;
const NEW_TAB_WIDTH: f32 = 48.0;
const TAB_CLOSE_SIZE: f32 = 20.0;
const TAB_FONT_SIZE: f32 = 20.0;
const PIN_WIDTH: f32 = 22.0;
const DOUBLE_CLICK_MS: u128 = 400;

// Palette
const SURFACE: Color8 = Color8::from_rgb(30, 30, 30);
const TEXT_COLOR: Color8 = Color8::from_rgb(236, 236, 236);
const MUTED: Color8 = Color8::from_rgb(144, 144, 144);
const ACCENT: Color8 = Color8::from_rgb(200, 134, 10);

// Tab context menu
const CTX_MENU_WIDTH: f32 = 180.0;
const CTX_ITEM_HEIGHT: f32 = 36.0;

// ── Tab bar Y offset (sits below the title bar) ────────────────────────────

fn bar_y() -> f32 {
    crate::ui_chrome::TITLE_BAR_HEIGHT
}

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
    StartDrag,
}

// ── Layout helpers ──────────────────────────────────────────────────────────

fn calc_tab_width(tab_count: usize, available: f32) -> f32 {
    let space = available - NEW_TAB_WIDTH - 4.0;
    let per_tab = space / tab_count.max(1) as f32 - TAB_GAP;
    per_tab.clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH)
}

fn tab_rect(idx: usize, tab_count: usize, screen_w: f32) -> Rect {
    let tab_w = calc_tab_width(tab_count, screen_w - 16.0);
    let x = 8.0 + idx as f32 * (tab_w + TAB_GAP);
    Rect::new(x, bar_y(), tab_w, TAB_BAR_HEIGHT)
}

fn tab_close_rect(tab: Rect) -> Rect {
    let x = tab.x + tab.w - TAB_PAD_H - TAB_CLOSE_SIZE + 4.0;
    let y = tab.y + (tab.h - TAB_CLOSE_SIZE) / 2.0;
    Rect::new(x, y, TAB_CLOSE_SIZE, TAB_CLOSE_SIZE)
}

fn new_tab_button_rect(tab_count: usize, screen_w: f32) -> Rect {
    let tab_w = calc_tab_width(tab_count, screen_w - 16.0);
    let x = 8.0 + tab_count as f32 * (tab_w + TAB_GAP) + 4.0;
    Rect::new(x, bar_y(), NEW_TAB_WIDTH, TAB_BAR_HEIGHT)
}

fn hit(rect: Rect, pos: Option<(f32, f32)>) -> bool {
    if let Some((x, y)) = pos {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    } else {
        false
    }
}

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw_tab_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &TabBarState,
    tabs: &[TabDisplay],
    active: usize,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let sw = screen_w as f32;
    let tab_count = tabs.len();

    // Tab bar background
    painter.rect_filled(
        Rect::new(0.0, bar_y(), sw, TAB_BAR_HEIGHT),
        0.0,
        c(SURFACE),
    );

    for (i, tab) in tabs.iter().enumerate() {
        let mut rect = tab_rect(i, tab_count, sw);

        // If dragging this tab, offset it
        if state.dragging == Some(i) && state.drag_committed {
            rect.x += state.drag_offset_x;
        }

        let is_active = i == active;
        let is_hovered = hit(rect, cursor_pos);
        let is_renaming = state.renaming == Some(i);

        // Tab background
        if is_renaming {
            painter.rect_filled(rect, 4.0, c(Color8::from_rgba(50, 50, 50, 255)));
            // Gold border for rename mode
            let b = 2.0;
            painter.rect_filled(Rect::new(rect.x, rect.y, rect.w, b), 2.0, c(ACCENT));
            painter.rect_filled(
                Rect::new(rect.x, rect.y + rect.h - b, rect.w, b),
                2.0,
                c(ACCENT),
            );
            painter.rect_filled(Rect::new(rect.x, rect.y, b, rect.h), 2.0, c(ACCENT));
            painter.rect_filled(
                Rect::new(rect.x + rect.w - b, rect.y, b, rect.h),
                2.0,
                c(ACCENT),
            );
        } else if is_active {
            painter.rect_filled(rect, 4.0, c(Color8::from_rgba(50, 50, 50, 255)));
            // Accent bar on bottom
            painter.rect_filled(
                Rect::new(rect.x, rect.y + rect.h - 3.0, rect.w, 3.0),
                0.0,
                c(ACCENT),
            );
        } else if is_hovered {
            painter.rect_filled(rect, 4.0, c(Color8::from_rgba(45, 45, 45, 255)));
        } else {
            painter.rect_filled(rect, 4.0, c(Color8::from_rgba(35, 35, 35, 255)));
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
                c(ACCENT),
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
            );
        } else {
            let text_color = if is_active { c(TEXT_COLOR) } else { c(MUTED) };
            let display = truncate_title(tab.title, max_text_w);
            text.queue(
                &display,
                TAB_FONT_SIZE,
                text_x,
                text_y,
                text_color,
                max_text_w.max(10.0),
                screen_w,
                screen_h,
            );
        }

        // Close X button
        if has_close && !is_renaming {
            draw_close_x(painter, tab_close_rect(rect), cursor_pos, is_active);
        }
    }

    // "+" new tab button
    let nb = new_tab_button_rect(tab_count, sw);
    let plus_hovered = hit(nb, cursor_pos);
    if plus_hovered {
        painter.rect_filled(nb, 6.0, c(Color8::from_rgba(255, 255, 255, 35)));
    } else {
        painter.rect_filled(nb, 6.0, c(Color8::from_rgba(255, 255, 255, 12)));
        let b = 1.5;
        let bc = c(Color8::from_rgba(255, 255, 255, 40));
        painter.rect_filled(Rect::new(nb.x, nb.y, nb.w, b), 0.0, bc);
        painter.rect_filled(Rect::new(nb.x, nb.y + nb.h - b, nb.w, b), 0.0, bc);
        painter.rect_filled(Rect::new(nb.x, nb.y, b, nb.h), 0.0, bc);
        painter.rect_filled(Rect::new(nb.x + nb.w - b, nb.y, b, nb.h), 0.0, bc);
    }
    let plus_color = if plus_hovered {
        c(TEXT_COLOR)
    } else {
        c(Color8::from_rgb(190, 190, 190))
    };
    let plus_font = TAB_FONT_SIZE + 6.0;
    let plus_y = nb.y + (nb.h - plus_font) / 2.0;
    text.queue(
        "+",
        plus_font,
        nb.x + (nb.w - 12.0) / 2.0,
        plus_y,
        plus_color,
        24.0,
        screen_w,
        screen_h,
    );
}

fn draw_rename_field(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &TabBarState,
    _tab_rect: Rect,
    text_x: f32,
    text_y: f32,
    max_w: f32,
    screen_w: u32,
    screen_h: u32,
) {
    // Draw the rename text
    text.queue(
        &state.rename_buf,
        TAB_FONT_SIZE,
        text_x,
        text_y,
        c(TEXT_COLOR),
        max_w.max(10.0),
        screen_w,
        screen_h,
    );

    // Draw cursor
    let char_w = TAB_FONT_SIZE * 0.6;
    let cursor_x = text_x + state.rename_cursor as f32 * char_w;
    painter.rect_filled(
        Rect::new(cursor_x, text_y, 2.0, TAB_FONT_SIZE + 2.0),
        0.0,
        c(TEXT_COLOR),
    );
}

fn draw_close_x(
    painter: &mut Painter,
    cr: Rect,
    cursor_pos: Option<(f32, f32)>,
    is_active: bool,
) {
    let close_hovered = hit(cr, cursor_pos);
    if close_hovered {
        painter.rect_filled(cr, 3.0, c(Color8::from_rgba(232, 50, 50, 40)));
    }
    let xc = c(if close_hovered {
        Color8::from_rgb(232, 80, 80)
    } else if is_active {
        Color8::from_rgb(180, 60, 60)
    } else {
        Color8::from_rgb(120, 50, 50)
    });
    let inset = 5.0;
    let steps = 8;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let px = cr.x + inset + t * (cr.w - inset * 2.0);
        let py1 = cr.y + inset + t * (cr.h - inset * 2.0);
        let py2 = cr.y + cr.h - inset - t * (cr.h - inset * 2.0);
        painter.rect_filled(Rect::new(px - 0.75, py1 - 0.75, 1.5, 1.5), 0.0, xc);
        painter.rect_filled(Rect::new(px - 0.75, py2 - 0.75, 1.5, 1.5), 0.0, xc);
    }
}

fn truncate_title(title: &str, _max_w: f32) -> String {
    let chars: Vec<char> = title.chars().collect();
    if chars.len() > 18 {
        let mut t: String = chars[..17].iter().collect();
        t.push('\u{2026}');
        t
    } else {
        title.to_string()
    }
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
) {
    let (tab_idx, mx, my) = match state.context_menu {
        Some(v) => v,
        None => return,
    };
    if tab_idx >= tabs.len() {
        return;
    }

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
        c(Color8::from_rgba(0, 0, 0, 60)),
    );
    painter.rect_filled(menu, 6.0, c(Color8::from_rgb(39, 39, 39)));
    painter.rect_filled(
        Rect::new(menu.x + 3.0, menu.y, menu.w - 6.0, 1.0),
        0.0,
        c(Color8::from_rgba(255, 255, 255, 15)),
    );

    let mut iy = menu.y + 8.0;
    let font = 18.0;
    for label in items {
        let item_rect = Rect::new(menu.x + 4.0, iy, menu.w - 8.0, CTX_ITEM_HEIGHT);
        let hovered = hit(item_rect, cursor_pos);
        if hovered {
            painter.rect_filled(item_rect, 4.0, c(Color8::from_rgba(255, 255, 255, 15)));
        }
        let lc = if hovered { c(TEXT_COLOR) } else { c(MUTED) };
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

pub fn handle_click(
    state: &mut TabBarState,
    cursor_pos: Option<(f32, f32)>,
    tab_count: usize,
    tabs: &[TabDisplay],
    screen_w: u32,
) -> TabBarAction {
    let (px, py) = match cursor_pos {
        Some(p) => p,
        None => return TabBarAction::None,
    };

    let sw = screen_w as f32;

    // Context menu takes priority
    if let Some((tab_idx, mx, my)) = state.context_menu {
        let is_pinned = tabs.get(tab_idx).map_or(false, |t| t.pinned);
        let items: &[&str] = if is_pinned {
            &["Rename", "Unpin tab"]
        } else {
            &["Rename", "Pin tab", "Close tab"]
        };
        let h = 12.0 + items.len() as f32 * CTX_ITEM_HEIGHT + 12.0;
        let x = if mx + CTX_MENU_WIDTH > sw { mx - CTX_MENU_WIDTH } else { mx }.max(0.0);
        let y = if my + h > screen_w as f32 { my - h } else { my }.max(0.0);
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

    // Not in tab bar area
    if py < bar_y() || py > bar_y() + TAB_BAR_HEIGHT {
        // If click is in title bar area, allow drag
        if py < bar_y() {
            return TabBarAction::StartDrag;
        }
        return TabBarAction::None;
    }

    // If renaming, Enter/click-away confirms
    if let Some(idx) = state.renaming {
        let rect = tab_rect(idx, tab_count, sw);
        if !hit(rect, cursor_pos) {
            let name = state.rename_buf.clone();
            state.cancel_rename();
            return TabBarAction::ConfirmRename(idx, name);
        }
        return TabBarAction::None;
    }

    // New tab button
    if hit(new_tab_button_rect(tab_count, sw), cursor_pos) {
        return TabBarAction::NewTab;
    }

    // Tab clicks
    for i in 0..tab_count {
        let rect = tab_rect(i, tab_count, sw);
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
    tab_count: usize,
    screen_w: u32,
) -> bool {
    let (px, py) = match cursor_pos {
        Some(p) => p,
        None => return false,
    };

    if py < bar_y() || py > bar_y() + TAB_BAR_HEIGHT {
        return false;
    }

    let sw = screen_w as f32;
    for i in 0..tab_count {
        let rect = tab_rect(i, tab_count, sw);
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
    tab_count: usize,
    screen_w: u32,
) -> Option<TabBarAction> {
    let dragging_idx = state.dragging?;
    let delta = cursor_x - state.drag_start_x;

    if !state.drag_committed && delta.abs() < 8.0 {
        return None;
    }
    state.drag_committed = true;
    state.drag_offset_x = delta;

    let sw = screen_w as f32;
    let rect = tab_rect(dragging_idx, tab_count, sw);
    let dragged_center = rect.x + delta + rect.w / 2.0;

    // Check if we crossed into an adjacent tab
    for i in 0..tab_count {
        if i == dragging_idx {
            continue;
        }
        let other = tab_rect(i, tab_count, sw);
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
