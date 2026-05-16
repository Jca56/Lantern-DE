//! Chat tab rendering. Layout zones:
//!   ┌──────────┬──────────────────────────────────┐
//!   │ sidebar  │  header                          │
//!   │ (200dp)  │  ────────────────────────────────│
//!   │ threads  │  messages (scroll)               │
//!   │  list    │                                  │
//!   │          │  ────────────────────────────────│
//!   │          │  draft input + send hint         │
//!   └──────────┴──────────────────────────────────┘

use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};

use super::highlight::{lang_from_tag, tokenize, TokKind};
use super::markdown::{parse as parse_md, parse_inlines, Block, Inline};
use super::{ChatState, Role};

// ── Palette ─────────────────────────────────────────────────────────────────

fn text_color() -> Color { Color::from_rgb8(0xe8, 0xdc, 0xc8) }
fn text_dim() -> Color { Color::from_rgb8(0xa8, 0x9c, 0x88) }
fn sidebar_bg() -> Color { Color::from_rgba8(0x14, 0x12, 0x0e, 0xc0) }
fn sidebar_border() -> Color { Color::from_rgba8(0x66, 0x58, 0x40, 0x90) }
fn user_bubble() -> Color { Color::from_rgba8(0x3a, 0x2e, 0x1a, 0xe8) }
fn assist_bubble() -> Color { Color::from_rgba8(0x1c, 0x18, 0x12, 0xb0) }
fn code_bg() -> Color { Color::from_rgba8(0x0c, 0x0a, 0x08, 0xe0) }
fn code_border() -> Color { Color::from_rgba8(0x6c, 0x58, 0x3a, 0xa0) }
fn inline_code_bg() -> Color { Color::from_rgba8(0x2a, 0x22, 0x18, 0xc0) }
fn err_text() -> Color { Color::from_rgb8(0xff, 0x9c, 0x60) }
fn accent() -> Color { Color::from_rgb8(0xe2, 0xa8, 0x4a) }
fn input_bg() -> Color { Color::from_rgba8(0x0e, 0x0c, 0x0a, 0xd8) }
fn input_border() -> Color { Color::from_rgba8(0x6c, 0x58, 0x3a, 0xa0) }
fn active_thread() -> Color { Color::from_rgba8(0xe2, 0xa8, 0x4a, 0x35) }
fn hover_bg() -> Color { Color::from_rgba8(0xe8, 0xdc, 0xc8, 0x18) }
fn send_text() -> Color { Color::from_rgb8(0x14, 0x10, 0x08) }

fn kw_color() -> Color { Color::from_rgb8(0xe6, 0x90, 0x4a) }
fn ty_color() -> Color { Color::from_rgb8(0xe2, 0xc4, 0x5e) }
fn str_color() -> Color { Color::from_rgb8(0x9c, 0xc4, 0x70) }
fn num_color() -> Color { Color::from_rgb8(0xc0, 0x9e, 0xff) }
fn comment_color() -> Color { Color::from_rgb8(0x70, 0x80, 0x90) }
fn builtin_color() -> Color { Color::from_rgb8(0x80, 0xc8, 0xc8) }
fn punct_color() -> Color { Color::from_rgb8(0xc8, 0xb8, 0xa0) }

pub const SIDEBAR_W: f32 = 220.0;
pub const INPUT_H: f32 = 100.0;
pub const PAD: f32 = 18.0;
pub const HEADER_H: f32 = 44.0;
pub const THREAD_ROW_H: f32 = 56.0;
pub const NEW_BTN_H: f32 = 44.0;
pub const BUBBLE_GAP: f32 = 12.0;

fn alpha(c: Color, a: f32) -> Color { c.with_alpha(c.a * a) }

// ── Layout helpers ──────────────────────────────────────────────────────────

pub struct Layout {
    pub sidebar: Rect,
    pub new_btn: Rect,
    pub threads_clip: Rect,
    pub header: Rect,
    pub messages_clip: Rect,
    pub input: Rect,
    pub send_btn: Rect,
}

