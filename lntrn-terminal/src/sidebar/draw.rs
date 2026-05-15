use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::terminal::Color8;

use super::{
    hit, DirEntry, EditMode, SidebarMode, SidebarState, CHAR_WIDTH, CTX_FONT, CTX_ITEM_HEIGHT,
    CTX_MENU_WIDTH, FONT_SIZE, ICON_FONT, INDENT_PX, ITEM_HEIGHT, ROOT_CTX, TOGGLE_H,
};

// ── Colors ───────────────────────────────────────────────────────────────────

const SURFACE: Color8 = Color8::from_rgb(30, 30, 30);
const SURFACE_HOVER: Color8 = Color8::from_rgba(255, 255, 255, 15);
const TEXT: Color8 = Color8::from_rgb(200, 200, 200);
const TEXT_DIM: Color8 = Color8::from_rgb(120, 120, 120);
const ACCENT: Color8 = Color8::from_rgb(255, 200, 0);
const DANGER: Color8 = Color8::from_rgb(220, 60, 60);
const DIVIDER: Color8 = Color8::from_rgba(255, 255, 255, 12);
const MENU_BG: Color8 = Color8::from_rgb(42, 42, 42);

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

/// Draw the sidebar. Returns the width consumed (0 if hidden).
pub fn draw_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &SidebarState,
    chrome_h: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) -> f32 {
    if !state.visible {
        return 0.0;
    }

    let h = screen_h as f32 - chrome_h;
    let sw = state.width;
    let sidebar_rect = Rect::new(0.0, chrome_h, sw, h);

    // Background
    painter.rect_filled(sidebar_rect, 0.0, c(SURFACE));

    // Right edge divider
    painter.rect_filled(
        Rect::new(sw - 1.0, chrome_h, 1.0, h),
        0.0,
        c(DIVIDER),
    );

    // Mode toggle buttons [Files] [Git]
    draw_mode_toggle(painter, text, state, chrome_h, sw, screen_w, screen_h, cursor_pos);

    // In Git mode, the git_sidebar module draws the content below the toggle
    if state.mode == SidebarMode::Git {
        return sw;
    }

    // Header
    let header_h = 42.0;
    let header_y = chrome_h + TOGGLE_H + 4.0;
    let root_name = state
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    text.queue(
        &root_name.to_uppercase(),
        FONT_SIZE,
        14.0,
        header_y + (header_h - FONT_SIZE) / 2.0,
        c(TEXT_DIM),
        sw - 28.0,
        screen_w,
        screen_h,
    );

    // Clip the file list area
    let list_y = chrome_h + TOGGLE_H + header_h;
    let list_h = h - TOGGLE_H - header_h;
    let clip = Rect::new(0.0, list_y, sw, list_h);
    painter.push_clip(clip);

    // Draw entries
    let mut y = list_y - state.scroll_offset;
    for (i, entry) in state.entries.iter().enumerate() {
        if y + ITEM_HEIGHT < list_y {
            y += ITEM_HEIGHT;
            continue;
        }
        if y > list_y + list_h {
            break;
        }

        let indent = entry.depth as f32 * INDENT_PX + 10.0;
        let item_rect = Rect::new(4.0, y, sw - 8.0, ITEM_HEIGHT);

        let hovered = cursor_pos.map_or(false, |(cx, cy)| {
            cx >= item_rect.x
                && cx <= item_rect.x + item_rect.w
                && cy >= y.max(list_y)
                && cy <= (y + ITEM_HEIGHT).min(list_y + list_h)
        });

        if hovered {
            painter.rect_filled(item_rect, 4.0, c(SURFACE_HOVER));
        }

        // Icon — git status badge replaces · for tracked files
        let git_ch = if !entry.is_dir {
            state.git_marks.iter().find(|(p, _)| *p == entry.path).map(|(_, ch)| *ch)
        } else {
            None
        };
        let (icon, icon_color) = if entry.is_dir {
            (if entry.expanded { "▾" } else { "▸" }, c(ACCENT))
        } else if let Some(ch) = git_ch {
            let col = match ch {
                'M' => c(ACCENT),
                'A' => c(Color8::from_rgb(80, 200, 80)),
                'D' => c(DANGER),
                _ => c(TEXT_DIM),
            };
            (match ch { 'M'=>"M", 'A'=>"A", 'D'=>"D", 'R'=>"R", '?'=>"?", _=>"·" }, col)
        } else {
            ("·", c(TEXT_DIM))
        };
        text.queue(icon, ICON_FONT, indent, y + (ITEM_HEIGHT - ICON_FONT) / 2.0,
            icon_color, 16.0, screen_w, screen_h);

        // Line count (right-aligned, color-coded by size)
        if let Some(lines) = entry.line_count {
            let count_str = lines.to_string();
            let line_color = if lines >= 1000 {
                c(DANGER) // red
            } else if lines >= 700 {
                c(Color8::from_rgb(230, 150, 30)) // orange
            } else if lines >= 500 {
                c(ACCENT) // yellow
            } else {
                c(TEXT_DIM)
            };
            let fs = FONT_SIZE - 6.0;
            let cw = count_str.len() as f32 * fs * 0.55;
            text.queue(&count_str, fs, sw - cw - 10.0,
                y + (ITEM_HEIGHT - fs) / 2.0,
                line_color, cw + 4.0, screen_w, screen_h);
        }

        // Name — or inline edit field
        let name_x = indent + 16.0;
        let is_editing = state.edit.as_ref().map_or(false, |e| {
            e.mode == EditMode::Rename && e.entry_idx == i
        });

        if is_editing {
            let edit = state.edit.as_ref().unwrap();
            let text_y = y + (ITEM_HEIGHT - FONT_SIZE) / 2.0;
            let max_w = sw - name_x - 8.0;

            // Edit background
            painter.rect_filled(
                Rect::new(name_x - 4.0, y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0),
                4.0,
                c(Color8::from_rgba(50, 50, 50, 255)),
            );
            // Gold border
            let b = 1.5;
            let er = Rect::new(name_x - 4.0, y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0);
            painter.rect_filled(Rect::new(er.x, er.y, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y + er.h - b, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y, b, er.h), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x + er.w - b, er.y, b, er.h), 0.0, c(ACCENT));

            text.queue(
                &edit.buf,
                FONT_SIZE,
                name_x,
                text_y,
                c(TEXT),
                max_w,
                screen_w,
                screen_h,
            );

            // Cursor
            let cursor_x = name_x + edit.cursor as f32 * CHAR_WIDTH;
            painter.rect_filled(
                Rect::new(cursor_x, text_y, 2.0, FONT_SIZE + 2.0),
                0.0,
                c(TEXT),
            );
        } else {
            let name_color = if hovered { c(ACCENT) } else { c(TEXT) };
            text.queue(
                &entry.name,
                FONT_SIZE,
                name_x,
                y + (ITEM_HEIGHT - FONT_SIZE) / 2.0,
                name_color,
                sw - name_x - 8.0,
                screen_w,
                screen_h,
            );
        }

        y += ITEM_HEIGHT;
    }

    // Draw inline edit for new file/folder (appears after parent's children)
    if let Some(edit) = &state.edit {
        if edit.mode == EditMode::NewFile || edit.mode == EditMode::NewFolder {
            let depth = if edit.entry_idx < state.entries.len() {
                if state.entries[edit.entry_idx].is_dir {
                    state.entries[edit.entry_idx].depth + 1
                } else {
                    state.entries[edit.entry_idx].depth
                }
            } else {
                0
            };
            let insert_y = entry_y_position(edit.entry_idx, &state.entries, list_y, state.scroll_offset);
            let indent = depth as f32 * INDENT_PX + 10.0;
            let name_x = indent + 16.0;
            let text_y = insert_y + (ITEM_HEIGHT - FONT_SIZE) / 2.0;
            let max_w = sw - name_x - 8.0;

            // Icon
            let icon = if edit.mode == EditMode::NewFolder { "▸" } else { "·" };
            let icon_color = if edit.mode == EditMode::NewFolder { c(ACCENT) } else { c(TEXT_DIM) };
            text.queue(
                icon,
                ICON_FONT,
                indent,
                insert_y + (ITEM_HEIGHT - ICON_FONT) / 2.0,
                icon_color,
                16.0,
                screen_w,
                screen_h,
            );

            // Edit background
            painter.rect_filled(
                Rect::new(name_x - 4.0, insert_y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0),
                4.0,
                c(Color8::from_rgba(50, 50, 50, 255)),
            );
            let b = 1.5;
            let er = Rect::new(name_x - 4.0, insert_y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0);
            painter.rect_filled(Rect::new(er.x, er.y, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y + er.h - b, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y, b, er.h), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x + er.w - b, er.y, b, er.h), 0.0, c(ACCENT));

            text.queue(
                &edit.buf,
                FONT_SIZE,
                name_x,
                text_y,
                c(TEXT),
                max_w,
                screen_w,
                screen_h,
            );

            let cursor_x = name_x + edit.cursor as f32 * CHAR_WIDTH;
            painter.rect_filled(
                Rect::new(cursor_x, text_y, 2.0, FONT_SIZE + 2.0),
                0.0,
                c(TEXT),
            );
        }
    }

    painter.pop_clip();

    sw
}

