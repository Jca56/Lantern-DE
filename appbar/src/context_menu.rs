use std::sync::{Arc, Mutex};

use eframe::egui::{self, RichText};

use crate::config::{AppbarConfig, BarPosition};
use crate::theme;

// ── Viewport dimensions ─────────────────────────────────────────────────────

pub const MENU_WIDTH: f32 = 200.0;
pub const MENU_HEIGHT: f32 = 280.0;

// ── Actions ──────────────────────────────────────────────────────────────────

pub enum MenuAction {
    SetHeight(u32),
    SetPosition(BarPosition),
    ToggleAutoHide,
}

// ── Shared state ─────────────────────────────────────────────────────────────

pub struct MenuState {
    pub config: AppbarConfig,
    pub action: Option<MenuAction>,
    pub close_requested: bool,
    frames: u32,
    themed: bool,
}

impl MenuState {
    pub fn new(config: &AppbarConfig) -> Self {
        Self {
            config: config.clone(),
            action: None,
            close_requested: false,
            frames: 0,
            themed: false,
        }
    }

    pub fn reset(&mut self, config: &AppbarConfig) {
        self.config = config.clone();
        self.action = None;
        self.close_requested = false;
        self.frames = 0;
        self.themed = false;
    }
}

// ── Viewport render callback ─────────────────────────────────────────────────

pub fn render_menu_viewport(ctx: &egui::Context, state: &Arc<Mutex<MenuState>>) {
    let mut st = state.lock().unwrap();

    // Apply Fox Dark theme on first frame
    if !st.themed {
        theme::apply_theme(ctx);
        st.themed = true;
    }

    st.frames += 1;

    // Close on Escape
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.close_requested = true;
        return;
    }

    // Close on focus loss (allow a few frames for the window to settle)
    if st.frames > 3 {
        if let Some(false) = ctx.input(|i| i.viewport().focused) {
            st.close_requested = true;
            return;
        }
    }

    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::SURFACE)
                .inner_margin(8.0),
        )
        .show(ctx, |ui| {
            ui.set_min_width(MENU_WIDTH - 16.0);

            ui.label(RichText::new("Fox Appbar").color(theme::TEXT).strong());
            ui.separator();

            // Height presets
            ui.label(RichText::new("Height").color(theme::MUTED).size(14.0));
            for (label, height) in [
                ("Small (40px)", 40u32),
                ("Medium (56px)", 56),
                ("Large (72px)", 72),
            ] {
                let selected = st.config.height == height;
                if ui.selectable_label(selected, label).clicked() {
                    st.action = Some(MenuAction::SetHeight(height));
                    st.close_requested = true;
                }
            }

            ui.separator();

            // Position
            ui.label(RichText::new("Position").color(theme::MUTED).size(14.0));
            for (label, pos) in [("Top", BarPosition::Top), ("Bottom", BarPosition::Bottom)] {
                let selected = st.config.bar_position == pos;
                if ui.selectable_label(selected, label).clicked() {
                    st.action = Some(MenuAction::SetPosition(pos));
                    st.close_requested = true;
                }
            }

            ui.separator();

            // Auto-hide toggle
            let auto_label = if st.config.auto_hide.enabled {
                "Auto-hide: On"
            } else {
                "Auto-hide: Off"
            };
            if ui.button(auto_label).clicked() {
                st.action = Some(MenuAction::ToggleAutoHide);
                st.close_requested = true;
            }
        });
}