pub fn layout(panel: Rect, top_y: f32, scale: f32) -> Layout {
    let sb_w = SIDEBAR_W * scale;
    let input_h = INPUT_H * scale;
    let header_h = HEADER_H * scale;
    let new_h = NEW_BTN_H * scale;
    let pad = PAD * scale;

    let sidebar = Rect::new(panel.x, top_y, sb_w, panel.h - (top_y - panel.y));
    let new_btn = Rect::new(sidebar.x + pad / 2.0, sidebar.y + pad / 2.0,
                            sidebar.w - pad, new_h);
    let threads_clip = Rect::new(sidebar.x, new_btn.y + new_btn.h + pad / 2.0,
                                 sidebar.w, sidebar.h - (new_btn.h + pad));

    let main_x = sidebar.x + sidebar.w;
    let main_w = panel.w - sidebar.w;
    let header = Rect::new(main_x, top_y, main_w, header_h);

    let input = Rect::new(main_x + pad, panel.y + panel.h - input_h - pad,
                          main_w - pad * 2.0, input_h);
    let send_w = 120.0 * scale;
    let send_btn = Rect::new(input.x + input.w - send_w, input.y + input.h - 36.0 * scale,
                             send_w, 32.0 * scale);
    let messages_clip = Rect::new(main_x, header.y + header.h,
                                  main_w, input.y - (header.y + header.h));

    Layout { sidebar, new_btn, threads_clip, header, messages_clip, input, send_btn }
}

/// Hit-test the sidebar: returns either a thread row click or X-button click.
pub enum ThreadHit {
    Row(usize),
    Delete(usize),
}

pub fn thread_hit_test(
    l: &Layout,
    scroll: f32,
    n_threads: usize,
    scale: f32,
    phys_x: f32,
    phys_y: f32,
) -> Option<ThreadHit> {
    if !l.threads_clip.contains(phys_x, phys_y) { return None; }
    let row_h = THREAD_ROW_H * scale;
    let rel = phys_y - l.threads_clip.y + scroll;
    let idx = (rel / row_h) as usize;
    if idx >= n_threads { return None; }
    let y = l.threads_clip.y + idx as f32 * row_h - scroll;
    let row = Rect::new(l.threads_clip.x + 8.0 * scale, y + 2.0 * scale,
                        l.threads_clip.w - 16.0 * scale, row_h - 4.0 * scale);
    let xb = x_button_rect(row, scale);
    if xb.contains(phys_x, phys_y) {
        Some(ThreadHit::Delete(idx))
    } else {
        Some(ThreadHit::Row(idx))
    }
}

fn x_button_rect(row: Rect, scale: f32) -> Rect {
    let s = 28.0 * scale;
    Rect::new(row.x + row.w - s - 6.0 * scale,
              row.y + (row.h - s) / 2.0,
              s, s)
}

// ── Public entrypoint ───────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    state: &mut ChatState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha_panel: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let l = layout(panel, top_y, scale);
    let font = text_size * scale;
    let mono_font = (text_size - 1.0) * scale;
    let line_h = font * 1.45;

    // Sidebar background.
    painter.rect_filled(l.sidebar, 0.0, alpha(sidebar_bg(), alpha_panel));
    painter.rect_filled(
        Rect::new(l.sidebar.x + l.sidebar.w - 1.0 * scale, l.sidebar.y,
                  1.0 * scale, l.sidebar.h),
        0.0, alpha(sidebar_border(), alpha_panel),
    );

    draw_new_button(painter, text, state, &l, scale, font, alpha_panel, surface_w, surface_h);
    draw_thread_list(painter, text, state, &l, scale, font, alpha_panel, surface_w, surface_h);

    draw_header(text, state, &l, font, alpha_panel, surface_w, surface_h);
    draw_messages(
        painter, text, mono_text, state, &l, scale, font, mono_font, line_h,
        alpha_panel, surface_w, surface_h,
    );
    draw_input(painter, text, state, &l, scale, font, alpha_panel, surface_w, surface_h);

    if let Some(err) = state.last_error.as_ref() {
        let err_y = l.input.y - 28.0 * scale;
        text.queue(
            err, font * 0.85, l.input.x, err_y,
            alpha(err_text(), alpha_panel), l.input.w, surface_w, surface_h,
        );
    } else if let Some(err) = state.key_error.as_ref() {
        if state.api_key.is_none() {
            let err_y = l.input.y - 28.0 * scale;
            text.queue(
                err, font * 0.85, l.input.x, err_y,
                alpha(err_text(), alpha_panel), l.input.w, surface_w, surface_h,
            );
        }
    }
}

