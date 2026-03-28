use eframe::egui;

use crate::app::{LanternMusicApp, ScanState};
use crate::config::MusicConfig;
use crate::track::format_duration;

// ── Central track list panel ─────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut LanternMusicApp) {
    let panel_frame = egui::Frame::NONE
        .fill(egui::Color32::TRANSPARENT)
        .inner_margin(egui::Margin::symmetric(12, 8));

    egui::CentralPanel::default()
        .frame(panel_frame)
        .show(ctx, |ui| {
            match &app.scan_state {
                ScanState::AskUser => render_scan_prompt(ui, app),
                ScanState::Scanning(_) => render_scanning(ui, app),
                ScanState::Idle => {
                    if app.queue.tracks.is_empty() {
                        render_empty(ui, app);
                    } else {
                        render_track_list(ui, app);
                    }
                }
            }
        });
}

// ── Scan prompt ──────────────────────────────────────────────────────────────

fn render_scan_prompt(ui: &mut egui::Ui, app: &mut LanternMusicApp) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);

        ui.label(
            egui::RichText::new("Scan for music?")
                .size(20.0)
                .color(app.fox_theme.text)
                .strong(),
        );

        ui.add_space(8.0);

        ui.label(
            egui::RichText::new(&app.config.general.music_dir)
                .size(14.0)
                .color(app.fox_theme.muted)
                .monospace(),
        );

        ui.add_space(16.0);

        ui.horizontal(|ui| {
            let btn_width = 80.0;
            let total = btn_width * 2.0 + 12.0;
            ui.add_space((ui.available_width() - total) / 2.0);

            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Scan")
                            .size(15.0)
                            .color(app.fox_theme.text),
                    )
                    .min_size(egui::vec2(btn_width, 32.0)),
                )
                .clicked()
            {
                app.start_scan();
            }

            ui.add_space(12.0);

            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("Skip")
                            .size(15.0)
                            .color(app.fox_theme.muted),
                    )
                    .min_size(egui::vec2(btn_width, 32.0)),
                )
                .clicked()
            {
                app.scan_state = ScanState::Idle;
            }
        });
    });
}

// ── Scanning indicator ───────────────────────────────────────────────────────

fn render_scanning(ui: &mut egui::Ui, app: &LanternMusicApp) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);

        ui.label(
            egui::RichText::new("Scanning...")
                .size(18.0)
                .color(app.fox_theme.text),
        );

        ui.add_space(8.0);
        ui.spinner();
    });
}

// ── Empty state ──────────────────────────────────────────────────────────────

fn render_empty(ui: &mut egui::Ui, app: &LanternMusicApp) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 3.0);

        ui.label(
            egui::RichText::new("No music loaded")
                .size(18.0)
                .color(app.fox_theme.muted),
        );

        ui.add_space(8.0);

        let config_path = MusicConfig::path();
        let hint = format!(
            "Set music_dir in {}",
            config_path.display()
        );
        ui.label(
            egui::RichText::new(hint)
                .size(14.0)
                .color(app.fox_theme.muted),
        );

        ui.add_space(4.0);

        ui.label(
            egui::RichText::new("or use File > Scan Library")
                .size(14.0)
                .color(app.fox_theme.muted),
        );
    });
}

// ── Track list ───────────────────────────────────────────────────────────────

fn render_track_list(ui: &mut egui::Ui, app: &mut LanternMusicApp) {
    let current_idx = app.queue.current;
    let accent = app.fox_theme.accent;
    let text_color = app.fox_theme.text;
    let muted = app.fox_theme.muted;
    let surface_2 = app.fox_theme.surface_2;

    let mut play_index: Option<usize> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, track) in app.queue.tracks.iter().enumerate() {
                let is_current = current_idx == Some(i);
                let row_color = if is_current {
                    accent.linear_multiply(0.15)
                } else {
                    egui::Color32::TRANSPARENT
                };

                let row_frame = egui::Frame::NONE
                    .fill(row_color)
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(8, 4));

                let response = row_frame
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Track number
                            let num_color = if is_current { accent } else { muted };
                            ui.label(
                                egui::RichText::new(format!("{:>3}", i + 1))
                                    .size(14.0)
                                    .color(num_color)
                                    .monospace(),
                            );

                            ui.add_space(8.0);

                            // Title + Artist
                            let title_color = if is_current {
                                accent
                            } else {
                                text_color
                            };
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(track.display_title())
                                        .size(15.0)
                                        .color(title_color),
                                );
                                if !track.artist.is_empty() {
                                    ui.label(
                                        egui::RichText::new(track.display_artist())
                                            .size(14.0)
                                            .color(muted),
                                    );
                                }
                            });

                            // Duration (right-aligned)
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if let Some(dur) = track.duration {
                                        ui.label(
                                            egui::RichText::new(format_duration(dur))
                                                .size(14.0)
                                                .color(muted)
                                                .monospace(),
                                        );
                                    }
                                },
                            );
                        });
                    })
                    .response;

                // Hover highlight
                if response.hovered() && !is_current {
                    ui.painter().rect_filled(
                        response.rect,
                        egui::CornerRadius::same(4),
                        surface_2.linear_multiply(0.5),
                    );
                }

                if response.interact(egui::Sense::click()).clicked() {
                    play_index = Some(i);
                }
            }
        });

    // Handle track click outside the borrow
    if let Some(idx) = play_index {
        if let Some(path) = app.queue.play_index(idx) {
            if let Some(player) = app.player.as_mut() {
                player.play_file(&path).ok();
            }
        }
    }
}
