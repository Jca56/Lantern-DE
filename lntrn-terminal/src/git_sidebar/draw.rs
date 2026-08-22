use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::git::ops::{FileState, FileStatus};
use crate::terminal::Color8;

use super::{
    c, GitSidebarState, ACCENT, BLUE, BTN_BG, BUTTON_H, CHAR_W, DIVIDER, FONT, GREEN, INPUT_H,
    ITEM_H, PAD, RED, SECTION_H, SMALL_FONT, SURFACE_HOVER, TEXT_C, TEXT_DIM,
};

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw_git_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &GitSidebarState,
    sw: f32,
    top_y: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let scale = state.scale;
    let section_h = SECTION_H * scale;
    let item_h = ITEM_H * scale;
    let button_h = BUTTON_H * scale;
    let input_h = INPUT_H * scale;
    let font = FONT * scale;
    let small_font = SMALL_FONT * scale;
    let pad = PAD * scale;

    let area_h = screen_h as f32 - top_y;
    let clip = Rect::new(0.0, top_y, sw, area_h);
    painter.push_clip(clip);

    let mut y = top_y - state.scroll_offset;

    // ── 1. Branch header (collapsible) ──────────────────────────────
    if let Some(ref status) = state.status {
        let chevron = if state.branches_expanded { "v" } else { ">" };
        let header_rect = Rect::new(0.0, y, sw - 44.0 * scale, section_h);
        let header_hover = cursor_pos.map_or(false, |(cx, cy)| {
            cx >= 0.0 && cx < sw - 44.0 * scale && cy >= y && cy < y + section_h
        });

        // Chevron
        let chev_color = if header_hover { c(ACCENT) } else { c(TEXT_DIM) };
        text.queue(
            chevron,
            small_font,
            pad,
            y + (section_h - small_font) / 2.0,
            chev_color,
            16.0 * scale,
            screen_w,
            screen_h,
        );
        // Branch name
        let name_color = if header_hover { c(ACCENT) } else { c(TEXT_C) };
        text.queue(
            &status.branch,
            font,
            pad + 18.0 * scale,
            y + 5.0 * scale,
            name_color,
            sw - pad * 2.0 - 60.0 * scale,
            screen_w,
            screen_h,
        );
        if header_hover {
            painter.rect_filled(header_rect, 0.0, c(SURFACE_HOVER));
        }

        // Refresh button (right side)
        let ref_w = 28.0 * scale;
        let ref_x = sw - pad - ref_w;
        let ref_rect = Rect::new(ref_x, y + 2.0 * scale, ref_w, section_h - 4.0 * scale);
        let ref_hover = cursor_pos.map_or(false, |(cx, cy)| {
            cx >= ref_rect.x
                && cx <= ref_rect.x + ref_rect.w
                && cy >= ref_rect.y
                && cy <= ref_rect.y + ref_rect.h
        });
        let ref_bg = if ref_hover {
            c(SURFACE_HOVER)
        } else {
            c(BTN_BG)
        };
        painter.rect_filled(ref_rect, 4.0 * scale, ref_bg);
        let ref_color = if ref_hover { c(ACCENT) } else { c(TEXT_DIM) };
        text.queue(
            "R",
            small_font,
            ref_x + (ref_w - small_font * 0.55) / 2.0,
            y + (section_h - small_font) / 2.0,
            ref_color,
            ref_w,
            screen_w,
            screen_h,
        );

        // Ahead/behind (left of refresh)
        let ab = format!("{}  {}", status.ahead, status.behind);
        let ab_x = ref_x - 8.0 * scale - ab.len() as f32 * small_font * 0.55;
        text.queue(
            &ab,
            small_font,
            ab_x,
            y + 8.0 * scale,
            c(TEXT_DIM),
            100.0 * scale,
            screen_w,
            screen_h,
        );
        y += section_h;

        // Expanded branch list
        if state.branches_expanded {
            for branch in &state.branches {
                let item_rect = Rect::new(4.0 * scale, y, sw - 8.0 * scale, item_h);
                let hovered = cursor_pos.map_or(false, |(cx, cy)| {
                    cx >= 0.0
                        && cx <= sw
                        && cy >= y.max(top_y)
                        && cy < (y + item_h).min(screen_h as f32)
                });
                if hovered && !branch.is_current {
                    painter.rect_filled(item_rect, 4.0 * scale, c(SURFACE_HOVER));
                }
                let icon = if branch.is_current { "*" } else { " " };
                let name_color = if branch.is_current {
                    c(ACCENT)
                } else if hovered {
                    c(ACCENT)
                } else {
                    c(TEXT_C)
                };
                text.queue(
                    icon,
                    font,
                    pad + 10.0 * scale,
                    y + (item_h - font) / 2.0,
                    c(ACCENT),
                    16.0 * scale,
                    screen_w,
                    screen_h,
                );
                text.queue(
                    &branch.name,
                    font,
                    pad + 28.0 * scale,
                    y + (item_h - font) / 2.0,
                    name_color,
                    sw - pad * 2.0 - 28.0 * scale,
                    screen_w,
                    screen_h,
                );
                y += item_h;
            }
        }

        y += 4.0 * scale;
        divider(painter, y, sw, scale);
        y += 6.0 * scale;

        // ── 2. Commit section ───────────────────────────────────────
        text.queue(
            "COMMIT",
            small_font,
            pad,
            y + 6.0 * scale,
            c(TEXT_DIM),
            sw - pad * 2.0,
            screen_w,
            screen_h,
        );
        y += section_h;

        draw_commit_input(painter, text, state, sw, y, screen_w, screen_h, scale);
        y += input_h + 4.0 * scale;

        draw_button_at(
            painter,
            text,
            "Commit",
            pad,
            sw - pad * 2.0,
            y,
            screen_w,
            screen_h,
            cursor_pos,
            c(ACCENT),
            scale,
        );
        y += button_h;

        // Push / Pull side by side
        let half = (sw - pad * 3.0) / 2.0;
        draw_button_at(
            painter,
            text,
            "Push",
            pad,
            half,
            y,
            screen_w,
            screen_h,
            cursor_pos,
            c(BLUE),
            scale,
        );
        draw_button_at(
            painter,
            text,
            "Pull",
            pad * 2.0 + half,
            half,
            y,
            screen_w,
            screen_h,
            cursor_pos,
            c(BLUE),
            scale,
        );
        y += button_h + 4.0 * scale;

        divider(painter, y, sw, scale);
        y += 6.0 * scale;

        // ── 3. Changes section ──────────────────────────────────────
        let staged: Vec<&FileStatus> = status.files.iter().filter(|f| f.staged).collect();
        if !staged.is_empty() {
            text.queue(
                "STAGED",
                small_font,
                pad,
                y + 6.0 * scale,
                c(GREEN),
                sw - pad * 2.0,
                screen_w,
                screen_h,
            );
            y += section_h;
            for file in &staged {
                draw_file_item(
                    painter, text, file, sw, y, screen_w, screen_h, cursor_pos, scale,
                );
                y += item_h;
            }
        }

        let unstaged: Vec<&FileStatus> = status.files.iter().filter(|f| !f.staged).collect();
        if !unstaged.is_empty() {
            text.queue(
                "CHANGES",
                small_font,
                pad,
                y + 6.0 * scale,
                c(RED),
                sw - pad * 2.0,
                screen_w,
                screen_h,
            );
            y += section_h;
            for file in &unstaged {
                draw_file_item(
                    painter, text, file, sw, y, screen_w, screen_h, cursor_pos, scale,
                );
                y += item_h;
            }
        }

        if status.files.is_empty() {
            text.queue(
                "Clean working tree",
                font,
                pad,
                y + 6.0 * scale,
                c(TEXT_DIM),
                sw - pad * 2.0,
                screen_w,
                screen_h,
            );
            y += item_h;
        }

        // Stage All / Unstage All
        if !status.files.is_empty() {
            y += 4.0 * scale;
            let half = (sw - pad * 3.0) / 2.0;
            draw_button_at(
                painter,
                text,
                "Stage All",
                pad,
                half,
                y,
                screen_w,
                screen_h,
                cursor_pos,
                c(GREEN),
                scale,
            );
            draw_button_at(
                painter,
                text,
                "Unstage All",
                pad * 2.0 + half,
                half,
                y,
                screen_w,
                screen_h,
                cursor_pos,
                c(TEXT_DIM),
                scale,
            );
            y += button_h + 4.0 * scale;
        }

        divider(painter, y, sw, scale);
        y += 8.0 * scale;
    } else {
        text.queue(
            "No repo found",
            font,
            pad,
            y + 6.0 * scale,
            c(TEXT_DIM),
            sw - pad * 2.0,
            screen_w,
            screen_h,
        );
        y += item_h + 8.0 * scale;
    }

    // ── 4. Recent commits ───────────────────────────────────────────
    text.queue(
        "RECENT",
        small_font,
        pad,
        y + 6.0 * scale,
        c(TEXT_DIM),
        sw - pad * 2.0,
        screen_w,
        screen_h,
    );
    y += section_h;

    for commit in state.graph.iter().take(30) {
        let has_deco = !commit.decorations.is_empty();
        let row_h = if has_deco {
            item_h + small_font
        } else {
            item_h
        };

        text.queue(
            &commit.short_hash,
            small_font,
            pad,
            y + 6.0 * scale,
            c(BLUE),
            60.0 * scale,
            screen_w,
            screen_h,
        );
        text.queue(
            &commit.subject,
            small_font,
            pad + 65.0 * scale,
            y + 6.0 * scale,
            c(TEXT_C),
            sw - pad - 65.0 * scale,
            screen_w,
            screen_h,
        );
        if has_deco {
            let deco = commit.decorations.join(", ");
            text.queue(
                &deco,
                small_font - 2.0 * scale,
                pad + 65.0 * scale,
                y + 6.0 * scale + small_font + 2.0 * scale,
                c(ACCENT),
                sw - pad - 65.0 * scale,
                screen_w,
                screen_h,
            );
        }
        y += row_h;
    }

    painter.pop_clip();

    // ── Status toast (auto-sized, anchored to bottom) ───────────────
    // Multi-line errors (like `git push` output) used to be clipped to a fixed
    // 32px pill, and commit text above leaked into the same band of pixels.
    // We now: (1) auto-size the toast to fit wrapped content, (2) tell the
    // text renderer to occlude any earlier text that overlaps the toast, so
    // the message is the only thing readable in that region.
    if let Some((ref msg, is_error)) = state.message {
        let inner_w = sw - pad * 2.0;
        // Crude width estimator at small_font — overshoots slightly, which is
        // fine because over-estimating height never causes overlap.
        let chars_per_line = ((inner_w / (small_font * 0.55)).floor() as usize).max(1);
        let mut line_count = 0usize;
        for line in msg.lines() {
            let n = line.chars().count().max(1);
            line_count += n.div_ceil(chars_per_line);
        }
        if line_count == 0 {
            line_count = 1;
        }
        line_count = line_count.min(8); // cap so a wall-of-text error doesn't take the whole sidebar

        let vpad = 10.0 * scale;
        let line_h = small_font + 2.0 * scale;
        let toast_h = (line_count as f32 * line_h + vpad * 2.0).max(32.0 * scale);
        let toast_y = screen_h as f32 - toast_h;
        let bg_color = if is_error { c(RED) } else { c(GREEN) };

        // Hide whatever was queued earlier (recent commits, branch list, etc.)
        // anywhere it overlaps the toast region.
        text.occlude_rect([0.0, toast_y, sw, toast_h]);

        painter.rect_filled(Rect::new(0.0, toast_y, sw, toast_h), 0.0, bg_color);
        // Push a clip that's tall enough for the wrapped lines — the default
        // (~one font-height below y) would chop everything past line 1.
        text.push_clip([0.0, toast_y, sw, toast_h]);
        text.queue(
            msg,
            small_font,
            pad,
            toast_y + vpad,
            c(Color8::from_rgb(255, 255, 255)),
            inner_w,
            screen_w,
            screen_h,
        );
        text.pop_clip();
    }
}

