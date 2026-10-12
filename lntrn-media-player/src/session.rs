//! What the player does between Wayland events: one wake-up's worth of
//! pipeline upkeep, MPRIS traffic and input, and drawing a frame when one is
//! owed. wayland.rs owns the connection and the loop; this owns everything
//! the loop drives.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use lntrn_render::Rect;
use lntrn_ui::gpu::InteractionContext;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
use wayland_protocols::xdg::shell::client::xdg_toplevel::{ResizeEdge, XdgToplevel};

use crate::app::App;
use crate::event_pump::Waker;
use crate::input::{edge_resize, handle_key, resize_edge_shape};
use crate::mpris_server::{MprisCmd, PlayerState};
use crate::render::{ControlRects, Theme};
use crate::wayland::State;
use crate::{
    Gpu, ZONE_CANVAS, ZONE_CLOSE, ZONE_FULLSCREEN, ZONE_LOOP, ZONE_MAXIMIZE, ZONE_MINIMIZE,
    ZONE_NEXT, ZONE_PLAY_PAUSE, ZONE_PREV, ZONE_SEEK_BAR, ZONE_TITLE_BAR, ZONE_VIEW_MENU,
    ZONE_VIEW_SWATCH_BASE,
};

/// How often an idle window looks for a theme change in lantern.toml.
const THEME_POLL: Duration = Duration::from_secs(1);
/// While playing, MPRIS clients get a fresh position this often (they read
/// it on demand; pushing every frame was 60 allocations a second).
const MPRIS_POSITION_PUSH: Duration = Duration::from_millis(250);
/// Scroll distance of one wheel notch; each notch is one volume step.
const SCROLL_NOTCH: f32 = 15.0;
const VOLUME_STEP: f64 = 0.05;
/// Width of the window edge band that triggers a resize, logical px.
const RESIZE_BORDER: f32 = 10.0;

pub struct Session {
    app: App,
    input: InteractionContext,
    /// Where the seek bar was last drawn (drag mapping).
    rects: ControlRects,
    theme: Theme,
    theme_checked: Instant,
    theme_stamp: Option<SystemTime>,
    last_title: String,
    last_frame: Instant,
    /// Scroll not yet turned into a volume step (a trackpad sends many small
    /// deltas; a wheel sends one notch at a time).
    scroll_accum: f32,
    mpris_tx: Sender<PlayerState>,
    mpris_rx: Receiver<MprisCmd>,
    mpris_sent: Option<PlayerState>,
    mpris_sent_at: Instant,
}

impl Session {
    pub fn new(
        initial_path: Option<&Path>,
        waker: Waker,
        mpris_tx: Sender<PlayerState>,
        mpris_rx: Receiver<MprisCmd>,
    ) -> Self {
        let mut app = App::new(waker);
        if let Some(path) = initial_path {
            app.open_file(path);
        }
        let now = Instant::now();
        Self {
            app,
            input: InteractionContext::new(),
            rects: ControlRects {
                seek: Rect::new(0.0, 0.0, 0.0, 0.0),
            },
            theme: Theme::load(),
            theme_checked: now,
            theme_stamp: theme_stamp(),
            last_title: String::new(),
            last_frame: now,
            scroll_accum: 0.0,
            mpris_tx,
            mpris_rx,
            mpris_sent: None,
            mpris_sent_at: now,
        }
    }

    // ── Loop pacing ─────────────────────────────────────────────────────

    /// The next moment something is due without any event to announce it.
    pub fn next_deadline(&self, state: &State, now: Instant) -> Option<Instant> {
        let theme = Some(self.theme_checked + THEME_POLL);
        [self.app.next_deadline(now), state.keyboard.repeat_at(), theme]
            .into_iter()
            .flatten()
            .min()
    }

    /// Something is easing (controls fading, bars moving, the volume readout
    /// going away) and wants the next frame as soon as this one is shown.
    pub fn is_animating(&self) -> bool {
        self.app.is_animating(Instant::now())
    }

    // ── One wake-up ─────────────────────────────────────────────────────