// ── Sidebar ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_new_button(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ChatState,
    l: &Layout,
    scale: f32,
    font: f32,
    a: f32,
    sw: u32,
    sh: u32,
) {
    let bg = if state.hover_new_thread { alpha(accent(), a * 0.4) } else { alpha(user_bubble(), a) };
    painter.rect_filled(l.new_btn, 10.0 * scale, bg);
    painter.rect_stroke(l.new_btn, 10.0 * scale, 1.0 * scale, alpha(sidebar_border(), a));
    let label = "+ New chat";
    let lw = text.measure_width(label, font);
    text.queue(
        label, font,
        l.new_btn.x + (l.new_btn.w - lw) / 2.0,
        l.new_btn.y + (l.new_btn.h - font * 1.2) / 2.0,
        alpha(text_color(), a), l.new_btn.w, sw, sh,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_thread_list(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &mut ChatState,
    l: &Layout,
    scale: f32,
    font: f32,
    a: f32,
    sw: u32,
    sh: u32,
) {
    let clip = l.threads_clip;
    painter.push_clip(clip);
    text.push_clip([clip.x, clip.y, clip.w, clip.h]);

    let row_h = THREAD_ROW_H * scale;
    let total = state.threads.len() as f32 * row_h;
    state.sidebar_scroll_max = (total - clip.h).max(0.0);
    if state.sidebar_scroll > state.sidebar_scroll_max {
        state.sidebar_scroll = state.sidebar_scroll_max;
    }
    let scroll = state.sidebar_scroll;

    let confirm = state.confirm_delete;
    let active_idx = state.active;
    for (i, t) in state.threads.iter().enumerate() {
        let y = clip.y + i as f32 * row_h - scroll;
        if y + row_h < clip.y || y > clip.y + clip.h { continue; }
        let row = Rect::new(clip.x + 8.0 * scale, y + 2.0 * scale,
                            clip.w - 16.0 * scale, row_h - 4.0 * scale);
        let is_active = active_idx == Some(i);
        let is_hover = state.hover_thread == Some(i);
        if is_active {
            painter.rect_filled(row, 8.0 * scale, alpha(active_thread(), a));
        } else if is_hover {
            painter.rect_filled(row, 8.0 * scale, alpha(hover_bg(), a));
        }

        let x_btn = x_button_rect(row, scale);
        let pending_delete = confirm == Some(i);
        let title_max_w = (x_btn.x - 12.0 * scale) - (row.x + 12.0 * scale);
        text.queue(
            &t.title, font, row.x + 12.0 * scale, row.y + 8.0 * scale,
            alpha(text_color(), a), title_max_w, sw, sh,
        );
        let count = t.messages.len();
        let sub = if pending_delete {
            "click ✕ again to delete".to_string()
        } else if count == 0 {
            "empty".to_string()
        } else if count == 1 {
            "1 message".to_string()
        } else {
            format!("{count} messages")
        };
        let sub_color = if pending_delete { err_text() } else { text_dim() };
        text.queue(
            &sub, font * 0.78, row.x + 12.0 * scale, row.y + 28.0 * scale,
            alpha(sub_color, a), title_max_w, sw, sh,
        );

        // X delete button — red on confirm, dim otherwise.
        let x_color = if pending_delete { err_text() } else { text_dim() };
        let x_bg = if pending_delete { alpha(err_text(), a * 0.18) } else { alpha(hover_bg(), a * 0.4) };
        painter.rect_filled(x_btn, 6.0 * scale, x_bg);
        let xw = text.measure_width("✕", font * 0.85);
        text.queue(
            "✕", font * 0.85,
            x_btn.x + (x_btn.w - xw) / 2.0,
            x_btn.y + (x_btn.h - font * 0.85 * 1.2) / 2.0,
            alpha(x_color, a), x_btn.w, sw, sh,
        );
    }

    painter.pop_clip();
    text.pop_clip();
}

// ── Header ──────────────────────────────────────────────────────────────────

fn draw_header(
    text: &mut TextRenderer,
    state: &ChatState,
    l: &Layout,
    font: f32,
    a: f32,
    sw: u32,
    sh: u32,
) {
    let title = state.active_thread().map(|t| t.title.as_str()).unwrap_or("Chat");
    text.queue(
        title, font * 1.1, l.header.x + 18.0, l.header.y + (l.header.h - font * 1.2) / 2.0,
        alpha(text_color(), a), l.header.w - 36.0, sw, sh,
    );
    if state.streaming {
        text.queue(
            "● streaming…", font * 0.85,
            l.header.x + l.header.w - 160.0,
            l.header.y + (l.header.h - font) / 2.0,
            alpha(accent(), a), 140.0, sw, sh,
        );
    }
}

// ── Messages ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_messages(
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    state: &mut ChatState,
    l: &Layout,
    scale: f32,
    font: f32,
    mono_font: f32,
    line_h: f32,
    a: f32,
    sw: u32,
    sh: u32,
) {
    let clip = l.messages_clip;
    painter.push_clip(clip);
    text.push_clip([clip.x, clip.y, clip.w, clip.h]);
    mono_text.push_clip([clip.x, clip.y, clip.w, clip.h]);

    // Home view — no thread selected.
    if state.active.is_none() {
        draw_home(text, state, clip, scale, font, a, sw, sh);
        painter.pop_clip();
        text.pop_clip();
        mono_text.pop_clip();
        return;
    }

    let pad = 16.0 * scale;
    let bubble_max_w = (clip.w * 0.78).min(900.0 * scale);

    // First measure pass to know total height, then offset by scroll.
    let mut y = pad;
    let mut entries: Vec<MsgEntry> = Vec::new();

    if let Some(thread) = state.active_thread() {
        for m in &thread.messages {
            let blocks = if m.role == Role::Assistant {
                parse_md(&m.content)
            } else {
                vec![Block::Paragraph(parse_inlines(&m.content))]
            };
            let h = measure_blocks(&blocks, bubble_max_w - pad * 2.0, font, mono_font, line_h, text, mono_text);
            entries.push(MsgEntry { role: m.role, blocks, h });
            y += h + pad * 2.0 + BUBBLE_GAP * scale;
        }
    }

    // streaming "pending" bubble
    if state.streaming {
        let blocks = parse_md(&state.pending);
        let h = measure_blocks(&blocks, bubble_max_w - pad * 2.0, font, mono_font, line_h, text, mono_text)
            .max(line_h);
        entries.push(MsgEntry { role: Role::Assistant, blocks, h });
        y += h + pad * 2.0 + BUBBLE_GAP * scale;
    }

    let total = y;
    state.messages_scroll_max = (total - clip.h).max(0.0);
    if state.messages_scroll > state.messages_scroll_max {
        state.messages_scroll = state.messages_scroll_max;
    }
    if state.messages_scroll == f32::MAX {
        state.messages_scroll = state.messages_scroll_max;
    }
    let scroll = state.messages_scroll;

    // Empty state
    if entries.is_empty() && !state.streaming {
        let hint = if state.api_key.is_none() {
            "Set your API key in lntrn-keychain (name = \"Claude API\") and reopen this tab."
        } else {
            "Type below to start a new conversation."
        };
        text.queue(
            hint, font, clip.x + pad * 1.5, clip.y + pad * 2.0,
            alpha(text_dim(), a), clip.w - pad * 3.0, sw, sh,
        );
    }

    // Draw pass.
    let mut cur_y = clip.y + pad - scroll;
    for entry in entries.iter() {
        let bubble_h = entry.h + pad * 2.0;
        let is_user = entry.role == Role::User;
        let bx = if is_user {
            clip.x + clip.w - bubble_max_w - pad
        } else {
            clip.x + pad
        };
        let bubble = Rect::new(bx, cur_y, bubble_max_w, bubble_h);

        if cur_y + bubble_h > clip.y && cur_y < clip.y + clip.h {
            let bg = if is_user { user_bubble() } else { assist_bubble() };
            painter.rect_filled(bubble, 14.0 * scale, alpha(bg, a));
            let inner_x = bubble.x + pad;
            let inner_y = bubble.y + pad;
            let inner_w = bubble.w - pad * 2.0;
            draw_blocks(
                painter, text, mono_text, &entry.blocks,
                inner_x, inner_y, inner_w, scale, font, mono_font, line_h,
                a, sw, sh,
            );
        }
        cur_y += bubble_h + BUBBLE_GAP * scale;
    }

    painter.pop_clip();
    text.pop_clip();
    mono_text.pop_clip();
}

struct MsgEntry {
    role: Role,
    blocks: Vec<Block>,
    h: f32,
}

#[allow(clippy::too_many_arguments)]
fn draw_home(
    text: &mut TextRenderer,
    state: &ChatState,
    clip: Rect,
    scale: f32,
    font: f32,
    a: f32,
    sw: u32,
    sh: u32,
) {
    let pad = 32.0 * scale;
    let cx = clip.x + clip.w / 2.0;
    let mut y = clip.y + clip.h * 0.18;

    let title = "Lantern Chat";
    let tf = font * 2.2;
    let tw = text.measure_width_styled(title, tf, FontWeight::Bold, FontStyle::Normal);
    text.queue_styled(
        title, tf, cx - tw / 2.0, y,
        alpha(accent(), a), tw + 8.0,
        FontWeight::Bold, FontStyle::Normal, sw, sh,
    );
    y += tf * 1.6;

    let sub = if state.api_key.is_none() {
        "Set your API key in lntrn-keychain (name = \"Claude API\") to begin."
    } else if state.threads.is_empty() {
        "Pick \"+ New chat\" on the left to start your first conversation."
    } else {
        "Pick a conversation on the left, or start a new one with + New chat."
    };
    let sw_text = text.measure_width(sub, font);
    text.queue(
        sub, font,
        cx - sw_text / 2.0, y,
        alpha(text_color(), a), sw_text + 8.0, sw, sh,
    );
    y += font * 2.6;

    if !state.threads.is_empty() {
        let header_label = "Recent";
        let hf = font * 0.95;
        let hw = text.measure_width(header_label, hf);
        text.queue(
            header_label, hf,
            cx - hw / 2.0, y,
            alpha(text_dim(), a), hw + 8.0, sw, sh,
        );
        y += hf * 1.8;
        for t in state.threads.iter().take(5) {
            let label = format!("· {}", t.title);
            let lw = text.measure_width(&label, font);
            text.queue(
                &label, font,
                cx - lw / 2.0, y,
                alpha(text_color(), a), lw + 8.0, sw, sh,
            );
            y += font * 1.5;
        }
    }

    let model_hint = format!("model · {}", super::api::MODEL);
    let mhw = text.measure_width(&model_hint, font * 0.78);
    text.queue(
        &model_hint, font * 0.78,
        cx - mhw / 2.0, clip.y + clip.h - pad,
        alpha(text_dim(), a), mhw + 8.0, sw, sh,
    );
}

// ── Block layout ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn measure_blocks(
    blocks: &[Block],
    max_w: f32,
    font: f32,
    mono_font: f32,
    line_h: f32,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
) -> f32 {
    let mut total = 0.0;
    let block_gap = line_h * 0.5;
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 { total += block_gap; }
        total += match b {
            Block::Paragraph(inlines) => measure_inlines(inlines, max_w, font, text, false),
            Block::Heading { level, inlines } => {
                let h_font = font * heading_scale(*level);
                let h_line = h_font * 1.35;
                measure_inlines_with(inlines, max_w, h_font, h_line, text, FontWeight::Bold)
            }
            Block::Code { body, .. } => measure_code(body, mono_font, line_h, mono_text),
            Block::Bullet(items) | Block::Numbered(items) => {
                let mut h = 0.0;
                for inlines in items {
                    h += measure_inlines(inlines, max_w - 24.0, font, text, false) + line_h * 0.1;
                }
                h
            }
            Block::Quote(inlines) => measure_inlines(inlines, max_w - 16.0, font, text, false),
            Block::Rule => line_h * 0.6,
        };
    }
    total
}

