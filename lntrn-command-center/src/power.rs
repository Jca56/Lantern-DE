//! Power action buttons that float to the right of the main panel.
//!
//! Four stacked buttons (Lock / Sleep / Restart / Shutdown), each
//! rendered as a soft rounded plate with the matching `spark-menu-*`
//! icon from `~/.lantern/icons/`. A click pops a confirm modal; the
//! action only runs once Confirm is hit.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::render::IconRequest;

/// Button face size (logical px). Square.
pub const BUTTON_SIZE: f32 = 72.0;
/// Vertical gap between buttons (logical px).
pub const BUTTON_GAP: f32 = 14.0;
/// Horizontal gap between the panel's right edge and the column.
pub const COLUMN_LEFT_GAP: f32 = 20.0;
/// Corner radius for each button face.
pub const BUTTON_RADIUS: f32 = 18.0;
/// Icon size as a fraction of the button face size.
pub const ICON_SIZE_FRAC: f32 = 0.55;

/// Background dim color (matches the panel's surface color), and the
/// alpha values used in idle / hover states.
const BG_RGB: (u8, u8, u8) = (24, 24, 24);
const BG_ALPHA_IDLE: f32 = 0.55;
const BG_ALPHA_HOVER: f32 = 0.85;
const ACCENT_RGB: (u8, u8, u8) = (0xc8, 0x86, 0x0a);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Lock,
    Sleep,
    Restart,
    Shutdown,
}

impl PowerAction {
    pub const ALL: [PowerAction; 4] = [
        PowerAction::Lock,
        PowerAction::Sleep,
        PowerAction::Restart,
        PowerAction::Shutdown,
    ];

    pub fn icon_name(self) -> &'static str {
        match self {
            PowerAction::Lock => "spark-menu-lockscreen",
            PowerAction::Sleep => "spark-menu-sleep",
            PowerAction::Restart => "spark-menu-restart",
            PowerAction::Shutdown => "spark-menu-shutdown",
        }
    }

    pub fn command(self) -> (&'static str, &'static [&'static str]) {
        match self {
            PowerAction::Lock => ("loginctl", &["lock-session"]),
            PowerAction::Sleep => ("systemctl", &["suspend"]),
            PowerAction::Restart => ("systemctl", &["reboot"]),
            PowerAction::Shutdown => ("systemctl", &["poweroff"]),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            PowerAction::Lock => "Lock screen?",
            PowerAction::Sleep => "Suspend?",
            PowerAction::Restart => "Restart?",
            PowerAction::Shutdown => "Shut down?",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            PowerAction::Lock => "Your session will be locked.",
            PowerAction::Sleep => "The system will suspend to RAM.",
            PowerAction::Restart => "The system will reboot now.",
            PowerAction::Shutdown => "The system will power off now.",
        }
    }

    pub fn confirm_label(self) -> &'static str {
        match self {
            PowerAction::Lock => "Lock",
            PowerAction::Sleep => "Suspend",
            PowerAction::Restart => "Restart",
            PowerAction::Shutdown => "Shut Down",
        }
    }
}

/// Rect of the i-th button in physical pixels. Stack grows upward from
/// the panel's bottom-right corner so the bottom button (Shutdown) sits
/// at panel-bottom and Lock floats highest.
pub fn button_rect(panel: Rect, scale: f32, idx: usize) -> Rect {
    let size = BUTTON_SIZE * scale;
    let gap = BUTTON_GAP * scale;
    let left_gap = COLUMN_LEFT_GAP * scale;
    let x = panel.x + panel.w + left_gap;
    let total = PowerAction::ALL.len() as f32 * size
        + (PowerAction::ALL.len() as f32 - 1.0) * gap;
    let column_top = panel.y + panel.h - total;
    let y = column_top + idx as f32 * (size + gap);
    Rect::new(x, y, size, size)
}

/// Total logical width the column occupies including the gap to the
/// panel. Used to sanity-check that the surface is wide enough.
#[allow(dead_code)]
pub fn column_width_logical() -> f32 {
    COLUMN_LEFT_GAP + BUTTON_SIZE
}

/// Hit-test a physical-pixel cursor against the power column.
pub fn hit_test(panel: Rect, scale: f32, px: f32, py: f32) -> Option<PowerAction> {
    for (i, action) in PowerAction::ALL.iter().enumerate() {
        let r = button_rect(panel, scale, i);
        if px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h {
            return Some(*action);
        }
    }
    None
}

