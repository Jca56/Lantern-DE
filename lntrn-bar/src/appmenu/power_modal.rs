//! Power confirmation modal — semi-transparent popup that gates destructive
//! actions (poweroff/reboot/logout) behind an explicit confirm click.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::input::InteractionState;
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::svg_icon::IconCache;

use super::{AppMenu, ZONE_PWR_MODAL};

/// Destructive power actions that need a confirmation tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerAction {
    Power,
    Reboot,
    Logout,
}

impl PowerAction {
    pub fn title(&self) -> &'static str {
        match self {
            PowerAction::Power => "Power Off?",
            PowerAction::Reboot => "Reboot?",
            PowerAction::Logout => "Log Out?",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            PowerAction::Power => "This will shut down your computer.",
            PowerAction::Reboot => "This will restart your computer.",
            PowerAction::Logout => "This will end your current session.",
        }
    }
    pub fn icon_key(&self) -> &'static str {
        match self {
            PowerAction::Power => "power_modal_power",
            PowerAction::Reboot => "power_modal_reboot",
            PowerAction::Logout => "power_modal_logout",
        }
    }
    pub fn confirm_label(&self) -> &'static str {
        match self {
            PowerAction::Power => "Power Off",
            PowerAction::Reboot => "Reboot",
            PowerAction::Logout => "Log Out",
        }
    }
    pub fn execute(&self) {
        match self {
            PowerAction::Power => {
                let _ = std::process::Command::new("systemctl").arg("poweroff").spawn();
            }
            PowerAction::Reboot => {
                let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            }
            PowerAction::Logout => {
                let _ = std::process::Command::new("loginctl")
                    .arg("terminate-session").arg("self").spawn();
            }
        }
    }
}

pub(crate) const ZONE_CONFIRM: u32 = ZONE_PWR_MODAL;
pub(crate) const ZONE_CANCEL: u32 = ZONE_PWR_MODAL + 1;
pub(crate) const ZONE_BACKDROP: u32 = ZONE_PWR_MODAL + 2;