/// Draw the sidebar context menu overlay (call in overlay pass).
pub fn draw_sidebar_context_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &SidebarState,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let (idx, mx, my) = match state.context_menu {
        Some(v) => v,
        None => return,
    };
    if idx != ROOT_CTX && idx >= state.entries.len() {
        return;
    }

    let items: &[(&str, Color8)] = if idx == ROOT_CTX {
        &[
            ("New File", TEXT),
            ("New Folder", TEXT),
        ]
    } else if state.entries[idx].is_dir {
        &[
            ("New File", TEXT),
            ("New Folder", TEXT),
            ("Rename", TEXT),
            ("Delete", DANGER),
        ]
    } else {
        &[
            ("Open with Lantern Code", TEXT),
            ("Rename", TEXT),
            ("Delete", DANGER),
        ]
    };

    let item_count = items.len();
    let menu_h = 10.0 + item_count as f32 * CTX_ITEM_HEIGHT + 10.0;
    let x = mx.min(screen_w as f32 - CTX_MENU_WIDTH - 4.0).max(0.0);
    let y = if my + menu_h > screen_h as f32 { my - menu_h } else { my }.max(0.0);
    let menu = Rect::new(x, y, CTX_MENU_WIDTH, menu_h);

    // Shadow + bg
    painter.rect_filled(
        Rect::new(menu.x + 2.0, menu.y + 2.0, menu.w, menu.h),
        8.0,
        c(Color8::from_rgba(0, 0, 0, 60)),
    );
    painter.rect_filled(menu, 8.0, c(MENU_BG));

    let mut iy = menu.y + 6.0;
    for (label, color) in items {
        let item_rect = Rect::new(menu.x + 4.0, iy, menu.w - 8.0, CTX_ITEM_HEIGHT);
        let hovered = hit(item_rect, cursor_pos);
        if hovered {
            painter.rect_filled(item_rect, 4.0, c(SURFACE_HOVER));
        }
        let lc = if hovered { c(ACCENT) } else { c(*color) };
        text.queue(
            label,
            CTX_FONT,
            menu.x + 16.0,
            iy + (CTX_ITEM_HEIGHT - CTX_FONT) / 2.0,
            lc,
            CTX_MENU_WIDTH - 32.0,
            screen_w,
            screen_h,
        );
        iy += CTX_ITEM_HEIGHT;
    }
}