pub fn draw(
    painter: &mut Painter,
    icons: &mut Vec<IconRequest>,
    panel: Rect,
    scale: f32,
    alpha: f32,
    hovered: Option<PowerAction>,
) {
    let radius = BUTTON_RADIUS * scale;
    for (i, action) in PowerAction::ALL.iter().enumerate() {
        let r = button_rect(panel, scale, i);
        let is_hovered = hovered == Some(*action);
        let bg_a = if is_hovered { BG_ALPHA_HOVER } else { BG_ALPHA_IDLE };
        let bg = Color::from_rgb8(BG_RGB.0, BG_RGB.1, BG_RGB.2).with_alpha(bg_a * alpha);
        painter.rect_filled(r, radius, bg);
        if is_hovered {
            let accent = Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2)
                .with_alpha(0.65 * alpha);
            painter.rect_stroke_sdf(r, radius, 2.0 * scale, accent);
        }

        let icon_size = r.h * ICON_SIZE_FRAC;
        let icon_x = r.x + (r.w - icon_size) / 2.0;
        let icon_y = r.y + (r.h - icon_size) / 2.0;
        let icon_name = action.icon_name();
        icons.push(IconRequest {
            app_id: icon_name.to_string(),
            icon_name: Some(icon_name.to_string()),
            x: icon_x,
            y: icon_y,
            size: icon_size,
            opacity: alpha,
            clip: None,
        });
    }
}

/// Spawn the system command for `action`. Logs and swallows errors —
/// we don't want a failed `systemctl` call to crash the panel.
pub fn run(action: PowerAction) {
    let (cmd, args) = action.command();
    match std::process::Command::new(cmd).args(args).spawn() {
        Ok(_) => tracing::info!(?action, "spawned power action"),
        Err(e) => tracing::error!(?action, error = ?e, "failed to spawn power action"),
    }
}

// ── Confirm modal ───────────────────────────────────────────────────────────

/// Modal card geometry (logical px).
pub const MODAL_W: f32 = 420.0;
pub const MODAL_H: f32 = 220.0;
pub const MODAL_RADIUS: f32 = 18.0;
pub const MODAL_PAD: f32 = 28.0;
pub const TITLE_FONT: f32 = 24.0;
pub const SUBTITLE_FONT: f32 = 16.0;
pub const BUTTON_W: f32 = 140.0;
pub const BUTTON_H: f32 = 48.0;
pub const BUTTON_GAP_H: f32 = 16.0;
pub const BUTTON_FONT: f32 = 17.0;
pub const BUTTON_RADIUS_MODAL: f32 = 12.0;

const BACKDROP_ALPHA: f32 = 0.55;
const CARD_RGB: (u8, u8, u8) = (28, 28, 28);
const CARD_ALPHA: f32 = 0.98;
const CARD_BORDER_ALPHA: f32 = 0.12;
const CANCEL_BG_RGB: (u8, u8, u8) = (60, 60, 60);
const CANCEL_BG_ALPHA: f32 = 0.90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmHit {
    Confirm,
    Cancel,
    /// Hit landed on the card but not on a button — eats the click so
    /// the user doesn't accidentally dismiss by clicking the card body.
    CardBody,
}

/// Modal card rect centered over the surface.
pub fn modal_card_rect(surface_w: u32, surface_h: u32, scale: f32) -> Rect {
    let w = MODAL_W * scale;
    let h = MODAL_H * scale;
    let x = (surface_w as f32 - w) / 2.0;
    let y = (surface_h as f32 - h) / 2.0;
    Rect::new(x, y, w, h)
}

fn cancel_button_rect(card: Rect, scale: f32) -> Rect {
    let bw = BUTTON_W * scale;
    let bh = BUTTON_H * scale;
    let pad = MODAL_PAD * scale;
    let gap = BUTTON_GAP_H * scale;
    let total = bw * 2.0 + gap;
    let row_x = card.x + (card.w - total) / 2.0;
    let row_y = card.y + card.h - pad - bh;
    Rect::new(row_x, row_y, bw, bh)
}

fn confirm_button_rect(card: Rect, scale: f32) -> Rect {
    let bw = BUTTON_W * scale;
    let bh = BUTTON_H * scale;
    let pad = MODAL_PAD * scale;
    let gap = BUTTON_GAP_H * scale;
    let total = bw * 2.0 + gap;
    let row_x = card.x + (card.w - total) / 2.0;
    let row_y = card.y + card.h - pad - bh;
    Rect::new(row_x + bw + gap, row_y, bw, bh)
}

