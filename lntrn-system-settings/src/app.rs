use eframe::egui;
use crate::config::LanternConfig;
use crate::theme::FoxTheme;
use crate::ui::{self, Panel};

// ── Main application ─────────────────────────────────────────────────────────

pub struct SettingsApp {
    pub config: LanternConfig,
    pub fox_theme: FoxTheme,
    pub active_panel: Panel,
    pub dirty: bool,
    pub config_snapshot: String,
}

impl SettingsApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: LanternConfig) -> Self {
        let fox_theme = match config.appearance.theme.as_str() {
            "lantern" => FoxTheme::lantern(),
            _ => FoxTheme::dark(),
        };
        fox_theme.apply(&cc.egui_ctx);
        let config_snapshot = format!("{:?}", config);

        Self {
            config,
            fox_theme,
            active_panel: Panel::Appearance,
            dirty: false,
            config_snapshot,
        }
    }
}

impl eframe::App for SettingsApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Paint background with rounded corners
        let full = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::background())
            .rect_filled(full, egui::CornerRadius::same(10), self.fox_theme.bg);

        // Title bar (TopBottomPanel)
        ui::title_bar::render(ctx, self);

        // Gradient separator (TopBottomPanel)
        ui::gradient::render_separator(ctx);

        // Sidebar (SidePanel)
        ui::sidebar::render(ctx, self);

        // Content panel (CentralPanel)
        ui::content::render(ctx, self);

        // Detect dirty state
        let current = format!("{:?}", self.config);
        if current != self.config_snapshot {
            self.dirty = true;
        }
    }
}