// ── Drawing helpers ─────────────────────────────────────────────────────────

fn divider(painter: &mut Painter, y: f32, sw: f32, scale: f32) {
    let pad = PAD * scale;
    painter.rect_filled(
        Rect::new(pad, y, sw - pad * 2.0, 1.0 * scale),
        0.0,
        c(DIVIDER),
    );
}

fn draw_file_item(
    painter: &mut Painter,
    text: &mut TextRenderer,
    file: &FileStatus,
    sw: f32,
    y: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    scale: f32,
) {
    let item_h = ITEM_H * scale;
    let font = FONT * scale;
    let pad = PAD * scale;
    let item_rect = Rect::new(4.0 * scale, y, sw - 8.0 * scale, item_h);
    let hovered = cursor_pos.map_or(false, |(cx, cy)| {
        cx >= 0.0 && cx <= sw && cy >= y && cy < y + item_h
    });
    if hovered {
        painter.rect_filled(item_rect, 4.0 * scale, c(SURFACE_HOVER));
    }

    let status_color = match file.status {
        FileState::Modified => c(ACCENT),
        FileState::Added => c(GREEN),
        FileState::Deleted => c(RED),
        FileState::Renamed => c(BLUE),
        FileState::Untracked => c(TEXT_DIM),
    };
    let label = file.status.label();
    text.queue(
        label,
        font,
        pad,
        y + (item_h - font) / 2.0,
        status_color,
        20.0 * scale,
        screen_w,
        screen_h,
    );

    let dot = if file.staged { "+" } else { " " };
    let dot_color = if file.staged { c(GREEN) } else { c(TEXT_DIM) };
    text.queue(
        dot,
        font,
        pad + 20.0 * scale,
        y + (item_h - font) / 2.0,
        dot_color,
        14.0 * scale,
        screen_w,
        screen_h,
    );

    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let name_color = if hovered { c(ACCENT) } else { c(TEXT_C) };
    text.queue(
        name,
        font,
        pad + 36.0 * scale,
        y + (item_h - font) / 2.0,
        name_color,
        sw - pad - 36.0 * scale,
        screen_w,
        screen_h,
    );
}

