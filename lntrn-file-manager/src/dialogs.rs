use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextInput, TextLabel};

use crate::fs::Drive;
use crate::conflict::ConflictDialog;
use crate::sudo::PendingPrivOp;
use crate::{
    ZONE_CLOUD_LOGIN_CANCEL, ZONE_CLOUD_LOGIN_EMAIL, ZONE_CLOUD_LOGIN_PASSWORD,
    ZONE_CLOUD_LOGIN_SCRIM, ZONE_CLOUD_LOGIN_SUBMIT, ZONE_CONFLICT_APPLY_TO_ALL,
    ZONE_CONFLICT_CANCEL, ZONE_CONFLICT_KEEP_BOTH, ZONE_CONFLICT_REPLACE, ZONE_CONFLICT_SCRIM,
    ZONE_CONFLICT_SKIP, ZONE_DRIVE_DIALOG_CANCEL, ZONE_DRIVE_DIALOG_CONFIRM, ZONE_DRIVE_DIALOG_OK,
    ZONE_DRIVE_DIALOG_SCRIM, ZONE_SUDO_CANCEL, ZONE_SUDO_PASSWORD, ZONE_SUDO_SCRIM,
    ZONE_SUDO_SUBMIT,
};

// ── Cloud login dialog ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginField {
    Email,
    Password,
}

#[derive(Clone, Debug)]
pub struct CloudLoginDialog {
    pub email_buf: String,
    pub email_cursor: usize,
    pub password_buf: String,
    pub password_cursor: usize,
    pub focused: LoginField,
    pub error: Option<String>,
    pub submitting: bool,
}

impl CloudLoginDialog {
    pub fn new() -> Self {
        Self {
            email_buf: String::new(),
            email_cursor: 0,
            password_buf: String::new(),
            password_cursor: 0,
            focused: LoginField::Email,
            error: None,
            submitting: false,
        }
    }

    pub fn focus_next(&mut self) {
        self.focused = match self.focused {
            LoginField::Email => LoginField::Password,
            LoginField::Password => LoginField::Email,
        };
    }

    pub fn focused_buf_mut(&mut self) -> (&mut String, &mut usize) {
        match self.focused {
            LoginField::Email => (&mut self.email_buf, &mut self.email_cursor),
            LoginField::Password => (&mut self.password_buf, &mut self.password_cursor),
        }
    }

    pub fn can_submit(&self) -> bool {
        !self.submitting && !self.email_buf.is_empty() && !self.password_buf.is_empty()
    }
}

