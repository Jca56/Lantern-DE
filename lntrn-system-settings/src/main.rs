//! Lantern System Settings.
//!
//! ## Module layout
//!
//! **Wayland host** — `main.rs`, `wayland.rs`, `wayland_state.rs`,
//! `popup_backend.rs`, `chrome.rs`, `click_router.rs`. The event loop,
//! dispatch impls, window chrome, and the click-dispatch router.
//!
//! **Shared panel infra** — `panels.rs` owns the layout constants
//! (`ROW_H`, `CARD_*`, etc.), the `GLOW_COLORS` palette, `PanelState`,
//! and the common helpers (`draw_section_card`, `draw_color_swatch_row`,
//! `slider_value_from_cursor`, `draw_save_cancel_bar`, etc.).
//!
//! **Per-panel modules** — `appearance_panel.rs` (with its sibling section
//! modules `appearance_layout.rs`, `appearance_animations.rs`, and
//! `appearance_focus.rs`), `display_panel.rs`, `input_panel.rs`,
//! `power_panel.rs`, `notifications_panel.rs`, `icon_panel.rs`. Each panel
//! exports:
//!   * `draw_<name>_panel(...)` — renders the panel
//!   * `handle_<name>_click(config, zone_id, ...)` — mutates config
//!   * Private `ZONE_*: u32` constants for hit zones
//!
//! **Display sub-system** — `monitor_settings.rs`, `monitor_arrange.rs`,
//! `output_manager.rs`, `wallpaper_picker.rs`.
//!
//! ## Adding a new panel
//! 1. Create `<name>_panel.rs` with the two functions above.
//! 2. Add `mod <name>_panel;` here.
//! 3. Add a variant to `Panel` + an entry in `PANELS` in `wayland.rs`.
//! 4. Wire `draw_*` into `wayland.rs::run()` and `handle_*` into
//!    `click_router::route_panel_click`.
//! 5. Add any new config fields to `config.rs` (with `sanitize` clamps).

mod appearance_animations;
mod appearance_focus;
mod appearance_layout;
mod appearance_panel;
mod appearance_themes;
mod chrome;
mod click_router;
mod config;
mod display_panel;
mod monitor_settings;
mod output_manager;
mod icon_panel;
mod icons;
mod input_panel;
mod monitor_arrange;
mod notifications_panel;
mod panels;
mod popup_backend;
mod power_panel;
mod sidebar;
mod test_window;
mod text_edit;
mod themes;
mod wallpaper_picker;
mod wayland;
mod wayland_state;

fn main() {
    // --test-window spawns a minimal 500x500 blank window so the user can
    // see how the WM-tab sliders (titlebar height, corner radius, border
    // width) affect a real SSD-decorated window. Every regular Lantern app
    // uses CSD, so they don't react to these settings.
    if std::env::args().any(|a| a == "--test-window") {
        if let Err(e) = test_window::run() {
            eprintln!("[test-window] fatal: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = wayland::run() {
        eprintln!("[system-settings] fatal: {e}");
        std::process::exit(1);
    }
}