impl AppMenu {
    /// Draw the power confirmation modal on top of the menu. Caller is
    /// responsible for setting a high render layer before calling.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_power_modal(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        icon_cache: &IconCache,
        palette: &FoxPalette,
        scale: f32, screen_w: u32, screen_h: u32,
        icon_draws: &mut Vec<(String, f32, f32, f32, f32, Option<[f32; 4]>)>,
    ) {
        let Some(action) = self.pending_power else { return; };

        // ── Backdrop: dim everything inside the menu surface ──────────────
        // We cover both the panel and the floating row above it so the user
        // sees a unified "blocked" state.
        let pad = super::PADDING * scale;
        let float_h = (super::TAB_SIZE + 6.0) * scale + super::SEARCH_FLOAT_GAP * scale;
        let backdrop = Rect::new(
            self.bounds.x - pad,
            self.bounds.y - float_h - pad,
            self.bounds.w + pad * 2.0,
            self.bounds.h + float_h + pad * 2.0,
        );
        painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.55));
        let backdrop_state = ix.add_zone(ZONE_BACKDROP, backdrop);

        // ── Modal card ────────────────────────────────────────────────────
        let card_w = 480.0 * scale;
        let card_h = 280.0 * scale;
        let cx = self.bounds.x + (self.bounds.w - card_w) * 0.5;
        let cy = self.bounds.y + (self.bounds.h - card_h) * 0.5;
        let card = Rect::new(cx, cy, card_w, card_h);
        let cr = 16.0 * scale;

        // Drop shadow
        let shadow_expand = 10.0 * scale;
        painter.rect_filled(
            Rect::new(
                cx - shadow_expand, cy + shadow_expand,
                card_w + shadow_expand * 2.0, card_h + shadow_expand,
            ),
            cr, Color::BLACK.with_alpha(0.55),
        );

        // Card body — semi-transparent so a hint of the menu shows through
        painter.rect_filled(card, cr, palette.surface_2.with_alpha(0.92));
        // Black rounded border, matching the menu chrome
        painter.rect_stroke_sdf(card, cr, 3.0 * scale, Color::BLACK);

        // ── Action icon (centered, near top) ──────────────────────────────
        let icon_sz = 80.0 * scale;
        let icon_x = cx + (card_w - icon_sz) * 0.5;
        let icon_y = cy + 24.0 * scale;
        if icon_cache.get(action.icon_key()).is_some() {
            icon_draws.push((action.icon_key().to_string(), icon_x, icon_y, icon_sz, icon_sz, None));
        }

        // ── Title ─────────────────────────────────────────────────────────
        let title = action.title();
        let title_font = 30.0 * scale;
        let title_w = text.measure_width(title, title_font);
        let title_x = cx + (card_w - title_w) * 0.5;
        let title_y = icon_y + icon_sz + 14.0 * scale;
        text.queue(title, title_font, title_x, title_y, palette.text, card_w, screen_w, screen_h);

        // ── Message ───────────────────────────────────────────────────────
        let msg = action.message();
        let msg_font = 18.0 * scale;
        let msg_w = text.measure_width(msg, msg_font);
        let msg_x = cx + (card_w - msg_w) * 0.5;
        let msg_y = title_y + title_font + 6.0 * scale;
        text.queue(msg, msg_font, msg_x, msg_y, palette.text_secondary, card_w, screen_w, screen_h);

        // ── Buttons ───────────────────────────────────────────────────────
        let btn_w = 168.0 * scale;
        let btn_h = 54.0 * scale;
        let btn_gap = 16.0 * scale;
        let btn_y = cy + card_h - btn_h - 22.0 * scale;
        let cancel_x = cx + (card_w * 0.5) - btn_w - btn_gap * 0.5;
        let confirm_x = cx + (card_w * 0.5) + btn_gap * 0.5;
        let btn_cr = 12.0 * scale;
        let btn_font = 22.0 * scale;
        let danger = Color::from_rgb8(239, 68, 68);

        // Cancel (neutral, default focus)
        let cancel_rect = Rect::new(cancel_x, btn_y, btn_w, btn_h);
        let cancel_state = ix.add_zone(ZONE_CANCEL, cancel_rect);
        let cancel_bg = if cancel_state.is_hovered() { palette.surface } else { palette.bg };
        painter.rect_filled(cancel_rect, btn_cr, cancel_bg);
        painter.rect_stroke_sdf(cancel_rect, btn_cr, 3.0 * scale, Color::BLACK);
        let cancel_label = "Cancel";
        let cancel_w = text.measure_width(cancel_label, btn_font);
        let cancel_lx = cancel_x + (btn_w - cancel_w) * 0.5;
        let cancel_ly = btn_y + (btn_h - btn_font) * 0.5;
        text.queue(cancel_label, btn_font, cancel_lx, cancel_ly, palette.text, btn_w, screen_w, screen_h);

        // Confirm (danger red)
        let confirm_rect = Rect::new(confirm_x, btn_y, btn_w, btn_h);
        let confirm_state = ix.add_zone(ZONE_CONFIRM, confirm_rect);
        let confirm_bg = if confirm_state.is_hovered() {
            Color::from_rgb8(220, 50, 50)
        } else {
            danger
        };
        painter.rect_filled(confirm_rect, btn_cr, confirm_bg);
        painter.rect_stroke_sdf(confirm_rect, btn_cr, 3.0 * scale, Color::BLACK);
        let confirm_label = action.confirm_label();
        let confirm_w = text.measure_width(confirm_label, btn_font);
        let confirm_lx = confirm_x + (btn_w - confirm_w) * 0.5;
        let confirm_ly = btn_y + (btn_h - btn_font) * 0.5;
        text.queue(confirm_label, btn_font, confirm_lx, confirm_ly, Color::WHITE, btn_w, screen_w, screen_h);

        // Press dispatch is handled in `on_left_click` via `zone_at`, but we
        // still query the states here so the visuals update on hover.
        let _ = (backdrop_state, cancel_state, confirm_state, InteractionState::Idle);
    }
}