pub fn draw_cloud_login(
    dialog: &CloudLoginDialog,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 28.0 * s;
    let body_font = 18.0 * s;
    let label_font = 16.0 * s;
    let field_h = 48.0 * s;
    let row_gap = 14.0 * s;
    let btn_h = 44.0 * s;
    let btn_w = 130.0 * s;
    let btn_gap = 12.0 * s;
    let dialog_w = 520.0 * s;

    let body_lines: f32 = 2.0; // subtitle is two lines
    let err_lines: f32 = if dialog.error.is_some() { 1.0 } else { 0.0 };

    let dialog_h = pad * 2.0
        + title_font + pad * 0.4
        + body_font * body_lines + row_gap
        + (label_font + 4.0 * s + field_h + row_gap) * 2.0
        + (err_lines * (body_font + row_gap * 0.5))
        + pad * 0.4
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay_with_scrim(
        painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s,
        ZONE_CLOUD_LOGIN_SCRIM,
    );

    let mut cy = dy + pad;
    TextLabel::new("Sign in to Cloud Sync", dx + pad, cy)
        .size(FontSize::Custom(title_font))
        .color(pal.text)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += title_font + pad * 0.4;

    TextLabel::new("Use the email + password you set in your Firebase", dx + pad, cy)
        .size(FontSize::Custom(body_font))
        .color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + 4.0 * s;
    TextLabel::new("project's Authentication console.", dx + pad, cy)
        .size(FontSize::Custom(body_font))
        .color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + row_gap;

    // ── Email field ────────────────────────────────────────────────────
    TextLabel::new("Email", dx + pad, cy)
        .size(FontSize::Custom(label_font))
        .color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += label_font + 4.0 * s;
    let email_rect = Rect::new(dx + pad, cy, dialog_w - pad * 2.0, field_h);
    input.add_zone(ZONE_CLOUD_LOGIN_EMAIL, email_rect);
    TextInput::new(email_rect)
        .text(&dialog.email_buf)
        .placeholder("you@example.com")
        .focused(dialog.focused == LoginField::Email)
        .cursor_pos(dialog.email_cursor)
        .scale(s)
        .draw(painter, text, pal, sw, sh);
    cy += field_h + row_gap;

    // ── Password field (masked) ───────────────────────────────────────
    TextLabel::new("Password", dx + pad, cy)
        .size(FontSize::Custom(label_font))
        .color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += label_font + 4.0 * s;
    let pw_rect = Rect::new(dx + pad, cy, dialog_w - pad * 2.0, field_h);
    input.add_zone(ZONE_CLOUD_LOGIN_PASSWORD, pw_rect);
    let masked: String = "•".repeat(dialog.password_buf.chars().count());
    TextInput::new(pw_rect)
        .text(&masked)
        .placeholder("••••••••")
        .focused(dialog.focused == LoginField::Password)
        .cursor_pos(dialog.password_cursor)
        .scale(s)
        .draw(painter, text, pal, sw, sh);
    cy += field_h + row_gap;

    // ── Error message (optional) ──────────────────────────────────────
    if let Some(err) = &dialog.error {
        TextLabel::new(err, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.danger)
            .max_width(dialog_w - pad * 2.0)
            .draw(text, sw, sh);
        cy += body_font + row_gap * 0.5;
    }

    cy += pad * 0.4;

    // ── Buttons ───────────────────────────────────────────────────────
    let total_btn_w = btn_w * 2.0 + btn_gap;
    let btn_x = dx + dialog_w - pad - total_btn_w;

    let cancel_rect = Rect::new(btn_x, cy, btn_w, btn_h);
    let cancel_state = input.add_zone(ZONE_CLOUD_LOGIN_CANCEL, cancel_rect);
    draw_button(
        painter, text, cancel_rect, "Cancel",
        cancel_state.is_hovered(), pal, ButtonStyle::Secondary, sw, sh, s,
    );

    let submit_label = if dialog.submitting { "Signing in..." } else { "Sign In" };
    let submit_rect = Rect::new(btn_x + btn_w + btn_gap, cy, btn_w, btn_h);
    let submit_state = input.add_zone(ZONE_CLOUD_LOGIN_SUBMIT, submit_rect);
    let submit_style = if dialog.can_submit() {
        ButtonStyle::Primary
    } else {
        ButtonStyle::Secondary
    };
    draw_button(
        painter, text, submit_rect, submit_label,
        submit_state.is_hovered() && dialog.can_submit(),
        pal, submit_style, sw, sh, s,
    );
}

fn draw_overlay_with_scrim(
    painter: &mut Painter,
    input: &mut InteractionContext,
    screen_w: f32,
    screen_h: f32,
    dx: f32, dy: f32,
    dialog_w: f32, dialog_h: f32,
    cr: f32,
    pal: &FoxPalette,
    s: f32,
    scrim_zone: u32,
) {
    let backdrop = Rect::new(0.0, 0.0, screen_w, screen_h);
    input.add_zone(scrim_zone, backdrop);
    painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.55));

    let panel = Rect::new(dx, dy, dialog_w, dialog_h);
    let shadow = Rect::new(dx - 8.0 * s, dy - 4.0 * s, dialog_w + 16.0 * s, dialog_h + 16.0 * s);
    painter.rect_filled(shadow, cr + 4.0 * s, Color::rgba(0.0, 0.0, 0.0, 0.3));
    painter.rect_filled(panel, cr, pal.surface);
    painter.rect_stroke_sdf(panel, cr, 1.0 * s, pal.muted.with_alpha(0.2));
}

/// Active drive dialog overlay state. Lives on App.
#[derive(Clone, Debug)]
pub enum DriveDialog {
    /// Confirm "Format X to ext4" — Cancel/Format buttons.
    ConfirmFormat { drive: Drive, error: Option<String> },
    /// Read-only properties view — single OK button.
    Properties { drive: Drive },
    /// Generic message/error notice — title + wrapped body, single OK button.
    Message { title: String, body: String },
}

pub fn draw(
    dialog: &DriveDialog,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    match dialog {
        DriveDialog::ConfirmFormat { drive, error } => {
            draw_confirm_format(drive, error.as_deref(), painter, text, palette, input, screen, s);
        }
        DriveDialog::Properties { drive } => {
            draw_properties(drive, painter, text, palette, input, screen, s);
        }
        DriveDialog::Message { title, body } => {
            draw_message(title, body, painter, text, palette, input, screen, s);
        }
    }
}

