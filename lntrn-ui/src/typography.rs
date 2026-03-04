// ── Font size presets ─────────────────────────────────────────────────────────

pub const FONT_HEADING: f32 = 24.0;
pub const FONT_SUBHEADING: f32 = 20.0;
pub const FONT_TAB: f32 = 18.0;
pub const FONT_BODY: f32 = 16.0;
pub const FONT_SMALL: f32 = 14.0;
pub const FONT_ICON: f32 = 22.0;
pub const FONT_STATUS: f32 = 13.5;

// ── Font families ────────────────────────────────────────────────────────────

pub const FAMILY_PROPORTIONAL: &str = "Ubuntu";
pub const FAMILY_MONOSPACE: &str = "JetBrains Mono";

// ── Text scale ───────────────────────────────────────────────────────────────

use std::sync::atomic::{AtomicU32, Ordering};

static TEXT_SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0f32 as bits

/// Set the global text scale factor (1.0 = default, 1.25 = 125%, etc.)
pub fn set_text_scale(scale: f32) {
    TEXT_SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

/// Get the current global text scale factor.
pub fn text_scale() -> f32 {
    f32::from_bits(TEXT_SCALE.load(Ordering::Relaxed))
}

/// Apply text scale to a base size: `ts(FONT_BODY)` returns the scaled body font size.
pub fn ts(base: f32) -> f32 {
    base * text_scale()
}
