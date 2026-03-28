use eframe::egui;

use crate::app::LanternMusicApp;
use crate::track::format_duration;

// ── Bottom control panel ─────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut LanternMusicApp) {
    let panel_frame = egui::Frame::NONE
        .fill(app.fox_theme.surface)
        .inner_margin(egui::Margin::symmetric(16, 12));

    egui::TopBottomPanel::bottom("controls")
        .frame(panel_frame)
        .show(ctx, |ui| {
            now_playing_info(ui, app);
            ui.add_space(6.0);
            progress_bar(ui, app);
            ui.add_space(8.0);
            transport_row(ui, app);
        });
}

// ── Now playing info ─────────────────────────────────────────────────────────

fn now_playing_info(ui: &mut egui::Ui, app: &LanternMusicApp) {
    let (title, artist) = match app.queue.current_track() {
        Some(track) => (track.display_title(), track.display_artist()),
        None => ("No track playing", ""),
    };

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("\u{266B}")
                .size(18.0)
                .color(app.fox_theme.accent),
        );
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(15.0)
                    .color(app.fox_theme.text)
                    .strong(),
            );
            if !artist.is_empty() {
                ui.label(
                    egui::RichText::new(artist)
                        .size(14.0)
                        .color(app.fox_theme.text_secondary),
                );
            }
        });
    });
}

// ── Progress bar ─────────────────────────────────────────────────────────────

fn progress_bar(ui: &mut egui::Ui, app: &LanternMusicApp) {
    let elapsed = app
        .player
        .as_ref()
        .map(|p| p.elapsed())
        .unwrap_or_default();

    let total = app
        .queue
        .current_track()
        .and_then(|t| t.duration)
        .unwrap_or_default();

    let elapsed_secs = elapsed.as_secs_f32();
    let total_secs = total.as_secs_f32();

    ui.horizontal(|ui| {
        // Elapsed time
        ui.label(
            egui::RichText::new(format_duration(elapsed))
                .size(14.0)
                .color(app.fox_theme.muted)
                .monospace(),
        );

        // Progress bar
        let available = ui.available_width() - 60.0;
        let bar_height = 6.0;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(available.max(40.0), bar_height),
            egui::Sense::hover(),
        );

        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(3),
            app.fox_theme.surface_2,
        );

        if total_secs > 0.0 {
            let progress = (elapsed_secs / total_secs).clamp(0.0, 1.0);
            let filled = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(
                    rect.left() + rect.width() * progress,
                    rect.bottom(),
                ),
            );
            ui.painter().rect_filled(
                filled,
                egui::CornerRadius::same(3),
                app.fox_theme.accent,
            );
        }

        // Total time
        ui.label(
            egui::RichText::new(format_duration(total))
                .size(14.0)
                .color(app.fox_theme.muted)
                .monospace(),
        );
    });
}

// ── Transport buttons + volume ───────────────────────────────────────────────

fn transport_row(ui: &mut egui::Ui, app: &mut LanternMusicApp) {
    ui.horizontal(|ui| {
        let btn_size = egui::vec2(36.0, 32.0);
        let text_color = app.fox_theme.text;
        let accent = app.fox_theme.accent;

        // Previous
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("\u{23EE}")
                        .size(18.0)
                        .color(text_color),
                )
                .frame(false)
                .min_size(btn_size),
            )
            .clicked()
        {
            if let Some(path) = app.queue.previous() {
                if let Some(player) = app.player.as_mut() {
                    player.play_file(&path).ok();
                }
            }
        }

        // Play / Pause
        let is_playing = app
            .player
            .as_ref()
            .map(|p| p.is_playing())
            .unwrap_or(false);

        let play_icon = if is_playing { "\u{23F8}" } else { "\u{25B6}" };
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(play_icon)
                        .size(22.0)
                        .color(accent),
                )
                .frame(false)
                .min_size(egui::vec2(44.0, 36.0)),
            )
            .clicked()
        {
            handle_play_pause(app);
        }

        // Next
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("\u{23ED}")
                        .size(18.0)
                        .color(text_color),
                )
                .frame(false)
                .min_size(btn_size),
            )
            .clicked()
        {
            if let Some(path) = app.queue.next() {
                if let Some(player) = app.player.as_mut() {
                    player.play_file(&path).ok();
                }
            }
        }

        ui.add_space(16.0);

        // Volume
        ui.label(
            egui::RichText::new("\u{1F50A}")
                .size(16.0)
                .color(app.fox_theme.muted),
        );

        let mut vol = app
            .player
            .as_ref()
            .map(|p| p.volume())
            .unwrap_or(app.config.playback.volume);

        let vol_response = ui.add(
            egui::Slider::new(&mut vol, 0.0..=1.0)
                .show_value(false)
                .trailing_fill(true),
        );

        if vol_response.changed() {
            if let Some(player) = app.player.as_mut() {
                player.set_volume(vol);
            }
        }

        if vol_response.drag_stopped() {
            app.config.playback.volume = vol;
            app.config.save();
        }
    });
}

fn handle_play_pause(app: &mut LanternMusicApp) {
    if let Some(player) = app.player.as_mut() {
        if player.is_playing() || player.is_paused() {
            player.toggle_pause();
        } else if !app.queue.tracks.is_empty() {
            // Nothing playing — start from current or first track
            let path = if let Some(track) = app.queue.current_track() {
                Some(track.path.clone())
            } else {
                app.queue.play_index(0)
            };

            if let Some(path) = path {
                player.play_file(&path).ok();
            }
        }
    }
}
