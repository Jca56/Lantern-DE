//! Getting the picture back when a monitor returns.
//!
//! A DisplayPort monitor that is switched off drops off the bus after a
//! couple of minutes, and raises hotplug again while it is still waking up.
//! The driver can refuse a modeset it cannot train the link for yet (NVIDIA:
//! `Failed to apply atomic modeset`, -22). The output was brought up with a
//! single attempt, and the connector scanner only reports connect/disconnect
//! *transitions*, so nothing ever tried again: the monitor sat on "No input"
//! until a reboot.
//!
//! Two cases, both retried until they work:
//!
//! - **The last output went away** ([`park`]): the output, its windows, its
//!   panels and its DRM surface are all kept, and only rendering stops, so
//!   clients never see the desktop lose its only screen. When the same
//!   monitor returns ([`reconnect_parked`]) the CRTC is switched off and a
//!   blank frame is committed, which makes that commit a full modeset (the
//!   link is retrained) whose verdict comes back with the call.
//! - **Any other output** is torn down as before and built fresh on return.
//!   A bring-up that fails ([`schedule_connect_retry`]) is tried again for
//!   as long as the connector stays connected.
//!
//! A hotplug event is the best hint that a monitor became ready, so anything
//! still waiting is tried again right away when one arrives ([`pending`] /
//! [`nudge`]).

use std::time::{Duration, Instant};

use smithay::backend::drm::compositor::{FrameFlags, PrimaryPlaneElement};
use smithay::backend::drm::DrmNode;
use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::drm::control::{connector, crtc, Device as ControlDevice};
use tracing::{info, warn};

use crate::render::{render_surface, CustomRenderElements};
use crate::udev::{GpuBackend, OutputSurface, UdevOutputId, UdevOutputModes, BG_COLOR};
use crate::Lantern;

/// Failed attempts at switching the CRTC off after which a relight stops
/// insisting on a full modeset and settles for an ordinary commit.
const MAX_CLEAR_FAILURES: u32 = 4;

/// Wait before the next try, given how many were made: quick at first (a
/// waking monitor is ready within a second or two), then every 5 s for as
/// long as it takes.
fn retry_delay(attempts: u32) -> Duration {
    const STEPS_MS: [u64; 5] = [250, 500, 1000, 2000, 4000];
    STEPS_MS
        .get(attempts.saturating_sub(1) as usize)
        .map(|ms| Duration::from_millis(*ms))
        .unwrap_or(Duration::from_secs(5))
}

/// Whether try number `attempt` gets a log line: the first few, then about
/// one a minute, so a monitor that stays dark for hours doesn't flood the log.
fn worth_logging(attempt: u32) -> bool {
    attempt <= 8 || attempt % 12 == 0
}

fn backend_mut(state: &mut Lantern, node: DrmNode) -> Option<&mut GpuBackend> {
    state.udev.as_mut()?.backends.get_mut(&node)
}

fn surface_mut(
    state: &mut Lantern,
    node: DrmNode,
    crtc: crtc::Handle,
) -> Option<&mut OutputSurface> {
    backend_mut(state, node)?.surfaces.get_mut(&crtc)
}

/// The registered output scanned out by `crtc`.
fn output_of(state: &Lantern, node: DrmNode, crtc: crtc::Handle) -> Option<Output> {
    state
        .workspaces
        .outputs_iter()
        .find(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .is_some_and(|id| id.device_id == node && id.crtc == crtc)
        })
        .cloned()
}

// ── Parking the last output ─────────────────────────────────────────────────

/// A returned monitor that has no picture yet. While this exists the output
/// is not rendered: each try is a blocking modeset, and the ordinary render
/// path would repeat it on every pointer motion.
pub(crate) struct Relight {
    /// Tries made so far.
    attempts: u32,
    /// The CRTC was switched off, so the next commit is a full modeset.
    cleared: bool,
    /// Failed attempts at switching it off.
    clear_failures: u32,
    timer: Option<RegistrationToken>,
    started: Instant,
}

impl Relight {
    /// Give up the relight, handing back its timer to cancel.
    pub fn into_timer(self) -> Option<RegistrationToken> {
        self.timer
    }
}