    /// Runs on every wake-up, drawn frame or not: a minimized window gets no
    /// frame callbacks, and playback, MPRIS and input mustn't wait for one.
    pub fn on_wake(&mut self, state: &mut State, toplevel: &XdgToplevel, gpu: &Gpu) {
        let now = Instant::now();
        let s = state.fractional_scale() as f32;
        let wf = gpu.ctx.width() as f32;
        let win_hf = gpu.ctx.height() as f32;

        if self.app.tick(now) {
            state.needs_redraw = true;
        }
        self.handle_mpris(state);

        if now >= self.theme_checked + THEME_POLL {
            self.theme_checked = now;
            let stamp = theme_stamp();
            if stamp != self.theme_stamp {
                self.theme_stamp = stamp;
                self.theme = Theme::load();
                state.needs_redraw = true;
            }
        }
        state.keyboard.tick_repeat(now);

        // ── Pointer ─────────────────────────────────────────────────────
        let cx = state.cursor_x as f32 * s;
        let cy = state.cursor_y as f32 * s;
        if state.pointer_in_surface {
            self.input.on_cursor_moved(cx, cy);
            // Controls stay up while the pointer is here, and for a moment
            // after it leaves.
            self.app.note_pointer_activity();
        } else {
            self.input.on_cursor_left();
        }
        self.app.pointer_in_window = state.pointer_in_surface;

        // Controls own the bottom edge while visible; the title bar owns the
        // top edge in video mode (always) and in audio mode whenever it's
        // faded in on hover.
        let controls_visible = self.app.controls_alpha > 0.05;
        let title_visible = !state.fullscreen
            && (self.app.video_mode() || (self.app.audio_only && controls_visible));
        let edge = if state.maximized || state.fullscreen {
            None
        } else {
            let border = RESIZE_BORDER * s;
            edge_resize(cx, cy, wf, win_hf, border, controls_visible, title_visible)
        };
        if state.pointer_in_surface {
            let desired = edge.map_or(Shape::Default, resize_edge_shape);
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.pointer_enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // Seek bar drag.
        if self.app.seeking && state.pointer_in_surface && self.rects.seek.w > 0.0 {
            self.app.seek_value = ((cx - self.rects.seek.x) / self.rects.seek.w).clamp(0.0, 1.0);
        }

        // ── Keyboard / scroll / buttons ─────────────────────────────────
        while let Some(press) = state.keyboard.pop() {
            handle_key(&mut self.app, toplevel, state, press);
            state.needs_redraw = true;
        }
        self.handle_scroll(state);
        if std::mem::take(&mut state.left_pressed) {
            self.handle_left_press(state, toplevel, edge, cx);
        }
        if std::mem::take(&mut state.left_released) {
            if self.app.seeking {
                self.app.seek_to_fraction(self.app.seek_value);
            }
            self.input.on_left_released();
        }

        if self.app.is_animating(now) {
            state.needs_redraw = true;
        }
        self.push_mpris(now);
    }

    /// Transport commands from the Command Center / media keys. Drained on
    /// every wake-up — the MPRIS thread pokes the waker when it queues one.
    fn handle_mpris(&mut self, state: &mut State) {
        while let Ok(cmd) = self.mpris_rx.try_recv() {
            match cmd {
                MprisCmd::PlayPause => self.app.toggle_play_pause(),
                MprisCmd::Play => self.app.play(),
                MprisCmd::Pause | MprisCmd::Stop => self.app.pause(),
                MprisCmd::Next => {
                    self.app.next_track();
                }
                MprisCmd::Previous => {
                    self.app.prev_track();
                }
                // Microseconds → nanoseconds; a wild offset just clamps.
                MprisCmd::Seek(offset_us) => self.app.seek_relative(offset_us.saturating_mul(1000)),
            }
            state.needs_redraw = true;
        }
    }

    /// Wheel → volume, one step per notch however the scroll arrives.
    fn handle_scroll(&mut self, state: &mut State) {
        self.scroll_accum += std::mem::take(&mut state.scroll_delta);
        let notches = (self.scroll_accum / SCROLL_NOTCH).trunc();
        if notches != 0.0 {
            self.scroll_accum -= notches * SCROLL_NOTCH;
            // Scrolling up (negative) turns it up.
            self.app.adjust_volume(-notches as f64 * VOLUME_STEP);
            state.needs_redraw = true;
        }
    }

    fn handle_left_press(
        &mut self,
        state: &mut State,
        toplevel: &XdgToplevel,
        edge: Option<ResizeEdge>,
        cx: f32,
    ) {
        if let Some(edge) = edge {
            if let Some(seat) = &state.seat {
                toplevel.resize(seat, state.pointer_serial, edge);
            }
            return;
        }
        let Some(zone_id) = self.input.on_left_pressed() else {
            return;
        };
        let swatches = ZONE_VIEW_SWATCH_BASE
            ..ZONE_VIEW_SWATCH_BASE + crate::vis_theme::theme_count() as u32;
        // Any click outside the View menu/button closes it.
        if zone_id != ZONE_VIEW_MENU && !swatches.contains(&zone_id) {
            self.app.view_menu_open = false;
        }
        match zone_id {
            ZONE_PLAY_PAUSE => self.app.toggle_play_pause(),
            ZONE_VIEW_MENU => self.app.view_menu_open = !self.app.view_menu_open,
            z if swatches.contains(&z) => {
                self.app.set_vis_theme((z - ZONE_VIEW_SWATCH_BASE) as usize);
                self.app.view_menu_open = false;
            }
            ZONE_PREV => {
                self.app.prev_track();
            }
            ZONE_NEXT => {
                self.app.next_track();
            }
            ZONE_SEEK_BAR => {
                if self.rects.seek.w > 0.0 {
                    self.app.seeking = true;
                    self.app.seek_value =
                        ((cx - self.rects.seek.x) / self.rects.seek.w).clamp(0.0, 1.0);
                }
            }
            // The title bar — and the whole canvas — is a move handle.
            // Playback toggles via the controls or spacebar instead.
            ZONE_TITLE_BAR | ZONE_CANVAS => {
                if let Some(seat) = &state.seat {
                    toplevel._move(seat, state.pointer_serial);
                }
            }
            ZONE_CLOSE => state.running = false,
            ZONE_MAXIMIZE => {
                if state.maximized {
                    toplevel.unset_maximized();
                } else {
                    toplevel.set_maximized();
                }
            }
            ZONE_MINIMIZE => toplevel.set_minimized(),
            ZONE_FULLSCREEN => {
                if state.fullscreen {
                    toplevel.unset_fullscreen();
                } else {
                    toplevel.set_fullscreen(None);
                }
            }
            ZONE_LOOP => self.app.cycle_loop_mode(),
            _ => {}
        }
    }

    /// Tell the MPRIS server what's playing. Sent when something changed;
    /// while playing, the ticking position alone is throttled.
    fn push_mpris(&mut self, now: Instant) {
        let snapshot = PlayerState {
            title: self.app.file_name.clone(),
            url: self.app.file_uri.clone(),
            has_media: self.app.pipeline.is_some(),
            playing: self.app.is_playing(),
            can_go_next: self.app.can_go_next(),
            can_go_previous: self.app.can_go_previous(),
            position_ns: self.app.position_ns,
            duration_ns: self.app.duration_ns,
            volume: self.app.volume,
        };
        let send = match &self.mpris_sent {
            None => true,
            Some(prev) if *prev == snapshot => false,
            Some(prev) => {
                let only_position = PlayerState {
                    position_ns: prev.position_ns,
                    ..snapshot.clone()
                } == *prev;
                !(only_position && snapshot.playing)
                    || now.duration_since(self.mpris_sent_at) >= MPRIS_POSITION_PUSH
            }
        };
        if send {
            let _ = self.mpris_tx.send(snapshot.clone());
            self.mpris_sent = Some(snapshot);
            self.mpris_sent_at = now;
        }
    }

    // ── Drawing ─────────────────────────────────────────────────────────

    pub fn render(&mut self, state: &State, gpu: &mut Gpu, toplevel: &XdgToplevel) {
        // Window title follows the open file, however it changed (buttons,
        // keys, MPRIS, the folder advancing on its own).
        let title = if self.app.file_name.is_empty() {
            "Lantern Media Player".to_string()
        } else {
            format!("{} — Lantern Media Player", self.app.file_name)
        };
        if title != self.last_title {
            toplevel.set_title(title.clone());
            self.last_title = title;
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.app.prepare_frame(&gpu.ctx, &gpu.tex_pass, now, dt);

        let win_hf = gpu.ctx.height() as f32;
        self.rects = crate::render::render_frame(
            gpu,
            &self.app,
            &mut self.input,
            &self.theme,
            state.fractional_scale() as f32,
            win_hf,
            state.fullscreen,
            now,
        );
    }

    // ── Closing ─────────────────────────────────────────────────────────

    /// Remember where we left off so reopening the same file resumes there.
    pub fn save_position(&mut self) {
        self.app.save_current_position();
    }

    /// Bounded Null transition — see `MediaPipeline`'s Drop.
    pub fn drop_pipeline(&mut self) {
        self.app.pipeline = None;
    }
}

/// When lantern.toml (theme, accent, opacity, corner radius) last changed.
fn theme_stamp() -> Option<SystemTime> {
    let path = lntrn_theme::config::lantern_config_path()?;
    std::fs::metadata(path).ok()?.modified().ok()
}
