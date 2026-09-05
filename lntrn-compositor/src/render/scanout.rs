//! Primary scan-out output tracking: which output each surface was actually
//! presented on this frame, and the frame-callback / presentation-feedback
//! fan-out that depends on it.
//!
//! Before this existed, every output's vblank sent `wl_surface.frame` to
//! every window in the global Space with a closure that always answered
//! "yes, this is your primary output". Smithay sends a callback when the
//! surface is on the primary output OR its throttle expired, so that closure
//! made the throttle a no-op: a client got callbacks at the combined rate of
//! every monitor, windows on hidden workspaces kept rendering at full rate,
//! and a hidden client committing into an otherwise idle output could drive
//! an unbounded empty-render loop through the no-flip callback path.
//!
//! Now the DRM compositor's per-element render states are folded into each
//! surface after every render (`update_primary_scanout_outputs`), and the
//! callback / feedback senders consult that. Surfaces not presented on the
//! flipping output only get the `HIDDEN_SURFACE_THROTTLE` trickle, which is
//! the standard "occluded surface" rate.

use std::time::Duration;

use smithay::backend::renderer::element::{
    default_primary_scanout_output_compare, RenderElementStates,
};
use smithay::desktop::utils::{
    send_frames_surface_tree, surface_presentation_feedback_flags_from_states,
    surface_primary_scanout_output, take_presentation_feedback_surface_tree,
    update_surface_primary_scanout_output, with_surfaces_surface_tree,
    OutputPresentationFeedback,
};
use smithay::input::pointer::CursorImageStatus;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::IsAlive;
use smithay::wayland::compositor::SurfaceData;

use crate::Lantern;

/// Frame-callback rate for surfaces that are NOT presented on the output
/// whose vblank is firing (other monitor, hidden workspace, fully occluded).
/// One callback per second keeps such clients from stalling on a pending
/// `wl_surface.frame` without letting them render invisibly at refresh rate.
pub const HIDDEN_SURFACE_THROTTLE: Duration = Duration::from_secs(1);

/// Every surface the render path can present: windows in the global Space
/// (all workspaces — hidden ones must be visited so a previous "primary =
/// this output" mark gets cleared), layer surfaces, lock surfaces and a
/// client-provided cursor surface.
fn for_each_presentable_surface<F>(state: &Lantern, mut f: F)
where
    F: FnMut(&WlSurface, &SurfaceData),
{
    for window in state.space.elements() {
        window.with_surfaces(&mut f);
    }
    for ls in &state.layer_surfaces {
        if ls.alive() {
            with_surfaces_surface_tree(ls.wl_surface(), &mut f);
        }
    }
    if let Some(lock) = state.session_lock.as_ref() {
        for ls in lock.surfaces.values() {
            if ls.alive() {
                with_surfaces_surface_tree(ls.wl_surface(), &mut f);
            }
        }
    }
    if let CursorImageStatus::Surface(ref surface) = state.cursor.status {
        if surface.alive() {
            with_surfaces_surface_tree(surface, &mut f);
        }
    }
}

/// Fold this frame's render element states into every surface's primary
/// scan-out output. Call once per `render_frame` on `output`, whether or not
/// the frame produced damage — an empty frame still reports which elements
/// are on screen.
pub(crate) fn update_primary_scanout_outputs(
    state: &Lantern,
    output: &Output,
    states: &RenderElementStates,
) {
    for_each_presentable_surface(state, |surface, data| {
        update_surface_primary_scanout_output(
            surface,
            output,
            data,
            states,
            default_primary_scanout_output_compare,
        );
    });
}

/// Send `wl_surface.frame` callbacks for `output`. Surfaces whose primary
/// scan-out output is `output` are answered now; everything else only when
/// its `HIDDEN_SURFACE_THROTTLE` trickle is due.
pub(crate) fn send_frame_callbacks(state: &Lantern, output: &Output, time: Duration) {
    let throttle = Some(HIDDEN_SURFACE_THROTTLE);
    for window in state.space.elements() {
        window.send_frame(output, time, throttle, surface_primary_scanout_output);
    }
    for ls in &state.layer_surfaces {
        if ls.alive() {
            send_frames_surface_tree(
                ls.wl_surface(),
                output,
                time,
                throttle,
                surface_primary_scanout_output,
            );
        }
    }
    if let Some(lock) = state.session_lock.as_ref() {
        for ls in lock.surfaces.values() {
            if ls.alive() {
                send_frames_surface_tree(
                    ls.wl_surface(),
                    output,
                    time,
                    throttle,
                    surface_primary_scanout_output,
                );
            }
        }
    }
    if let CursorImageStatus::Surface(ref surface) = state.cursor.status {
        if surface.alive() {
            send_frames_surface_tree(
                surface,
                output,
                time,
                throttle,
                surface_primary_scanout_output,
            );
        }
    }
}

/// Move out the pending `wp_presentation` feedback of every surface whose
/// primary scan-out output is `output`. Feedback for surfaces living on the
/// other monitor stays queued for that monitor's own flip — previously it
/// was collected (and on a no-flip frame, discarded) by whichever output
/// happened to render first.
pub(crate) fn collect_presentation_feedback(
    state: &Lantern,
    output: &Output,
    states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut feedback = OutputPresentationFeedback::new(output);
    let mut take = |surface: &WlSurface| {
        take_presentation_feedback_surface_tree(
            surface,
            &mut feedback,
            surface_primary_scanout_output,
            |surface, _| surface_presentation_feedback_flags_from_states(surface, states),
        );
    };
    for window in state.space.elements() {
        if let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(window) {
            take(&surface);
        }
    }
    for ls in &state.layer_surfaces {
        if ls.alive() {
            take(ls.wl_surface());
        }
    }
    if let Some(lock) = state.session_lock.as_ref() {
        for ls in lock.surfaces.values() {
            if ls.alive() {
                take(ls.wl_surface());
            }
        }
    }
    if let CursorImageStatus::Surface(ref surface) = state.cursor.status {
        if surface.alive() {
            take(surface);
        }
    }
    feedback
}