/// If the output on `crtc` is the only one left, keep it and just stop
/// rendering it. Returns false when other outputs remain (or it isn't a
/// registered output), in which case the caller tears it down as usual.
pub(crate) fn park(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) -> bool {
    let Some(output) = output_of(state, node, crtc) else {
        return false;
    };
    if state.workspaces.outputs_iter().any(|o| o.name() != output.name()) {
        return false;
    }
    let Some(surface) = surface_mut(state, node, crtc) else {
        return false;
    };
    surface.parked = true;
    surface.frame_pending = false;
    surface.frame_pending_since = None;
    surface.pending_render = false;
    let feedback = surface.pending_feedback.take();
    let timers = [
        surface.watchdog_timer.take(),
        surface.noflip_timer.take(),
        surface.relight.take().and_then(Relight::into_timer),
    ];
    for tok in timers.into_iter().flatten() {
        state.loop_handle.remove(tok);
    }
    // The frame in flight never reached a screen.
    if let Some(mut feedback) = feedback {
        feedback.discarded();
    }
    info!(
        output = %output.name(),
        "last monitor disconnected: output parked (desktop kept, rendering paused)"
    );
    true
}

/// A connector came up. If it carries the parked output, this handles it and
/// returns true: the same monitor is relit, one whose mode list isn't
/// readable yet is looked at again shortly. Returns false when the caller
/// should build a fresh output (a parked output whose monitor was swapped
/// for a different one is torn down first).
pub(crate) fn reconnect_parked(
    state: &mut Lantern,
    node: DrmNode,
    connector: &connector::Info,
    crtc: crtc::Handle,
) -> bool {
    let Some((old_crtc, output)) = parked_on_connector(state, node, connector.handle()) else {
        return false;
    };
    let name = output.name();

    // A monitor that is still waking up can report itself connected before
    // its mode list is readable. That says nothing about which monitor it is.
    if connector.modes().is_empty() {
        schedule_connect_retry(
            state,
            node,
            connector.handle(),
            &name,
            "it reports no modes yet",
        );
        return true;
    }

    let mode = surface_mut(state, node, old_crtc)
        .map(|s| s.drm_output.with_compositor(|c| c.pending_mode()));
    let same_monitor = mode.is_some_and(|m| connector.modes().contains(&m));
    if old_crtc != crtc || !same_monitor {
        info!(
            output = %name,
            "a different monitor (or CRTC) came back on a parked output: rebuilding it"
        );
        crate::udev_device::teardown_output(state, node, old_crtc);
        return false;
    }

    cancel_connect_retry(state, node, connector.handle());
    let Some(surface) = surface_mut(state, node, crtc) else {
        return false;
    };
    surface.parked = false;
    if surface.relight.is_none() {
        surface.relight = Some(Relight {
            attempts: 0,
            cleared: false,
            clear_failures: 0,
            timer: None,
            started: Instant::now(),
        });
        info!(output = %name, "monitor is back: relighting the parked output");
        relight_step(state, node, crtc);
    }
    true
}

/// The parked output driven through `connector`, with its CRTC.
fn parked_on_connector(
    state: &Lantern,
    node: DrmNode,
    connector: connector::Handle,
) -> Option<(crtc::Handle, Output)> {
    let backend = state.udev.as_ref()?.backends.get(&node)?;
    state.workspaces.outputs_iter().find_map(|o| {
        let id = o.user_data().get::<UdevOutputId>()?;
        let modes = o.user_data().get::<UdevOutputModes>()?;
        let parked = backend.surfaces.get(&id.crtc).is_some_and(|s| s.parked);
        (id.device_id == node && modes.connector_handle == connector && parked)
            .then(|| (id.crtc, o.clone()))
    })
}

/// A different output is up, so parked ones are no longer the last screen:
/// move their windows onto `onto` and let them go.
pub(crate) fn retire_parked(state: &mut Lantern, onto: &str) {
    let parked: Vec<(DrmNode, crtc::Handle)> = state
        .udev
        .iter()
        .flat_map(|u| u.backends.iter())
        .flat_map(|(node, b)| {
            b.surfaces
                .iter()
                .filter(|(_, s)| s.parked)
                .map(move |(crtc, _)| (*node, *crtc))
        })
        .collect();
    for (node, crtc) in parked {
        if let Some(output) = output_of(state, node, crtc) {
            let name = output.name();
            if name == onto {
                continue;
            }
            info!(from = %name, to = %onto, "parked output replaced by another monitor");
            state.migrate_windows_off_output(&name, onto);
        }
        crate::udev_device::teardown_output(state, node, crtc);
    }
}