fn draw_button_at(
    painter: &mut Painter,
    text: &mut TextRenderer,
    label: &str,
    x: f32,
    w: f32,
    y: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    label_color: Color,
    scale: f32,
) {
    let button_h = BUTTON_H * scale;
    let font = FONT * scale;
    let btn = Rect::new(x, y, w, button_h - 4.0 * scale);
    let hovered = cursor_pos.map_or(false, |(cx, cy)| {
        cx >= btn.x && cx <= btn.x + btn.w && cy >= btn.y && cy <= btn.y + btn.h
    });
    let bg = if hovered {
        c(Color8::from_rgba(70, 70, 70, 255))
    } else {
        c(BTN_BG)
    };
    painter.rect_filled(btn, 6.0 * scale, bg);
    let text_w = label.len() as f32 * font * 0.55;
    let tx = x + (w - text_w) / 2.0;
    text.queue(
        label,
        font,
        tx,
        y + (button_h - 4.0 * scale - font) / 2.0,
        label_color,
        w,
        screen_w,
        screen_h,
    );
}

fn draw_commit_input(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &GitSidebarState,
    sw: f32,
    y: f32,
    screen_w: u32,
    screen_h: u32,
    scale: f32,
) {
    let pad = PAD * scale;
    let input_h = INPUT_H * scale;
    let font = FONT * scale;
    let char_w = CHAR_W * scale;
    let x = pad;
    let w = sw - pad * 2.0;
    let inner_w = w - 16.0 * scale; // text area inside the border padding
    let r = Rect::new(x, y, w, input_h - 4.0 * scale);

    // Background
    painter.rect_filled(r, 4.0 * scale, c(Color8::from_rgba(40, 40, 40, 255)));

    // Border
    let border_color = if state.commit_focused {
        c(ACCENT)
    } else {
        c(Color8::from_rgba(80, 80, 80, 255))
    };
    let b = 1.5 * scale;
    painter.rect_filled(Rect::new(r.x, r.y, r.w, b), 0.0, border_color);
    painter.rect_filled(Rect::new(r.x, r.y + r.h - b, r.w, b), 0.0, border_color);
    painter.rect_filled(Rect::new(r.x, r.y, b, r.h), 0.0, border_color);
    painter.rect_filled(Rect::new(r.x + r.w - b, r.y, b, r.h), 0.0, border_color);

    let ty = y + (input_h - 4.0 * scale - font) / 2.0;

    if state.commit_msg.is_empty() && !state.commit_focused {
        text.queue(
            "commit message...",
            font,
            x + 8.0 * scale,
            ty,
            c(TEXT_DIM),
            inner_w,
            screen_w,
            screen_h,
        );
        return;
    }

    // Scroll the text so the cursor is always visible
    let visible_chars = (inner_w / char_w) as usize;
    let scroll_chars = if state.commit_cursor >= visible_chars {
        state.commit_cursor - visible_chars + 1
    } else {
        0
    };

    // Clip text to input bounds
    painter.push_clip(Rect::new(x + 4.0 * scale, r.y, w - 8.0 * scale, r.h));

    let display = if scroll_chars < state.commit_msg.len() {
        &state.commit_msg[scroll_chars..]
    } else {
        ""
    };
    text.queue(
        display,
        font,
        x + 8.0 * scale,
        ty,
        c(TEXT_C),
        inner_w + 200.0 * scale,
        screen_w,
        screen_h,
    );

    // Cursor
    if state.commit_focused {
        let cursor_x = x + 8.0 * scale + (state.commit_cursor - scroll_chars) as f32 * char_w;
        painter.rect_filled(
            Rect::new(cursor_x, ty, 2.0 * scale, font + 2.0 * scale),
            0.0,
            c(TEXT_C),
        );
    }

    painter.pop_clip();
}
