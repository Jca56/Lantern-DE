//! The look and the widgets Lantern apps on Lantern UI 2 share, so
//! Settings, Git and whatever is rebuilt next are one family: the same
//! palette, the same cards, the same switches.
//!
//! Everything is drawn from Lantern UI's paint and interact primitives; an
//! app declares it top to bottom like any other widget code:
//!
//! ```ignore
//! kit::page(ui, "Mouse", "Pointer, scrolling and clicking.", |ui| {
//!     kit::caption(ui, "Pointer");
//!     kit::card(ui, "pointer", |c| {
//!         changed |= c.switch("Acceleration", "A faster flick travels further.", &mut on);
//!     });
//! });
//! ```
//!
//! - `look`: the palette, and the theme the shell's own parts wear.
//! - `desktop`: what an app follows from the desktop's settings (accent,
//!   window opacity, font).
//! - `layout`: the page column, captions and notes.
//! - `card`: cards and their rows.
//! - `controls`: switch, slider, segmented control, buttons, text field.
//! - `pickers`: the dropdown and the colour chip.
//! - `bits`: badges and banners.
//! - `nav`: a sidebar's shade, captions and rows.
//! - `probe`: what a headless test can ask about a layout.
//! - `startup`: where an app's panics and complaints go.

pub mod bits;
pub mod card;
pub mod controls;
pub mod desktop;
pub mod layout;
pub mod look;
pub mod nav;
pub mod pickers;
pub mod probe;
pub mod startup;

pub use card::{Card, card};
pub use layout::{Keep, caption, caption_action, elide, mono_style, note, page, percent, percent_of_100, pixels, small_style, text_line, times, title_style};