/// One attempt at getting a picture onto a returned monitor. On success the
/// ordinary render path takes over; otherwise a timer makes the next attempt.
fn relight_step(state: &mut Lantern, node: DrmNode, crtc: crtc::Handle) {
    let name = output_of(state, node, crtc)
        .map(|o| o.name())
        .unwrap_or_default();
    let Some(udev) = state.udev.as_mut() else {
        return;
    };
    let Some(renderer) = udev.renderer.as_mut() else {
        return;
    };
    let Some(surface) = udev
        .backends
        .get_mut(&node)
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    let Some(relight) = surface.relight.as_mut() else {
        return;
    };
    let stale_timer = relight.timer.take();
    relight.attempts += 1;
    let attempt = relight.attempts;

    // Off first, so that the commit below is a full modeset: a flip alone
    // leaves a DisplayPort link that went down untrained.
    if !relight.cleared && relight.clear_failures < MAX_CLEAR_FAILURES {
        match surface.drm_output.with_compositor(|c| c.clear()) {
            Ok(()) => relight.cleared = true,
            Err(err) => {
                relight.clear_failures += 1;
                warn!(output = %name, attempt, ?err, "relight: could not switch the CRTC off");
            }
        }
    }
    let gave_up_forcing = relight.clear_failures >= MAX_CLEAR_FAILURES;

    // A blank frame committed synchronously, exactly as when an output is
    // first brought up: the driver's verdict comes back with the call.
    let result = if relight.cleared || gave_up_forcing {
        surface
            .drm_output
            .render_frame(
                renderer,
                &[] as &[CustomRenderElements],
                BG_COLOR,
                FrameFlags::empty(),
            )
            .map_err(|err| format!("{err:?}"))
            .map(|frame| {
                if frame.needs_sync() {
                    if let PrimaryPlaneElement::Swapchain(element) = &frame.primary_element {
                        let _ = element.sync.wait();
                    }
                }
            })
            .and_then(|()| {
                surface
                    .drm_output
                    .commit_frame()
                    .map_err(|err| format!("{err:?}"))
            })
    } else {
        Err("the CRTC could not be switched off".to_string())
    };

    let relit = match result {
        Ok(()) => surface.relight.take(),
        Err(why) => {
            if worth_logging(attempt) {
                warn!(
                    output = %name,
                    attempt,
                    retry_in_ms = retry_delay(attempt).as_millis() as u64,
                    "relight: modeset refused (monitor may still be waking up): {why}"
                );
            }
            None
        }
    };
    if relit.is_some() {
        surface.frame_pending = false;
        surface.frame_pending_since = None;
    }

    if let Some(tok) = stale_timer {
        state.loop_handle.remove(tok);
    }

    if let Some(relight) = relit {
        info!(
            output = %name,
            attempts = relight.attempts,
            took_ms = relight.started.elapsed().as_millis() as u64,
            forced_modeset = relight.cleared,
            "monitor relit"
        );
        // The link is up; now the real picture.
        render_surface(state, node, crtc);
        return;
    }

    let token = state
        .loop_handle
        .insert_source(
            Timer::from_duration(retry_delay(attempt)),
            move |_, _, state| {
                // This source is going away; don't leave its token behind.
                if let Some(r) = surface_mut(state, node, crtc).and_then(|s| s.relight.as_mut()) {
                    r.timer = None;
                }
                relight_step(state, node, crtc);
                TimeoutAction::Drop
            },
        )
        .ok();
    match surface_mut(state, node, crtc).and_then(|s| s.relight.as_mut()) {
        Some(relight) => relight.timer = token,
        None => {
            if let Some(tok) = token {
                state.loop_handle.remove(tok);
            }
        }
    }
}

// ── Retrying a bring-up that failed ─────────────────────────────────────────

/// A connected connector whose output could not be brought up yet.
pub(crate) struct ConnectRetry {
    attempts: u32,
    timer: Option<RegistrationToken>,
}

