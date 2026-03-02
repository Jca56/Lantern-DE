mod config;
mod context_menu;
mod theme;
mod widgets;
mod x11;

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, Stroke};

use config::BarPosition;
use widgets::WidgetSlot;

fn main() -> eframe::Result<()> {
    // Bypass winit's DPI scaling — we work in physical pixels
    std::env::set_var("WINIT_X11_SCALE_FACTOR", "1");

    let cfg = config::load_config();
    let screen = x11::get_primary_screen();
    let bar_height = cfg.height as f32;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([screen.width, bar_height])
            .with_resizable(false)
            .with_decorations(false)
            .with_taskbar(false)
            .with_app_id("fox-appbar"),
        ..Default::default()
    };

    eframe::run_native(
        "Fox Appbar",
        options,
        Box::new(move |cc| Ok(Box::new(FoxAppbar::new(cc, cfg)))),
    )
}

// ── App state ────────────────────────────────────────────────────────────────

struct FoxAppbar {
    config: config::AppbarConfig,
    widget_slots: Vec<WidgetSlot>,
    x11_configured: bool,
    context_menu_pos: Option<egui::Pos2>,
    menu_state: Arc<Mutex<context_menu::MenuState>>,
}

impl FoxAppbar {
    fn new(cc: &eframe::CreationContext<'_>, config: config::AppbarConfig) -> Self {
        theme::apply_theme(&cc.egui_ctx);
        let widget_slots = widgets::create_widgets(&config.widgets);
        let menu_state = Arc::new(Mutex::new(context_menu::MenuState::new(&config)));
        Self {
            config,
            widget_slots,
            x11_configured: false,
            context_menu_pos: None,
            menu_state,
        }
    }

    /// Apply X11 dock properties when the window exists
    fn configure_x11_dock(&mut self, ctx: &egui::Context) {
        if self.x11_configured {
            return;
        }
        self.x11_configured = true;

        let screen = x11::get_primary_screen();
        let bar_h = self.config.height;
        let scr_h = screen.height as u32;
        let scr_w = screen.width as u32;
        let scr_x = screen.x_offset as u32;
        let position = self.config.bar_position;

        let y = match position {
            BarPosition::Top => 0.0,
            BarPosition::Bottom => screen.height - bar_h as f32,
        };

        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
            egui::pos2(screen.x_offset, y),
        ));

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Err(e) = x11::apply_x11_dock_hints(bar_h, scr_h, scr_w, scr_x, position) {
                eprintln!("Failed to set X11 dock hints: {e}");
            }
        });
    }
}

// ── eframe::App ──────────────────────────────────────────────────────────────

impl eframe::App for FoxAppbar {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // BG color (28, 28, 28) as normalized floats — solid, no transparency
        [28.0 / 255.0, 28.0 / 255.0, 28.0 / 255.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.configure_x11_dock(ctx);

        let panel_response = egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(12, 0))
                    .stroke(Stroke::new(1.0, Color32::from_white_alpha(25))),
            )
            .show(ctx, |ui| {
                let bar_rect = ui.available_rect_before_wrap();

                for slot in &mut self.widget_slots {
                    let rect = widgets::widget_rect(slot, bar_rect);
                    slot.widget.render(ui, rect);
                }

                // Full bar area responds to right-click for context menu
                ui.interact(bar_rect, ui.id().with("bar_bg"), egui::Sense::click())
            });

        // Right-click toggles context menu
        if panel_response.inner.secondary_clicked() {
            if self.context_menu_pos.is_some() {
                self.context_menu_pos = None;
            } else {
                let pos = ctx.input(|i| i.pointer.latest_pos()).unwrap_or_default();
                self.context_menu_pos = Some(pos);
                let mut st = self.menu_state.lock().unwrap();
                st.reset(&self.config);
            }
        }

        // Show context menu as a separate viewport window
        if let Some(pos) = self.context_menu_pos {
            let screen = x11::get_primary_screen();
            let bar_h = self.config.height as f32;

            let menu_x = (screen.x_offset + pos.x)
                .min(screen.x_offset + screen.width - context_menu::MENU_WIDTH);
            let menu_y = match self.config.bar_position {
                BarPosition::Bottom => screen.height - bar_h - context_menu::MENU_HEIGHT,
                BarPosition::Top => bar_h,
            };

            let state = Arc::clone(&self.menu_state);
            ctx.show_viewport_deferred(
                egui::ViewportId::from_hash_of("fox_appbar_ctx_menu"),
                egui::ViewportBuilder::default()
                    .with_inner_size([context_menu::MENU_WIDTH, context_menu::MENU_HEIGHT])
                    .with_position(egui::pos2(menu_x, menu_y))
                    .with_decorations(false)
                    .with_taskbar(false),
                move |vp_ctx, _class| {
                    context_menu::render_menu_viewport(vp_ctx, &state);
                },
            );
        }

        // Process actions from the menu viewport
        {
            let mut st = self.menu_state.lock().unwrap();
            let prev_position = self.config.bar_position;
            let prev_height = self.config.height;
            let mut config_changed = false;

            if let Some(action) = st.action.take() {
                match action {
                    context_menu::MenuAction::SetHeight(h) => {
                        self.config.height = h;
                        config_changed = true;
                    }
                    context_menu::MenuAction::SetPosition(p) => {
                        self.config.bar_position = p;
                        config_changed = true;
                    }
                    context_menu::MenuAction::ToggleAutoHide => {
                        self.config.auto_hide.enabled = !self.config.auto_hide.enabled;
                        config_changed = true;
                    }
                }
            }

            if st.close_requested {
                self.context_menu_pos = None;
            }

            if config_changed {
                config::save_config(&self.config);

                if self.config.bar_position != prev_position || self.config.height != prev_height {
                    let screen = x11::get_primary_screen();
                    let bar_h = self.config.height as f32;

                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(
                        egui::vec2(screen.width, bar_h),
                    ));

                    self.x11_configured = false;
                }
            }
        }
    }
}
