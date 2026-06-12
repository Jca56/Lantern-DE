use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::animations;

/// Animation render parameters.
///
/// * `alpha` — always-applied alpha multiplier.
/// * `scale` — uniform scale around the geometry center. Always 1.0 today;
///   kept for forward compat.
/// * `slide` — optional shrink/grow trajectory: `(render_loc, (scale_x, scale_y))`.
///   When `Some`, the renderer overrides the effective rect with this
///   anisotropic slide+shrink (same path as minimize). `None` = pure alpha.
pub struct AnimParams {
    pub alpha: f32,
    pub scale: f64,
    pub slide: Option<(Point<f64, Logical>, (f64, f64))>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind {
    Open,
    Close,
}

/// One "endpoint" rect for the slide trajectory. `Live` means "use whatever
/// the renderer's live eff rect is at render time" — needed for open, where
/// the window's final position is set asynchronously after first commit.
#[derive(Debug, Clone, Copy)]
enum SlideEnd {
    Fixed(Rectangle<i32, Logical>),
    Live,
}

#[derive(Debug, Clone)]
pub struct WindowAnimation {
    pub kind: AnimationKind,
    start_time: Instant,
    duration: Duration,
    /// Easing curve, resolved from the `[animations]` preset at construction
    /// so render_params doesn't re-read config (and the preset can't change
    /// mid-flight) — this runs every frame for every animated window.
    curve: crate::rect_anim::Curve,
    /// Starting alpha for interruption (close mid-open).
    start_alpha: f32,
    /// Endpoints of the slide trajectory. When both are None, no slide is
    /// produced — pure alpha. For open: source is `Fixed(small_at_center)`
    /// and target is `Live`. For close: source is `Live` (window's current
    /// rect) and target is `Fixed(icon_bottom_middle)`. The zombie close
    /// path uses `Fixed` for source since the window is already dead.
    source: Option<SlideEnd>,
    target: Option<SlideEnd>,
}

impl WindowAnimation {
    fn new_open() -> Self {
        Self {
            kind: AnimationKind::Open,
            start_time: Instant::now(),
            duration: animations::open_duration(),
            curve: animations::open_curve(),
            start_alpha: 0.0,
            source: None,
            target: None,
        }
    }

    fn new_open_grow(source: Rectangle<i32, Logical>) -> Self {
        Self {
            kind: AnimationKind::Open,
            start_time: Instant::now(),
            duration: animations::open_duration(),
            curve: animations::open_curve(),
            start_alpha: 0.0,
            source: Some(SlideEnd::Fixed(source)),
            target: Some(SlideEnd::Live),
        }
    }

    fn new_close(target: Rectangle<i32, Logical>) -> Self {
        Self {
            kind: AnimationKind::Close,
            start_time: Instant::now(),
            duration: animations::close_duration(),
            curve: animations::close_curve(),
            start_alpha: 1.0,
            source: Some(SlideEnd::Live),
            target: Some(SlideEnd::Fixed(target)),
        }
    }

    fn new_close_zombie(
        source: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
    ) -> Self {
        Self {
            kind: AnimationKind::Close,
            start_time: Instant::now(),
            duration: animations::close_duration(),
            curve: animations::close_curve(),
            start_alpha: 1.0,
            source: Some(SlideEnd::Fixed(source)),
            target: Some(SlideEnd::Fixed(target)),
        }
    }

    /// Create a close animation that starts from a mid-open state.
    fn new_interrupted(current_alpha: f32, target: Rectangle<i32, Logical>) -> Self {
        let fraction = current_alpha as f64;
        let duration_ms =
            (animations::close_duration().as_millis() as f64 * fraction).max(80.0) as u64;
        Self {
            kind: AnimationKind::Close,
            start_time: Instant::now(),
            duration: Duration::from_millis(duration_ms),
            curve: animations::close_curve(),
            start_alpha: current_alpha,
            source: Some(SlideEnd::Live),
            target: Some(SlideEnd::Fixed(target)),
        }
    }