fn entry_y_position(
    entry_idx: usize,
    entries: &[DirEntry],
    list_y: f32,
    scroll_offset: f32,
) -> f32 {
    // Position right after the entry and its expanded children
    let mut pos = entry_idx + 1;
    if entry_idx < entries.len() && entries[entry_idx].is_dir && entries[entry_idx].expanded {
        let parent_depth = entries[entry_idx].depth;
        while pos < entries.len() && entries[pos].depth > parent_depth {
            pos += 1;
        }
    }
    list_y - scroll_offset + pos as f32 * ITEM_HEIGHT
}

// ── Mode toggle ─────────────────────────────────────────────────────────────

fn draw_mode_toggle(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &SidebarState,
    chrome_h: f32,
    sw: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let y = chrome_h + 4.0;
    let btn_w = (sw - 16.0) / 2.0;
    let btn_h = TOGGLE_H - 8.0;

    // Files button
    let fx = 6.0;
    let files_rect = Rect::new(fx, y, btn_w, btn_h);
    let files_active = state.mode == SidebarMode::Files;
    let files_hover = !files_active && hit(files_rect, cursor_pos);
    let files_bg = if files_active {
        c(ACCENT)
    } else if files_hover {
        c(SURFACE_HOVER)
    } else {
        c(Color8::from_rgba(55, 55, 55, 255))
    };
    let files_fg = if files_active { c(Color8::from_rgb(255, 255, 255)) } else { c(TEXT_DIM) };
    painter.rect_filled(files_rect, 4.0, files_bg);
    let ft_w = 5.0 * CHAR_WIDTH;
    text.queue(
        "Files", FONT_SIZE, fx + (btn_w - ft_w) / 2.0, y + (btn_h - FONT_SIZE) / 2.0,
        files_fg, btn_w, screen_w, screen_h,
    );

    // Git button
    let gx = 6.0 + btn_w + 4.0;
    let git_rect = Rect::new(gx, y, btn_w, btn_h);
    let git_active = state.mode == SidebarMode::Git;
    let git_hover = !git_active && hit(git_rect, cursor_pos);
    let git_bg = if git_active {
        c(ACCENT)
    } else if git_hover {
        c(SURFACE_HOVER)
    } else {
        c(Color8::from_rgba(55, 55, 55, 255))
    };
    let git_fg = if git_active { c(Color8::from_rgb(255, 255, 255)) } else { c(TEXT_DIM) };
    painter.rect_filled(git_rect, 4.0, git_bg);
    let gt_w = 3.0 * CHAR_WIDTH;
    text.queue(
        "Git", FONT_SIZE, gx + (btn_w - gt_w) / 2.0, y + (btn_h - FONT_SIZE) / 2.0,
        git_fg, btn_w, screen_w, screen_h,
    );
}