/// Bringing up `connector` failed: try again later, for as long as it stays
/// connected.
pub(crate) fn schedule_connect_retry(
    state: &mut Lantern,
    node: DrmNode,
    connector: connector::Handle,
    output_name: &str,
    why: &str,
) {
    let Some(backend) = backend_mut(state, node) else {
        return;
    };
    let retry = backend
        .connect_retries
        .entry(connector)
        .or_insert(ConnectRetry {
            attempts: 0,
            timer: None,
        });
    retry.attempts += 1;
    let attempt = retry.attempts;
    let stale_timer = retry.timer.take();
    let delay = retry_delay(attempt);
    if worth_logging(attempt) {
        warn!(
            output = %output_name,
            attempt,
            retry_in_ms = delay.as_millis() as u64,
            "could not bring the output up: {why}"
        );
    }
    if let Some(tok) = stale_timer {
        state.loop_handle.remove(tok);
    }
    let token = state
        .loop_handle
        .insert_source(Timer::from_duration(delay), move |_, _, state| {
            // This source is going away; don't leave its token behind.
            if let Some(r) =
                backend_mut(state, node).and_then(|b| b.connect_retries.get_mut(&connector))
            {
                r.timer = None;
            }
            retry_connect(state, node, connector);
            TimeoutAction::Drop
        })
        .ok();
    match backend_mut(state, node).and_then(|b| b.connect_retries.get_mut(&connector)) {
        Some(retry) => retry.timer = token,
        None => {
            if let Some(tok) = token {
                state.loop_handle.remove(tok);
            }
        }
    }
}

/// Forget a pending retry: the connector went away, or its output is up.
pub(crate) fn cancel_connect_retry(
    state: &mut Lantern,
    node: DrmNode,
    connector: connector::Handle,
) {
    let retry = backend_mut(state, node).and_then(|b| b.connect_retries.remove(&connector));
    if let Some(tok) = retry.and_then(|r| r.timer) {
        state.loop_handle.remove(tok);
    }
}

/// Try again to bring up `connector`, unless that no longer makes sense.
fn retry_connect(state: &mut Lantern, node: DrmNode, connector: connector::Handle) {
    let Some(backend) = backend_mut(state, node) else {
        return;
    };
    // The scanner's view as of the last hotplug scan. A CRTC that already
    // drives a live output needs nothing; a parked one is waiting for this.
    let target = backend
        .drm_scanner
        .connectors()
        .get(&connector)
        .filter(|info| info.state() == connector::State::Connected)
        .cloned()
        .zip(backend.drm_scanner.crtc_for_connector(&connector))
        .filter(|(_, crtc)| backend.surfaces.get(crtc).map_or(true, |s| s.parked));
    let Some((scanned, crtc)) = target else {
        cancel_connect_retry(state, node, connector);
        return;
    };
    // Probe the connector again: its mode list may not have been there yet.
    let info = backend
        .drm_output_manager
        .device()
        .get_connector(connector, true)
        .unwrap_or(scanned);
    crate::udev_device::connector_connected(state, node, info, crtc);
}

// ── Hotplug nudges ──────────────────────────────────────────────────────────

/// What was still waiting for its monitor before a hotplug scan.
pub(crate) struct Pending {
    connects: Vec<connector::Handle>,
    relights: Vec<crtc::Handle>,
}

/// Snapshot of what is waiting on `node`. Taken before the scan's events are
/// handled, so a retry they schedule isn't immediately run a second time.
pub(crate) fn pending(state: &Lantern, node: DrmNode) -> Pending {
    let backend = state.udev.as_ref().and_then(|u| u.backends.get(&node));
    Pending {
        connects: backend
            .map(|b| b.connect_retries.keys().copied().collect())
            .unwrap_or_default(),
        relights: backend
            .map(|b| {
                b.surfaces
                    .iter()
                    .filter(|(_, s)| s.relight.is_some())
                    .map(|(crtc, _)| *crtc)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A hotplug event arrived: try everything that was waiting right now
/// instead of on its timer.
pub(crate) fn nudge(state: &mut Lantern, node: DrmNode, pending: Pending) {
    for connector in pending.connects {
        let waiting = backend_mut(state, node)
            .and_then(|b| b.connect_retries.get_mut(&connector))
            .map(|r| r.timer.take());
        if let Some(timer) = waiting {
            if let Some(tok) = timer {
                state.loop_handle.remove(tok);
            }
            retry_connect(state, node, connector);
        }
    }
    for crtc in pending.relights {
        relight_step(state, node, crtc);
    }
}
