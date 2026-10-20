//! What the picker and the badge share: the desktop's look, and how their
//! surfaces are made.

use lntrn_app::AppConfig;
use lntrn_kit::desktop::Desktop;
use lntrn_kit::look;
use lntrn_math::Color;
use lntrn_ui::{Anchor, KeyboardMode, Layer, LayerConfig, Margin, Theme};

pub const APP_ID: &str = "lntrn-screen-capture";
/// What the compositor knows the picker's surfaces as.
pub const PICKER_NAMESPACE: &str = "lntrn-screen-capture";
/// The badge's: the compositor shows it but leaves it out of every copy
/// of the screen, so it never records itself (`NO_CAPTURE_NAMESPACE` in
/// the compositor's `screencopy_render.rs`).
pub const BADGE_NAMESPACE: &str = "lntrn-screen-capture-badge";

/// The kit's theme in the desktop's accent, with nothing of the shell's
/// own painted: the surfaces are see-through but for what we draw.
pub fn theme(desktop: &Desktop) -> Theme {
    Theme { bg: Color::TRANSPARENT, border_width: 0.0, ..look::theme(desktop.accent) }
}

/// The app's window, made as the surface `layer` says.
pub fn config(desktop: &Desktop, layer: LayerConfig) -> AppConfig {
    AppConfig {
        title: "Screen Capture".into(),
        app_id: APP_ID.into(),
        layer: Some(layer),
        title_bar: false,
        sans: desktop.font.clone(),
        transparent: true,
        // Nothing of the shell's to keep between runs.
        persist: false,
        ..AppConfig::default()
    }
}

/// The picker on the monitor called `output`: all of it, over everything,
/// taking the keyboard for Enter and Escape.
pub fn picker_layer(output: &str) -> LayerConfig {
    LayerConfig::new(PICKER_NAMESPACE, (0, 0)).layer(Layer::Overlay).anchor(Anchor::ALL).exclusive(-1).keyboard(KeyboardMode::Exclusive).output(output)
}

/// The badge, `size` logical pixels in the top right corner of the
/// monitor called `output`, leaving the keyboard where it is.
pub fn badge_layer(output: &str, size: (u32, u32)) -> LayerConfig {
    LayerConfig::new(BADGE_NAMESPACE, size).layer(Layer::Overlay).anchor(Anchor::TOP_RIGHT).margin(Margin { top: 16, right: 16, bottom: 0, left: 0 }).keyboard(KeyboardMode::None).output(output)
}