    /// Linear progress 0.0..1.0, clamped.
    fn raw_progress(&self) -> f64 {
        let elapsed = self.start_time.elapsed();
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
    }

    pub fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Returns animation render parameters given the renderer's live state.
    /// `live_eff` is the eff rect the renderer would use without any slide
    /// override; `live_win_size` is `window.geometry().size`. Both are read
    /// fresh each frame so the animation tracks late position/size updates
    /// (e.g. `pending_center` firing after first commit).
    pub fn render_params(
        &self,
        live_eff: Rectangle<f64, Logical>,
        live_win_size: Size<i32, Logical>,
    ) -> AnimParams {
        let t = self.raw_progress();
        let p = self.curve.eval(t);
        let pf = (p as f32).clamp(0.0, 1.0);
        let alpha = match self.kind {
            AnimationKind::Open => self.start_alpha + (1.0 - self.start_alpha) * pf,
            AnimationKind::Close => self.start_alpha * (1.0 - pf),
        };

        let slide = match (self.source, self.target) {
            (Some(s), Some(t)) => {
                let src = resolve_end(s, live_eff);
                let tgt = resolve_end(t, live_eff);
                Some(compute_slide(src, tgt, live_win_size, p))
            }
            _ => None,
        };

        AnimParams { alpha, scale: 1.0, slide }
    }
}

fn resolve_end(end: SlideEnd, live_eff: Rectangle<f64, Logical>) -> Rectangle<f64, Logical> {
    match end {
        SlideEnd::Fixed(r) => Rectangle::new(
            (r.loc.x as f64, r.loc.y as f64).into(),
            (r.size.w as f64, r.size.h as f64).into(),
        ),
        SlideEnd::Live => live_eff,
    }
}

/// Anisotropic slide+scale producing render params consumed by the renderer.
///
/// Renderer formula (`win_size` is the window's intrinsic geometry size):
///   `visible_size = win_size * scale`
///   `visible_pos = render_loc + (win_size - visible_size) / 2`
///
/// To make `visible_rect` equal `source` at p=0 and `target` at p=1, we
/// scale by `lerp(source.size, target.size, p) / win_size` and offset by
/// `lerp(source.center, target.center, p) - win_size / 2`.
fn compute_slide(
    source: Rectangle<f64, Logical>,
    target: Rectangle<f64, Logical>,
    win_size: Size<i32, Logical>,
    p: f64,
) -> (Point<f64, Logical>, (f64, f64)) {
    let sw = source.size.w;
    let sh = source.size.h;
    let tw = target.size.w;
    let th = target.size.h;
    let ww = win_size.w as f64;
    let wh = win_size.h as f64;

    let visible_w = sw + (tw - sw) * p;
    let visible_h = sh + (th - sh) * p;
    let scale_x = if ww > 0.0 { visible_w / ww } else { 1.0 };
    let scale_y = if wh > 0.0 { visible_h / wh } else { 1.0 };

    let src_cx = source.loc.x + sw / 2.0;
    let src_cy = source.loc.y + sh / 2.0;
    let tgt_cx = target.loc.x + tw / 2.0;
    let tgt_cy = target.loc.y + th / 2.0;
    let cur_cx = src_cx + (tgt_cx - src_cx) * p;
    let cur_cy = src_cy + (tgt_cy - src_cy) * p;

    (Point::from((cur_cx - ww / 2.0, cur_cy - wh / 2.0)), (scale_x, scale_y))
}

/// A window that died (client-initiated close) but still has a close animation playing.
/// We render a fading shadow effect at its last known position.
pub struct ClosingWindow {
    pub surface: WlSurface,
    pub location: Point<i32, Logical>,
    pub size: Size<i32, Logical>,
    pub had_ssd: bool,
}

/// Tracks all active window animations.
pub struct AnimationState {
    animations: HashMap<WlSurface, WindowAnimation>,
    /// Surfaces whose compositor-initiated close animation already finished.
    /// Prevents double-animation when the client dies after we sent request_close.
    close_done: HashSet<WlSurface>,
}

impl AnimationState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
            close_done: HashSet::new(),
        }
    }

    /// Start an open animation for a window. Pure alpha fade.
    pub fn start_open(&mut self, surface: &WlSurface) {
        self.animations
            .insert(surface.clone(), WindowAnimation::new_open());
    }

    /// Start an open animation that grows the window from `source` (a small
    /// rect at the output center) to the window's live position while fading
    /// in. Used by presets like Springy.
    pub fn start_open_grow(&mut self, surface: &WlSurface, source: Rectangle<i32, Logical>) {
        self.animations
            .insert(surface.clone(), WindowAnimation::new_open_grow(source));
    }

    /// Start a close animation for a live window. Returns true if started.
    /// If the window is mid-open, interrupts and reverses from current state.
    /// `target` is the shrink-target (bottom-middle icon-sized rect, same as
    /// minimize). The source rect is read live each frame from the renderer.
    pub fn start_close(
        &mut self,
        surface: &WlSurface,
        target: Rectangle<i32, Logical>,
    ) -> bool {
        if let Some(anim) = self.animations.get(surface) {
            if anim.kind == AnimationKind::Close {
                return false;
            }
            // Interrupt mid-open: capture current alpha and reverse.
            // We pass a dummy live_eff here because we only need `alpha`.
            let dummy_eff = Rectangle::new((0.0, 0.0).into(), (0.0, 0.0).into());
            let dummy_ws = Size::from((0, 0));
            let params = anim.render_params(dummy_eff, dummy_ws);
            self.animations.insert(
                surface.clone(),
                WindowAnimation::new_interrupted(params.alpha, target),
            );
            return true;
        }
        self.animations
            .insert(surface.clone(), WindowAnimation::new_close(target));
        true
    }

    /// Start a close animation for a zombie (already-dead) window using the
    /// captured snapshot rect as the slide source. Used by the dead-window
    /// detector in udev/winit.
    pub fn start_close_zombie(
        &mut self,
        surface: &WlSurface,
        source: Rectangle<i32, Logical>,
        target: Rectangle<i32, Logical>,
    ) {
        self.animations.insert(
            surface.clone(),
            WindowAnimation::new_close_zombie(source, target),
        );
    }

    /// Get the current animation for a surface, if any.
    pub fn get(&self, surface: &WlSurface) -> Option<&WindowAnimation> {
        self.animations.get(surface)
    }

    /// Returns true if any animations are currently active.
    pub fn has_active(&self) -> bool {
        !self.animations.is_empty()
    }

    /// Remove a specific animation (e.g., when a close animation finishes).
    pub fn remove(&mut self, surface: &WlSurface) {
        self.animations.remove(surface);
        self.close_done.remove(surface);
    }

    /// Mark that a compositor-initiated close animation finished (Super+Q path).
    /// The surface will be ignored when it later dies as a dead window.
    pub fn mark_close_done(&mut self, surface: &WlSurface) {
        self.close_done.insert(surface.clone());
    }

    /// Check and consume the close_done flag. Returns true if the surface
    /// already had its close animation (don't animate again).
    pub fn take_close_done(&mut self, surface: &WlSurface) -> bool {
        self.close_done.remove(surface)
    }

    /// Tick all animations and return surfaces whose close animations just finished.
    /// Also removes finished open animations silently.
    pub fn tick(&mut self) -> Vec<WlSurface> {
        let mut finished_closes = Vec::new();

        self.animations.retain(|surface, anim| {
            if anim.is_finished() {
                if anim.kind == AnimationKind::Close {
                    finished_closes.push(surface.clone());
                }
                false
            } else {
                true
            }
        });

        finished_closes
    }
}
