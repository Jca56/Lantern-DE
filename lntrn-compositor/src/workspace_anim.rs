//! Horizontal slide transition when switching workspaces.
//!
//! Each output can have at most one in-flight transition at a time,
//! keyed by output name so multi-monitor switches don't interfere.
//!
//! During a transition the render loop offsets windows of both the
//! outgoing and incoming workspaces along X by the eased progress.
//! Ease-out-cubic over 280ms — snappy but not abrupt.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const SLIDE_DURATION: Duration = Duration::from_millis(280);

pub struct WorkspaceTransition {
    pub from_ws: u32,
    pub to_ws: u32,
    /// +1 when to_ws > from_ws (pan right), -1 when going back (pan left).
    pub direction: i32,
    pub start: Instant,
    pub duration: Duration,
}

impl WorkspaceTransition {
    pub fn progress(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f64();
        (elapsed / self.duration.as_secs_f64()).clamp(0.0, 1.0)
    }

    pub fn eased(&self, now: Instant) -> f64 {
        crate::easing::ease_out_cubic(self.progress(now))
    }

    pub fn is_done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    /// X offset (logical pixels) to apply to windows of the given workspace
    /// while this transition is in flight. `output_width` is the logical
    /// width of the output this transition is happening on.
    pub fn offset_for(&self, ws_id: u32, output_width: f64, now: Instant) -> Option<f64> {
        let p = self.eased(now);
        let w = output_width;
        let dir = self.direction as f64;
        if ws_id == self.from_ws {
            Some(-dir * p * w)
        } else if ws_id == self.to_ws {
            Some(dir * (1.0 - p) * w)
        } else {
            None
        }
    }

    /// Does this transition involve the given workspace?
    pub fn involves(&self, ws_id: u32) -> bool {
        ws_id == self.from_ws || ws_id == self.to_ws
    }
}

pub struct WorkspaceAnimState {
    transitions: HashMap<String, WorkspaceTransition>,
}

impl WorkspaceAnimState {
    pub fn new() -> Self {
        Self { transitions: HashMap::new() }
    }

    pub fn start(&mut self, output_name: &str, from_ws: u32, to_ws: u32) {
        if from_ws == to_ws { return; }
        let direction = if to_ws > from_ws { 1 } else { -1 };
        self.transitions.insert(
            output_name.to_string(),
            WorkspaceTransition {
                from_ws,
                to_ws,
                direction,
                start: Instant::now(),
                duration: SLIDE_DURATION,
            },
        );
    }

    pub fn get(&self, output_name: &str) -> Option<&WorkspaceTransition> {
        self.transitions.get(output_name)
    }

    /// Drop finished transitions. Returns true if any are still active.
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        self.transitions.retain(|_, t| !t.is_done(now));
        !self.transitions.is_empty()
    }

    pub fn is_active(&self) -> bool {
        !self.transitions.is_empty()
    }
}
