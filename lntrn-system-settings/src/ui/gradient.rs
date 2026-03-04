use eframe::egui;
use crate::theme::{GRADIENT_PINK, GRADIENT_BLUE, GRADIENT_GREEN, GRADIENT_YELLOW, GRADIENT_RED};

pub fn render_separator(ctx: &egui::Context) {
    egui::TopBottomPanel::top("gradient_separator")
        .frame(egui::Frame::NONE)
        .exact_height(4.0)
        .show(ctx, |ui| {
            draw_gradient_bar(ui, 4.0);
        });
}

pub fn draw_gradient_bar(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    let painter = ui.painter();
    let ppp = ui.ctx().pixels_per_point();
    let step = 1.0 / ppp;
    let w = rect.width();

    let stops: &[(f32, egui::Color32)] = &[
        (0.0, GRADIENT_PINK),
        (0.25, GRADIENT_BLUE),
        (0.50, GRADIENT_GREEN),
        (0.75, GRADIENT_YELLOW),
        (1.0, GRADIENT_RED),
    ];

    let mut x = 0.0_f32;
    while x < w {
        let t = x / w;
        let color = sample_gradient(stops, t);
        let x0 = rect.left() + x;
        let x1 = (x0 + step + 0.5).min(rect.right());
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x0, rect.top()),
                egui::pos2(x1, rect.bottom()),
            ),
            0.0,
            color,
        );
        x += step;
    }
}

fn sample_gradient(stops: &[(f32, egui::Color32)], t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    for i in 0..stops.len() - 1 {
        let (t0, c0) = stops[i];
        let (t1, c1) = stops[i + 1];
        if t <= t1 {
            let local_t = (t - t0) / (t1 - t0);
            return lerp_color(c0, c1, local_t);
        }
    }
    stops.last().unwrap().1
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}
