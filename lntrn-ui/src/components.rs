use eframe::egui;

use crate::theme::LanternTheme;
use crate::typography::ts;

// ── Styled button ────────────────────────────────────────────────────────────

pub enum ButtonKind {
    Default,
    Primary,
    Danger,
}

/// Render a themed button. Returns `true` if clicked.
pub fn button(ui: &mut egui::Ui, theme: &LanternTheme, kind: ButtonKind, label: &str) -> bool {
    let bt = match kind {
        ButtonKind::Default => &theme.button,
        ButtonKind::Primary => &theme.button_primary,
        ButtonKind::Danger => &theme.button_danger,
    };

    let text = egui::RichText::new(label)
        .size(ts(crate::typography::FONT_BODY))
        .color(bt.text);

    let btn = egui::Button::new(text)
        .fill(bt.bg)
        .stroke(bt.border)
        .corner_radius(bt.radius);

    let response = ui.add(btn);

    if response.hovered() {
        ui.painter().rect_filled(response.rect, bt.radius, bt.bg_hover);
    }

    response.clicked()
}

// ── Styled text input ────────────────────────────────────────────────────────

/// Render a themed single-line text input. Returns the `Response`.
pub fn text_input(
    ui: &mut egui::Ui,
    theme: &LanternTheme,
    value: &mut String,
    hint: &str,
) -> egui::Response {
    let input = &theme.input;

    let desired_size = egui::vec2(ui.available_width(), input.height);

    let frame = egui::Frame::NONE
        .fill(input.bg)
        .stroke(input.border)
        .corner_radius(input.radius)
        .inner_margin(egui::Margin::symmetric(10, 6));

    let mut response = None;

    frame.show(ui, |ui| {
        ui.set_min_size(desired_size - egui::vec2(20.0, 12.0));
        let r = ui.add(
            egui::TextEdit::singleline(value)
                .font(egui::FontId::proportional(ts(crate::typography::FONT_BODY)))
                .text_color(input.text)
                .hint_text(egui::RichText::new(hint).color(input.placeholder))
                .frame(false)
                .desired_width(ui.available_width()),
        );
        response = Some(r);
    });

    response.unwrap()
}

// ── Sidebar panel helper ─────────────────────────────────────────────────────

/// Render a sidebar panel with proper Lantern theming. The closure receives the
/// inner `Ui` to add sidebar content.
pub fn sidebar(
    ctx: &egui::Context,
    theme: &LanternTheme,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::SidePanel::left("lantern_sidebar")
        .exact_width(theme.sidebar_width)
        .frame(
            egui::Frame::NONE
                .fill(theme.sidebar_bg)
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = theme.item_spacing;
            add_contents(ui);
        });
}

/// Render a sidebar item (selectable row). Returns `true` if clicked.
pub fn sidebar_item(
    ui: &mut egui::Ui,
    theme: &LanternTheme,
    label: &str,
    selected: bool,
) -> bool {
    let text_color = if selected { theme.accent } else { theme.sidebar_text };
    let bg = if selected {
        theme.accent.linear_multiply(0.15)
    } else {
        egui::Color32::TRANSPARENT
    };

    let text = egui::RichText::new(label)
        .size(ts(crate::typography::FONT_BODY))
        .color(text_color);

    let response = ui.add(
        egui::Button::new(text)
            .fill(bg)
            .stroke(egui::Stroke::NONE)
            .corner_radius(theme.widget_radius)
            .min_size(egui::vec2(ui.available_width(), 32.0)),
    );

    response.clicked()
}

// ── Title bar helper ─────────────────────────────────────────────────────────

pub struct TitleBarResponse {
    pub minimize_clicked: bool,
    pub maximize_clicked: bool,
    pub close_clicked: bool,
}

/// Render a title bar with window controls. Returns which buttons were clicked.
pub fn title_bar(
    ui: &mut egui::Ui,
    theme: &LanternTheme,
    title: &str,
) -> TitleBarResponse {
    let height = theme.title_bar_height;
    let ctrl = theme.control_size;

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, theme.title_bar_bg);

    // Title text
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(ts(crate::typography::FONT_BODY)),
        theme.text,
    );

    let mut result = TitleBarResponse {
        minimize_clicked: false,
        maximize_clicked: false,
        close_clicked: false,
    };

    // Window controls (right-aligned): minimize, maximize, close
    let button_width = ctrl;
    let right_edge = rect.right() - 4.0;

    // Close
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(right_edge - button_width, rect.top()),
        egui::vec2(button_width, height),
    );
    let close_resp = ui.interact(close_rect, ui.id().with("close"), egui::Sense::click());
    let close_color = if close_resp.hovered() { theme.close_hover } else { theme.muted };
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{2715}",
        egui::FontId::proportional(14.0),
        close_color,
    );
    result.close_clicked = close_resp.clicked();

    // Maximize
    let max_rect = egui::Rect::from_min_size(
        egui::pos2(right_edge - button_width * 2.0, rect.top()),
        egui::vec2(button_width, height),
    );
    let max_resp = ui.interact(max_rect, ui.id().with("maximize"), egui::Sense::click());
    let max_color = if max_resp.hovered() { theme.control_hover } else { theme.muted };
    ui.painter().text(
        max_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{25A1}",
        egui::FontId::proportional(14.0),
        max_color,
    );
    result.maximize_clicked = max_resp.clicked();

    // Minimize
    let min_rect = egui::Rect::from_min_size(
        egui::pos2(right_edge - button_width * 3.0, rect.top()),
        egui::vec2(button_width, height),
    );
    let min_resp = ui.interact(min_rect, ui.id().with("minimize"), egui::Sense::click());
    let min_color = if min_resp.hovered() { theme.control_hover } else { theme.muted };
    ui.painter().text(
        min_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{2013}",
        egui::FontId::proportional(14.0),
        min_color,
    );
    result.minimize_clicked = min_resp.clicked();

    result
}

// ── Separator helper ─────────────────────────────────────────────────────────

/// Draw a themed separator (thin line with proper spacing).
pub fn separator(ui: &mut egui::Ui, theme: &LanternTheme) {
    let rect = ui.available_rect_before_wrap();
    let y = ui.cursor().top() + 4.0;
    ui.painter().line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.0, theme.separator),
    );
    ui.add_space(9.0);
}
