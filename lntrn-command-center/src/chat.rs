//! Claude-powered chatbot tab. v1 shell — visible empty state. Real
//! API + streaming + persistence + markdown landing in later phases.

#![allow(dead_code)] // fields wired up across Phases 2-6

use lntrn_render::{Color, Painter, Rect, TextRenderer};

const TEXT_RGB: (u8, u8, u8) = (0xff, 0xff, 0xff);
const HINT_ALPHA: f32 = 0.55;

/// One message in the active chat thread.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    /// Plain text for now — markdown blocks come in Phase 5.
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Default)]
pub struct ChatState {
    pub threads: Vec<Thread>,
    /// Index into `threads` of the active thread. None when no threads yet.
    pub active: Option<usize>,
    /// Draft message the user is composing.
    pub draft: String,
    /// True while a streaming response is in flight.
    pub streaming: bool,
    /// API key resolved from lntrn-keychain at startup; None when missing.
    pub api_key: Option<String>,
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }
}

fn text_color(alpha: f32) -> Color {
    Color::from_rgb8(TEXT_RGB.0, TEXT_RGB.1, TEXT_RGB.2).with_alpha(alpha)
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    state: &ChatState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let pad = 24.0 * scale;
    let font = text_size * scale;
    let line_h = font * 1.4;

    let body_left = panel.x + pad;
    let body_top = top_y + pad;
    let max_w = panel.w - pad * 2.0;

    let (title, hint) = if state.api_key.is_none() {
        (
            "Chat — set your API key first",
            "Run: lntrn-keychain set anthropic_api_key <your-key>",
        )
    } else if state.threads.is_empty() {
        ("Chat", "Start a new conversation — type below and hit Enter.")
    } else {
        ("Chat", "(message list lands in Phase 6)")
    };

    text.queue(
        title,
        font * 1.2,
        body_left,
        body_top,
        text_color(alpha),
        max_w,
        surface_w,
        surface_h,
    );
    text.queue(
        hint,
        font,
        body_left,
        body_top + line_h * 1.6,
        text_color(HINT_ALPHA * alpha),
        max_w,
        surface_w,
        surface_h,
    );
}