fn heading_scale(level: u8) -> f32 {
    match level { 1 => 1.5, 2 => 1.3, 3 => 1.18, 4 => 1.08, _ => 1.0 }
}

fn measure_inlines(
    inlines: &[Inline],
    max_w: f32,
    font: f32,
    text: &mut TextRenderer,
    _underline: bool,
) -> f32 {
    measure_inlines_with(inlines, max_w, font, font * 1.45, text, FontWeight::Normal)
}

fn measure_inlines_with(
    inlines: &[Inline],
    max_w: f32,
    font: f32,
    line_h: f32,
    text: &mut TextRenderer,
    _base_weight: FontWeight,
) -> f32 {
    let wrapped = wrap_inlines(inlines, max_w, font, text);
    (wrapped.len().max(1)) as f32 * line_h
}

fn measure_code(body: &str, _mono_font: f32, line_h: f32, _mono_text: &mut TextRenderer) -> f32 {
    let lines = body.split('\n').count().max(1);
    lines as f32 * line_h + 16.0
}

// Each rendered "line" is a vector of (style, text) segments.
#[derive(Clone)]
struct StyledRun {
    text: String,
    weight: FontWeight,
    style: FontStyle,
    is_code: bool,
}

type WrappedLine = Vec<StyledRun>;

fn wrap_inlines(
    inlines: &[Inline],
    max_w: f32,
    font: f32,
    text: &mut TextRenderer,
) -> Vec<WrappedLine> {
    let mut lines: Vec<WrappedLine> = vec![Vec::new()];
    let mut cur_w = 0.0;
    let space_w = text.measure_width(" ", font);

    for inl in inlines {
        let (s, weight, style, is_code) = match inl {
            Inline::Text(s) => (s.as_str(), FontWeight::Normal, FontStyle::Normal, false),
            Inline::Bold(s) => (s.as_str(), FontWeight::Bold, FontStyle::Normal, false),
            Inline::Italic(s) => (s.as_str(), FontWeight::Normal, FontStyle::Italic, false),
            Inline::Code(s) => (s.as_str(), FontWeight::Normal, FontStyle::Normal, true),
        };
        for tok in split_with_spaces(s) {
            let w = if tok == " " {
                space_w
            } else {
                text.measure_width_styled(tok, font, weight, style)
            };
            // explicit newline character — never appears here since inlines came from a single paragraph.
            if cur_w + w > max_w && cur_w > 0.0 && tok != " " {
                lines.push(Vec::new());
                cur_w = 0.0;
            }
            if cur_w == 0.0 && tok == " " { continue; }
            lines.last_mut().unwrap().push(StyledRun {
                text: tok.to_string(), weight, style, is_code,
            });
            cur_w += w;
        }
    }
    lines
}

