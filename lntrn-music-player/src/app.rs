use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::config::MusicConfig;
use crate::library;
use crate::player::Player;
use crate::queue::PlayQueue;
use crate::theme::{FoxTheme, ThemeName};
use crate::track::Track;
use crate::ui;

// ── Scan state ───────────────────────────────────────────────────────────────

pub enum ScanState {
    AskUser,
    Idle,
    Scanning(mpsc::Receiver<Vec<Track>>),
}

// ── Main application ─────────────────────────────────────────────────────────

pub struct LanternMusicApp {
    pub config: MusicConfig,
    pub theme_name: ThemeName,
    pub fox_theme: FoxTheme,
    pub player: Option<Player>,
    pub queue: PlayQueue,
    pub scan_state: ScanState,
    pub error_msg: Option<String>,
}

impl LanternMusicApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: MusicConfig,
        file_args: Vec<PathBuf>,
    ) -> Self {
        let (theme_name, fox_theme) = match config.general.theme.as_str() {
            "lantern" => (ThemeName::Lantern, FoxTheme::lantern()),
            _ => (ThemeName::Fox, FoxTheme::dark()),
        };
        fox_theme.apply(&cc.egui_ctx);

        let mut player = Player::new(config.playback.volume);
        let error_msg = if player.is_none() {
            Some("Failed to initialize audio output".to_string())
        } else {
            None
        };

        let mut queue = PlayQueue::new();

        // If files were passed on the command line, load and play immediately
        let scan_state = if !file_args.is_empty() {
            let tracks: Vec<Track> = file_args
                .iter()
                .map(|p| library::read_track_metadata(p))
                .collect();
            queue.set_tracks(tracks);

            if let Some(path) = queue.play_index(0) {
                if let Some(ref mut p) = player {
                    p.play_file(&path).ok();
                }
            }
            ScanState::Idle
        } else {
            let music_dir = &config.general.music_dir;
            if Path::new(music_dir).is_dir() {
                ScanState::AskUser
            } else {
                ScanState::Idle
            }
        };

        Self {
            config,
            theme_name,
            fox_theme,
            player,
            queue,
            scan_state,
            error_msg,
        }
    }

    pub fn start_scan(&mut self) {
        let dir = self.config.general.music_dir.clone();
        let dir_path = std::path::PathBuf::from(&dir);

        if !dir_path.is_dir() {
            self.error_msg = Some(format!("Directory not found: {}", dir));
            self.scan_state = ScanState::Idle;
            return;
        }

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let tracks = library::scan_directory(&dir_path);
            tx.send(tracks).ok();
        });

        self.scan_state = ScanState::Scanning(rx);
    }
}

impl eframe::App for LanternMusicApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for scan completion
        if let ScanState::Scanning(ref rx) = self.scan_state {
            if let Ok(tracks) = rx.try_recv() {
                self.queue.set_tracks(tracks);
                self.scan_state = ScanState::Idle;
            }
        }

        // Auto-advance when a track finishes
        let should_advance = self
            .player
            .as_ref()
            .is_some_and(|p| p.track_finished());

        if should_advance {
            if let Some(path) = self.queue.next() {
                if let Some(player) = self.player.as_mut() {
                    player.play_file(&path).ok();
                }
            } else if let Some(player) = self.player.as_mut() {
                player.clear_finished();
            }
        }

        // Keyboard: Space to toggle play/pause
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            if let Some(player) = self.player.as_mut() {
                if player.is_playing() || player.is_paused() {
                    player.toggle_pause();
                } else if !self.queue.tracks.is_empty() {
                    let path = if let Some(track) = self.queue.current_track() {
                        Some(track.path.clone())
                    } else {
                        self.queue.play_index(0)
                    };
                    if let Some(path) = path {
                        player.play_file(&path).ok();
                    }
                }
            }
        }

        // Paint rounded background
        let full = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::background())
            .rect_filled(full, egui::CornerRadius::same(10), self.fox_theme.bg);

        // Render UI panels
        ui::title_bar::render(ctx, self);
        ui::controls::render(ctx, self);
        ui::track_list::render(ctx, self);

        // Resize handles
        ui::window::handle_resize_edges(ctx);

        // Request repaint while playing or scanning
        let needs_repaint = self
            .player
            .as_ref()
            .is_some_and(|p| p.is_playing())
            || matches!(self.scan_state, ScanState::Scanning(_));

        if needs_repaint {
            ctx.request_repaint();
        }
    }
}