/// Hit-test the confirm modal. Returns `None` when the click landed
/// fully outside the card (caller treats that as Cancel).
pub fn hit_test_confirm(
    surface_w: u32,
    surface_h: u32,
    scale: f32,
    px: f32,
    py: f32,
) -> Option<ConfirmHit> {
    let card = modal_card_rect(surface_w, surface_h, scale);
    let cancel = cancel_button_rect(card, scale);
    let confirm = confirm_button_rect(card, scale);
    if px >= confirm.x && px <= confirm.x + confirm.w
        && py >= confirm.y && py <= confirm.y + confirm.h
    {
        return Some(ConfirmHit::Confirm);
    }
    if px >= cancel.x && px <= cancel.x + cancel.w
        && py >= cancel.y && py <= cancel.y + cancel.h
    {
        return Some(ConfirmHit::Cancel);
    }
    if px >= card.x && px <= card.x + card.w
        && py >= card.y && py <= card.y + card.h
    {
        return Some(ConfirmHit::CardBody);
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn draw_confirm(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &mut Vec<IconRequest>,
    action: PowerAction,
    surface_w: u32,
    surface_h: u32,
    scale: f32,
    alpha: f32,
) {
    // Dim backdrop covering the whole surface.
    let backdrop = Rect::new(0.0, 0.0, surface_w as f32, surface_h as f32);
    painter.rect_filled(
        backdrop,
        0.0,
        Color::BLACK.with_alpha(BACKDROP_ALPHA * alpha),
    );

    let card = modal_card_rect(surface_w, surface_h, scale);
    let radius = MODAL_RADIUS * scale;
    let pad = MODAL_PAD * scale;
    let title_font = TITLE_FONT * scale;
    let subtitle_font = SUBTITLE_FONT * scale;
    let button_font = BUTTON_FONT * scale;
    let icon_size = 56.0 * scale;

    // Card body.
    painter.rect_filled(
        card,
        radius,
        Color::from_rgb8(CARD_RGB.0, CARD_RGB.1, CARD_RGB.2).with_alpha(CARD_ALPHA * alpha),
    );
    painter.rect_stroke_sdf(
        card,
        radius,
        1.0 * scale,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(CARD_BORDER_ALPHA * alpha),
    );

    // Action icon, centered horizontally near the top.
    let icon_x = card.x + (card.w - icon_size) / 2.0;
    let icon_y = card.y + pad;
    icons.push(IconRequest {
        app_id: action.icon_name().to_string(),
        icon_name: Some(action.icon_name().to_string()),
        x: icon_x,
        y: icon_y,
        size: icon_size,
        opacity: alpha,
        clip: None,
    });

    // Title under the icon.
    let title = action.title();
    let title_w = text.measure_width(title, title_font);
    let title_x = card.x + (card.w - title_w) / 2.0;
    let title_y = icon_y + icon_size + 10.0 * scale;
    text.queue(
        title,
        title_font,
        title_x,
        title_y,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
        title_w,
        surface_w,
        surface_h,
    );

    // Subtitle.
    let subtitle = action.subtitle();
    let sub_w = text.measure_width(subtitle, subtitle_font);
    let sub_x = card.x + (card.w - sub_w) / 2.0;
    let sub_y = title_y + title_font + 4.0 * scale;
    text.queue(
        subtitle,
        subtitle_font,
        sub_x,
        sub_y,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(0.65 * alpha),
        sub_w,
        surface_w,
        surface_h,
    );

    // Buttons.
    let cancel = cancel_button_rect(card, scale);
    let confirm = confirm_button_rect(card, scale);
    let btn_radius = BUTTON_RADIUS_MODAL * scale;

    // Cancel: neutral grey.
    painter.rect_filled(
        cancel,
        btn_radius,
        Color::from_rgb8(CANCEL_BG_RGB.0, CANCEL_BG_RGB.1, CANCEL_BG_RGB.2)
            .with_alpha(CANCEL_BG_ALPHA * alpha),
    );
    let cancel_text = "Cancel";
    let cw = text.measure_width(cancel_text, button_font);
    text.queue(
        cancel_text,
        button_font,
        cancel.x + (cancel.w - cw) / 2.0,
        cancel.y + (cancel.h - button_font) / 2.0,
        Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha),
        cw,
        surface_w,
        surface_h,
    );

    // Confirm: accent gold.
    painter.rect_filled(
        confirm,
        btn_radius,
        Color::from_rgb8(ACCENT_RGB.0, ACCENT_RGB.1, ACCENT_RGB.2).with_alpha(alpha),
    );
    let confirm_text = action.confirm_label();
    let xw = text.measure_width(confirm_text, button_font);
    text.queue(
        confirm_text,
        button_font,
        confirm.x + (confirm.w - xw) / 2.0,
        confirm.y + (confirm.h - button_font) / 2.0,
        Color::BLACK.with_alpha(alpha),
        xw,
        surface_w,
        surface_h,
    );
}