/// Split into tokens where each token is either a single whitespace " "
/// or a non-whitespace run.
fn split_with_spaces(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            out.push(" ");
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(&s[start..i]);
        }
    }
    out
}

// ── Block drawing ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_blocks(
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    blocks: &[Block],
    x: f32, y_start: f32, w: f32,
    scale: f32, font: f32, mono_font: f32, line_h: f32,
    a: f32, sw: u32, sh: u32,
) {
    let block_gap = line_h * 0.5;
    let mut y = y_start;
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 { y += block_gap; }
        match b {
            Block::Paragraph(inlines) => {
                y = draw_styled_lines(text, painter, inlines, x, y, w, font, line_h, a, sw, sh, scale);
            }
            Block::Heading { level, inlines } => {
                let h_font = font * heading_scale(*level);
                let h_line = h_font * 1.35;
                y = draw_styled_lines_weight(
                    text, painter, inlines, x, y, w, h_font, h_line, a, sw, sh, scale, FontWeight::Bold,
                );
            }
            Block::Code { lang, body } => {
                let h = measure_code(body, mono_font, line_h, mono_text);
                let rect = Rect::new(x, y, w, h);
                painter.rect_filled(rect, 8.0 * scale, alpha(code_bg(), a));
                painter.rect_stroke(rect, 8.0 * scale, 1.0 * scale, alpha(code_border(), a));
                if !lang.is_empty() {
                    text.queue(
                        lang, font * 0.75,
                        x + w - 60.0 * scale, y + 4.0 * scale,
                        alpha(text_dim(), a), 60.0 * scale, sw, sh,
                    );
                }
                draw_code(mono_text, body, lang, x + 12.0, y + 8.0, mono_font, line_h, a, sw, sh);
                y += h;
            }
            Block::Bullet(items) => {
                for inlines in items {
                    text.queue("•", font, x, y, alpha(text_color(), a), 16.0 * scale, sw, sh);
                    y = draw_styled_lines(
                        text, painter, inlines,
                        x + 24.0 * scale, y, w - 24.0 * scale,
                        font, line_h, a, sw, sh, scale,
                    );
                    y += line_h * 0.1;
                }
            }
            Block::Numbered(items) => {
                for (i, inlines) in items.iter().enumerate() {
                    let marker = format!("{}.", i + 1);
                    text.queue(&marker, font, x, y, alpha(text_color(), a), 24.0 * scale, sw, sh);
                    y = draw_styled_lines(
                        text, painter, inlines,
                        x + 32.0 * scale, y, w - 32.0 * scale,
                        font, line_h, a, sw, sh, scale,
                    );
                    y += line_h * 0.1;
                }
            }
            Block::Quote(inlines) => {
                let bar = Rect::new(x, y, 3.0 * scale, line_h);
                painter.rect_filled(bar, 0.0, alpha(accent(), a * 0.6));
                y = draw_styled_lines(
                    text, painter, inlines,
                    x + 16.0 * scale, y, w - 16.0 * scale,
                    font, line_h, a, sw, sh, scale,
                );
            }
            Block::Rule => {
                let bar = Rect::new(x, y + line_h * 0.25, w, 1.0 * scale);
                painter.rect_filled(bar, 0.0, alpha(sidebar_border(), a));
                y += line_h * 0.5;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_styled_lines(
    text: &mut TextRenderer,
    painter: &mut Painter,
    inlines: &[Inline],
    x: f32, y: f32, w: f32,
    font: f32, line_h: f32,
    a: f32, sw: u32, sh: u32, scale: f32,
) -> f32 {
    draw_styled_lines_weight(text, painter, inlines, x, y, w, font, line_h, a, sw, sh, scale, FontWeight::Normal)
}

#[allow(clippy::too_many_arguments)]
fn draw_styled_lines_weight(
    text: &mut TextRenderer,
    painter: &mut Painter,
    inlines: &[Inline],
    x: f32, y: f32, w: f32,
    font: f32, line_h: f32,
    a: f32, sw: u32, sh: u32, scale: f32,
    base_weight: FontWeight,
) -> f32 {
    let lines = wrap_inlines(inlines, w, font, text);
    let mut cy = y;
    let space_w = text.measure_width(" ", font);
    for line in lines.iter() {
        let mut cx = x;
        for run in line {
            let weight = match (run.weight, base_weight) {
                (FontWeight::Bold, _) | (_, FontWeight::Bold) => FontWeight::Bold,
                _ => FontWeight::Normal,
            };
            let style = run.style;
            let tok_w = if run.text == " " {
                space_w
            } else {
                text.measure_width_styled(&run.text, font, weight, style)
            };
            if run.is_code && run.text != " " {
                let pad = 3.0 * scale;
                let bg = Rect::new(cx - pad, cy - 2.0 * scale, tok_w + pad * 2.0, font * 1.25);
                painter.rect_filled(bg, 4.0 * scale, alpha(inline_code_bg(), a));
            }
            text.queue_styled(
                &run.text, font, cx, cy,
                alpha(text_color(), a), tok_w + 4.0,
                weight, style, sw, sh,
            );
            cx += tok_w;
        }
        cy += line_h;
    }
    cy
}

#[allow(clippy::too_many_arguments)]
fn draw_code(
    mono_text: &mut TextRenderer,
    body: &str,
    lang: &str,
    x: f32, y: f32,
    font: f32, line_h: f32,
    a: f32, sw: u32, sh: u32,
) {
    let lang = lang_from_tag(lang);
    let toks = tokenize(body, lang);
    // Render each line as a sequence of colored tokens.
    let mut cur_x = x;
    let mut cur_y = y;
    let space_w = mono_text.measure_width(" ", font);
    for tok in toks {
        for (idx, piece) in tok.text.split_inclusive('\n').enumerate() {
            if idx > 0 {
                cur_x = x;
                cur_y += line_h;
            }
            let stripped = piece.trim_end_matches('\n');
            if stripped.is_empty() { continue; }
            let color = tok_color(tok.kind);
            let w = mono_text.measure_width(stripped, font);
            mono_text.queue(
                stripped, font, cur_x, cur_y,
                alpha(color, a), (w + 4.0).max(space_w),
                sw, sh,
            );
            cur_x += w;
        }
    }
}

fn tok_color(k: TokKind) -> Color {
    match k {
        TokKind::Keyword => kw_color(),
        TokKind::Type => ty_color(),
        TokKind::Builtin => builtin_color(),
        TokKind::String => str_color(),
        TokKind::Number => num_color(),
        TokKind::Comment => comment_color(),
        TokKind::Punct => punct_color(),
        TokKind::Plain => text_color(),
    }
}

// ── Input box ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_input(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ChatState,
    l: &Layout,
    scale: f32,
    font: f32,
    a: f32,
    sw: u32,
    sh: u32,
) {
    let pad = 12.0 * scale;
    painter.rect_filled(l.input, 14.0 * scale, alpha(input_bg(), a));
    painter.rect_stroke(l.input, 14.0 * scale, 1.0 * scale, alpha(input_border(), a));

    let inner_x = l.input.x + pad;
    let inner_y = l.input.y + pad;
    let inner_w = l.input.w - pad * 2.0 - 130.0 * scale;
    let q = state.draft.query();
    if q.is_empty() {
        let hint = if state.streaming {
            "Waiting for response…"
        } else if state.api_key.is_none() {
            "API key missing — see error below"
        } else {
            "Type your message…  (Enter to send, Shift+Enter for newline, Ctrl+N for new chat)"
        };
        text.queue(
            hint, font, inner_x, inner_y,
            alpha(text_dim(), a), inner_w, sw, sh,
        );
    } else {
        // Multi-line draft: split on \n and queue each line.
        for (i, line) in q.split('\n').enumerate() {
            text.queue(
                line, font, inner_x, inner_y + i as f32 * font * 1.4,
                alpha(text_color(), a), inner_w, sw, sh,
            );
        }
    }

    // Send pill.
    let send_color = if state.streaming || q.trim().is_empty() {
        alpha(user_bubble(), a * 0.6)
    } else if state.hover_send {
        alpha(accent(), a)
    } else {
        alpha(accent(), a * 0.85)
    };
    painter.rect_filled(l.send_btn, 10.0 * scale, send_color);
    let label = if state.streaming { "Sending…" } else { "Send ⏎" };
    let lw = text.measure_width(label, font);
    text.queue(
        label, font,
        l.send_btn.x + (l.send_btn.w - lw) / 2.0,
        l.send_btn.y + (l.send_btn.h - font * 1.2) / 2.0,
        alpha(send_text(), a), l.send_btn.w, sw, sh,
    );
}