/// Greedy word-wrap `body` (honoring explicit `\n`) into lines no wider than
/// `max_w` at `font_size`, measured with the real text engine so proportional
/// fonts wrap accurately.
fn wrap_lines(text: &mut TextRenderer, body: &str, font_size: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in body.split('\n') {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let trial = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if cur.is_empty() || text.measure_width(&trial, font_size) <= max_w {
                cur = trial;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn draw_message(
    title: &str,
    body: &str,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 26.0 * s;
    let body_font = 18.0 * s;
    let line_gap = 6.0 * s;
    let btn_h = 40.0 * s;
    let btn_w = 120.0 * s;
    let dialog_w = 480.0 * s;

    let text_w = dialog_w - pad * 2.0;
    let lines = wrap_lines(text, body, body_font, text_w);
    let line_h = body_font + line_gap;

    let dialog_h = pad * 2.0
        + title_font + pad * 0.6
        + line_h * lines.len() as f32
        + pad
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay(painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s);

    let mut cy = dy + pad;
    TextLabel::new(title, dx + pad, cy)
        .size(FontSize::Custom(title_font))
        .color(pal.text)
        .max_width(text_w)
        .draw(text, sw, sh);
    cy += title_font + pad * 0.6;

    for line in &lines {
        TextLabel::new(line, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.text_secondary)
            // Slack so the measured last glyph isn't clipped by the wrap bound.
            .max_width(text_w + 4.0 * s)
            .draw(text, sw, sh);
        cy += line_h;
    }

    cy += pad - line_gap;

    let ok_rect = Rect::new(dx + dialog_w - pad - btn_w, cy, btn_w, btn_h);
    let ok_state = input.add_zone(ZONE_DRIVE_DIALOG_OK, ok_rect);
    draw_button(painter, text, ok_rect, "OK",
        ok_state.is_hovered(), pal, ButtonStyle::Primary, sw, sh, s);
}

fn draw_confirm_format(
    drive: &Drive,
    error: Option<&str>,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 28.0 * s;
    let body_font = 18.0 * s;
    let row_gap = 8.0 * s;
    let btn_h = 40.0 * s;
    let btn_w = 120.0 * s;
    let btn_gap = 12.0 * s;
    let dialog_w = 480.0 * s;

    let body_lines: f32 = if error.is_some() { 4.0 } else { 3.0 };
    let dialog_h = pad * 2.0
        + title_font + pad * 0.6
        + body_font * body_lines + row_gap * (body_lines - 1.0)
        + pad
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay(painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s);

    let mut cy = dy + pad;
    TextLabel::new(&format!("Format {}?", drive.name), dx + pad, cy)
        .size(FontSize::Custom(title_font))
        .color(pal.text)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += title_font + pad * 0.6;

    let body_color = pal.text_secondary;
    TextLabel::new(&format!("Device: {}", drive.device), dx + pad, cy)
        .size(FontSize::Custom(body_font)).color(body_color)
        .max_width(dialog_w - pad * 2.0).draw(text, sw, sh);
    cy += body_font + row_gap;
    TextLabel::new(&format!("Size: {}", drive.total_display()), dx + pad, cy)
        .size(FontSize::Custom(body_font)).color(body_color)
        .max_width(dialog_w - pad * 2.0).draw(text, sw, sh);
    cy += body_font + row_gap;
    TextLabel::new("All data on this drive will be erased.", dx + pad, cy)
        .size(FontSize::Custom(body_font)).color(pal.danger)
        .max_width(dialog_w - pad * 2.0).draw(text, sw, sh);
    cy += body_font + row_gap;

    if let Some(err) = error {
        TextLabel::new(&format!("Error: {err}"), dx + pad, cy)
            .size(FontSize::Custom(body_font)).color(pal.danger)
            .max_width(dialog_w - pad * 2.0).draw(text, sw, sh);
        cy += body_font + row_gap;
    }

    cy += pad - row_gap;

    let total_btn_w = btn_w * 2.0 + btn_gap;
    let btn_x = dx + dialog_w - pad - total_btn_w;

    let cancel_rect = Rect::new(btn_x, cy, btn_w, btn_h);
    let cancel_state = input.add_zone(ZONE_DRIVE_DIALOG_CANCEL, cancel_rect);
    draw_button(painter, text, cancel_rect, "Cancel",
        cancel_state.is_hovered(), pal, ButtonStyle::Secondary, sw, sh, s);

    let confirm_rect = Rect::new(btn_x + btn_w + btn_gap, cy, btn_w, btn_h);
    let confirm_state = input.add_zone(ZONE_DRIVE_DIALOG_CONFIRM, confirm_rect);
    draw_button(painter, text, confirm_rect, "Format",
        confirm_state.is_hovered(), pal, ButtonStyle::Danger, sw, sh, s);
}

fn draw_properties(
    drive: &Drive,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 28.0 * s;
    let label_font = 18.0 * s;
    let row_h = 32.0 * s;
    let btn_h = 40.0 * s;
    let btn_w = 120.0 * s;
    let dialog_w = 560.0 * s;

    let mut rows: Vec<(&str, String)> = Vec::new();
    rows.push(("Name", drive.name.clone()));
    rows.push(("Device", drive.device.clone()));
    rows.push((
        "Filesystem",
        if drive.fstype.is_empty() { "—".into() } else { drive.fstype.clone() },
    ));
    rows.push(("Size", drive.total_display()));
    if drive.mounted {
        rows.push(("Mounted at", drive.mount_point.display().to_string()));
        let used = crate::fs::format_size(drive.used_bytes);
        rows.push(("Used", format!("{} of {}", used, drive.total_display())));
        rows.push(("Free", drive.free_display()));
    } else {
        rows.push(("Mounted", "no".into()));
    }
    rows.push(("Removable", if drive.removable { "yes".into() } else { "no".into() }));

    let dialog_h = pad * 2.0
        + title_font + pad * 0.6
        + row_h * rows.len() as f32
        + pad
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay(painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s);

    let mut cy = dy + pad;
    TextLabel::new(&format!("{} — Properties", drive.name), dx + pad, cy)
        .size(FontSize::Custom(title_font))
        .color(pal.text)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += title_font + pad * 0.6;

    let label_col_w = 140.0 * s;
    for (label, value) in &rows {
        TextLabel::new(label, dx + pad, cy)
            .size(FontSize::Custom(label_font))
            .color(pal.text_secondary)
            .max_width(label_col_w).draw(text, sw, sh);
        TextLabel::new(value, dx + pad + label_col_w, cy)
            .size(FontSize::Custom(label_font))
            .color(pal.text)
            .max_width(dialog_w - pad * 2.0 - label_col_w)
            .draw(text, sw, sh);
        cy += row_h;
    }

    cy += pad - row_h;

    let ok_rect = Rect::new(dx + dialog_w - pad - btn_w, cy, btn_w, btn_h);
    let ok_state = input.add_zone(ZONE_DRIVE_DIALOG_OK, ok_rect);
    draw_button(painter, text, ok_rect, "OK",
        ok_state.is_hovered(), pal, ButtonStyle::Primary, sw, sh, s);
}

fn draw_overlay(
    painter: &mut Painter,
    input: &mut InteractionContext,
    screen_w: f32,
    screen_h: f32,
    dx: f32, dy: f32,
    dialog_w: f32, dialog_h: f32,
    cr: f32,
    pal: &FoxPalette,
    s: f32,
) {
    let backdrop = Rect::new(0.0, 0.0, screen_w, screen_h);
    input.add_zone(ZONE_DRIVE_DIALOG_SCRIM, backdrop);
    painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.55));

    let panel = Rect::new(dx, dy, dialog_w, dialog_h);
    let shadow = Rect::new(dx - 8.0 * s, dy - 4.0 * s, dialog_w + 16.0 * s, dialog_h + 16.0 * s);
    painter.rect_filled(shadow, cr + 4.0 * s, Color::rgba(0.0, 0.0, 0.0, 0.3));
    painter.rect_filled(panel, cr, pal.surface);
    painter.rect_stroke_sdf(panel, cr, 1.0 * s, pal.muted.with_alpha(0.2));
}

#[derive(Clone, Copy)]
enum ButtonStyle { Primary, Secondary, Danger }

fn draw_button(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rect: Rect,
    label: &str,
    hovered: bool,
    pal: &FoxPalette,
    style: ButtonStyle,
    sw: u32, sh: u32, s: f32,
) {
    let cr = 8.0 * s;
    let (bg, fg, stroke) = match style {
        ButtonStyle::Primary => {
            let base = pal.accent;
            let bg = if hovered { brighten(base, 0.08) } else { base };
            (bg, Color::rgba(1.0, 1.0, 1.0, 1.0), None)
        }
        ButtonStyle::Secondary => {
            let bg = if hovered {
                pal.surface_2.with_alpha(1.0)
            } else {
                pal.surface_2.with_alpha(0.6)
            };
            (bg, pal.text, Some(pal.muted.with_alpha(0.3)))
        }
        ButtonStyle::Danger => {
            let base = pal.danger;
            let bg = if hovered { brighten(base, 0.08) } else { base };
            (bg, Color::rgba(1.0, 1.0, 1.0, 1.0), None)
        }
    };

    painter.rect_filled(rect, cr, bg);
    if let Some(c) = stroke {
        painter.rect_stroke_sdf(rect, cr, 1.0 * s, c);
    }

    let font = 18.0 * s;
    // approximate text width for centering — TextLabel handles its own glyphs
    let approx_w = label.chars().count() as f32 * font * 0.55;
    let tx = rect.x + (rect.w - approx_w) * 0.5;
    let ty = rect.y + (rect.h - font) * 0.5;
    TextLabel::new(label, tx, ty)
        .size(FontSize::Custom(font))
        .color(fg)
        .draw(text, sw, sh);
}

fn brighten(c: Color, amount: f32) -> Color {
    Color::rgba(
        (c.r + amount).min(1.0),
        (c.g + amount).min(1.0),
        (c.b + amount).min(1.0),
        c.a,
    )
}

// ── Sudo password modal ────────────────────────────────────────────────────

/// Modal that captures keys/clicks until dismissed. Holds the pending op so
/// the click/key handler can retry it once the password is submitted.
#[derive(Clone, Debug)]
pub struct SudoPrompt {
    pub password: String,
    pub cursor: usize,
    pub op: PendingPrivOp,
    pub error: Option<String>,
    pub submitting: bool,
}

impl SudoPrompt {
    pub fn new(op: PendingPrivOp) -> Self {
        Self {
            password: String::new(),
            cursor: 0,
            op,
            error: None,
            submitting: false,
        }
    }

    pub fn can_submit(&self) -> bool {
        !self.submitting && !self.password.is_empty()
    }
}

pub fn draw_sudo_prompt(
    dialog: &SudoPrompt,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 26.0 * s;
    let body_font = 18.0 * s;
    let label_font = 16.0 * s;
    let field_h = 48.0 * s;
    let row_gap = 14.0 * s;
    let btn_h = 44.0 * s;
    let btn_w = 130.0 * s;
    let btn_gap = 12.0 * s;
    let dialog_w = 540.0 * s;

    let action_desc = dialog.op.description();
    let err_lines: f32 = if dialog.error.is_some() { 1.0 } else { 0.0 };
    let dialog_h = pad * 2.0
        + title_font + pad * 0.4
        + body_font + row_gap
        + label_font + 4.0 * s + field_h + row_gap
        + (err_lines * (body_font + row_gap * 0.5))
        + pad * 0.4
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay_with_scrim(
        painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s,
        ZONE_SUDO_SCRIM,
    );

    let mut cy = dy + pad;
    TextLabel::new("Administrator password required", dx + pad, cy)
        .size(FontSize::Custom(title_font))
        .color(pal.text)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += title_font + pad * 0.4;

    TextLabel::new(&action_desc, dx + pad, cy)
        .size(FontSize::Custom(body_font))
        .color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + row_gap;

    TextLabel::new("Password", dx + pad, cy)
        .size(FontSize::Custom(label_font))
        .color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += label_font + 4.0 * s;
    let pw_rect = Rect::new(dx + pad, cy, dialog_w - pad * 2.0, field_h);
    input.add_zone(ZONE_SUDO_PASSWORD, pw_rect);
    let masked: String = "\u{2022}".repeat(dialog.password.chars().count());
    TextInput::new(pw_rect)
        .text(&masked)
        .placeholder("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}")
        .focused(true)
        .cursor_pos(dialog.cursor)
        .scale(s)
        .draw(painter, text, pal, sw, sh);
    cy += field_h + row_gap;

    if let Some(err) = &dialog.error {
        TextLabel::new(err, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.danger)
            .max_width(dialog_w - pad * 2.0)
            .draw(text, sw, sh);
        cy += body_font + row_gap * 0.5;
    }

    cy += pad * 0.4;

    let total_btn_w = btn_w * 2.0 + btn_gap;
    let btn_x = dx + dialog_w - pad - total_btn_w;

    let cancel_rect = Rect::new(btn_x, cy, btn_w, btn_h);
    let cancel_state = input.add_zone(ZONE_SUDO_CANCEL, cancel_rect);
    draw_button(
        painter, text, cancel_rect, "Cancel",
        cancel_state.is_hovered(), pal, ButtonStyle::Secondary, sw, sh, s,
    );

    let submit_label = if dialog.submitting { "Authenticating..." } else { "Authenticate" };
    let submit_rect = Rect::new(btn_x + btn_w + btn_gap, cy, btn_w, btn_h);
    let submit_state = input.add_zone(ZONE_SUDO_SUBMIT, submit_rect);
    let submit_style = if dialog.can_submit() {
        ButtonStyle::Primary
    } else {
        ButtonStyle::Secondary
    };
    draw_button(
        painter, text, submit_rect, submit_label,
        submit_state.is_hovered() && dialog.can_submit(),
        pal, submit_style, sw, sh, s,
    );
}

// ── Conflict dialog (Replace / Keep Both / Skip) ──────────────────────────

pub fn draw_conflict_dialog(
    dialog: &ConflictDialog,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 24.0 * s;
    let cr = 12.0 * s;
    let title_font = 24.0 * s;
    let body_font = 18.0 * s;
    let meta_font = 15.0 * s;
    let row_gap = 8.0 * s;
    let section_gap = 16.0 * s;
    let btn_h = 44.0 * s;
    let btn_w = 130.0 * s;
    let btn_gap = 10.0 * s;
    let dialog_w = 600.0 * s;
    let checkbox_size = 20.0 * s;

    let name = dialog.target.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dest_label = dialog.target.parent()
        .map(|p| p.file_name().map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()))
        .unwrap_or_default();
    let title = format!("\u{201C}{name}\u{201D} already exists");
    let subtitle = format!("in \u{201C}{dest_label}\u{201D}");

    let remaining_line = if dialog.remaining_count > 0 { 1.0 } else { 0.0 };

    let dialog_h = pad * 2.0
        + title_font + row_gap
        + body_font + section_gap
        + body_font + row_gap  // "Existing file:" label
        + meta_font * 2.0 + row_gap // size + date lines
        + section_gap
        + body_font + row_gap  // "Source file:" label
        + meta_font * 2.0 + section_gap
        + checkbox_size + row_gap
        + remaining_line * (meta_font + row_gap)
        + pad * 0.4
        + btn_h;

    let dx = (screen_w - dialog_w) * 0.5;
    let dy = (screen_h - dialog_h) * 0.5;

    draw_overlay_with_scrim(
        painter, input, screen_w, screen_h, dx, dy, dialog_w, dialog_h, cr, pal, s,
        ZONE_CONFLICT_SCRIM,
    );

    let mut cy = dy + pad;
    TextLabel::new(&title, dx + pad, cy)
        .size(FontSize::Custom(title_font)).color(pal.text)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += title_font + row_gap;

    TextLabel::new(&subtitle, dx + pad, cy)
        .size(FontSize::Custom(body_font)).color(pal.text_secondary)
        .max_width(dialog_w - pad * 2.0)
        .draw(text, sw, sh);
    cy += body_font + section_gap;

    // Existing file metadata.
    TextLabel::new("Existing", dx + pad, cy)
        .size(FontSize::Custom(body_font)).color(pal.accent)
        .draw(text, sw, sh);
    cy += body_font + row_gap;
    TextLabel::new(&format_meta_line(&dialog.target_meta), dx + pad + 12.0 * s, cy)
        .size(FontSize::Custom(meta_font)).color(pal.text)
        .max_width(dialog_w - pad * 2.0 - 12.0 * s).draw(text, sw, sh);
    cy += meta_font + row_gap;
    TextLabel::new(&format_mtime(&dialog.target_meta), dx + pad + 12.0 * s, cy)
        .size(FontSize::Custom(meta_font)).color(pal.muted)
        .max_width(dialog_w - pad * 2.0 - 12.0 * s).draw(text, sw, sh);
    cy += meta_font + section_gap;

    // Source metadata.
    TextLabel::new("Replacement", dx + pad, cy)
        .size(FontSize::Custom(body_font)).color(pal.accent)
        .draw(text, sw, sh);
    cy += body_font + row_gap;
    TextLabel::new(&format_meta_line(&dialog.source_meta), dx + pad + 12.0 * s, cy)
        .size(FontSize::Custom(meta_font)).color(pal.text)
        .max_width(dialog_w - pad * 2.0 - 12.0 * s).draw(text, sw, sh);
    cy += meta_font + row_gap;
    TextLabel::new(&format_mtime(&dialog.source_meta), dx + pad + 12.0 * s, cy)
        .size(FontSize::Custom(meta_font)).color(pal.muted)
        .max_width(dialog_w - pad * 2.0 - 12.0 * s).draw(text, sw, sh);
    cy += meta_font + section_gap;

    // Apply-to-all checkbox.
    if dialog.remaining_count > 0 {
        let cbox = Rect::new(dx + pad, cy, checkbox_size, checkbox_size);
        let hit = Rect::new(dx + pad, cy - 4.0 * s, dialog_w - pad * 2.0, checkbox_size + 8.0 * s);
        input.add_zone(ZONE_CONFLICT_APPLY_TO_ALL, hit);
        painter.rect_stroke_sdf(cbox, 4.0 * s, 1.5 * s, pal.muted.with_alpha(0.6));
        if dialog.apply_to_all {
            let inner = Rect::new(cbox.x + 4.0 * s, cbox.y + 4.0 * s,
                cbox.w - 8.0 * s, cbox.h - 8.0 * s);
            painter.rect_filled(inner, 2.0 * s, pal.accent);
        }
        let label = format!("Apply to the remaining {} conflict{}",
            dialog.remaining_count,
            if dialog.remaining_count == 1 { "" } else { "s" });
        TextLabel::new(&label, dx + pad + checkbox_size + 10.0 * s, cy + 2.0 * s)
            .size(FontSize::Custom(meta_font)).color(pal.text)
            .draw(text, sw, sh);
        cy += checkbox_size + row_gap;
    }
    cy += pad * 0.4;

    // Buttons: Skip | Keep Both | Replace + Cancel (right side).
    let total_w = btn_w * 3.0 + btn_gap * 2.0;
    let mut bx = dx + dialog_w - pad - total_w;

    let skip_rect = Rect::new(bx, cy, btn_w, btn_h);
    let skip_hov = input.add_zone(ZONE_CONFLICT_SKIP, skip_rect).is_hovered();
    draw_button(painter, text, skip_rect, "Skip", skip_hov, pal, ButtonStyle::Secondary, sw, sh, s);
    bx += btn_w + btn_gap;

    let kb_rect = Rect::new(bx, cy, btn_w, btn_h);
    let kb_hov = input.add_zone(ZONE_CONFLICT_KEEP_BOTH, kb_rect).is_hovered();
    draw_button(painter, text, kb_rect, "Keep Both", kb_hov, pal, ButtonStyle::Secondary, sw, sh, s);
    bx += btn_w + btn_gap;

    let rep_rect = Rect::new(bx, cy, btn_w, btn_h);
    let rep_hov = input.add_zone(ZONE_CONFLICT_REPLACE, rep_rect).is_hovered();
    draw_button(painter, text, rep_rect, "Replace", rep_hov, pal, ButtonStyle::Danger, sw, sh, s);

    // Cancel sits on the left so it's not next to Replace (avoid misclick).
    let cancel_rect = Rect::new(dx + pad, cy, btn_w, btn_h);
    let cancel_hov = input.add_zone(ZONE_CONFLICT_CANCEL, cancel_rect).is_hovered();
    draw_button(painter, text, cancel_rect, "Cancel All", cancel_hov, pal, ButtonStyle::Secondary, sw, sh, s);
}

fn format_meta_line(meta: &crate::conflict::ConflictMeta) -> String {
    let kind = if meta.is_dir { "Folder" } else { "File" };
    format!("{} \u{00B7} {}", kind, crate::fs::format_size(meta.size))
}

fn format_mtime(meta: &crate::conflict::ConflictMeta) -> String {
    let Some(t) = meta.mtime else { return "Modified: unknown".into(); };
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let yd = if leap { 366 } else { 365 };
        if remaining < yd { break; }
        remaining -= yd;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 0usize;
    while mo < 12 && remaining >= months[mo] as u64 {
        remaining -= months[mo] as u64;
        mo += 1;
    }
    format!("Modified: {y:04}-{:02}-{:02} {h:02}:{m:02}", mo + 1, remaining + 1)
}
