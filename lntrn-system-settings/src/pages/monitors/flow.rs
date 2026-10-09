//! What the Monitors page holds and how a change gets made: nothing
//! reaches the compositor until Apply, and what Apply did is on trial.
//! It is kept by a press within [`TRIAL_SECS`] and put back otherwise,
//! so a setup that can't be seen, or leaves the button out of reach,
//! undoes itself. Only a kept setup is written to the file.
//!
//! The trial is counted on the window's clock by [`MonitorsState::tick`],
//! which the app calls every frame whatever page shows: going to another
//! page doesn't stop it.

use lntrn_app::Waker;

use super::badge::Identify;
use super::canvas::Canvas;
use super::draft::{self, Draft};
use crate::config::Monitor;
use crate::outputs::{Change, Head, Link, Outcome, Outputs, View};

/// How long a setup is on trial.
pub const TRIAL_SECS: f64 = 15.0;
/// How often the page is redrawn while one is, to count it down.
const COUNT_EVERY: f64 = 0.25;

/// Where the monitors are heard from and told: the compositor, or a
/// stand-in under test.
pub trait Source {
    fn newer_than(&self, seen: u64) -> Option<View>;
    fn apply(&self, changes: Vec<Change>) -> Result<(), String>;
    fn take_outcome(&self) -> Option<Outcome>;
}

impl Source for Outputs {
    fn newer_than(&self, seen: u64) -> Option<View> {
        Outputs::newer_than(self, seen)
    }
    fn apply(&self, changes: Vec<Change>) -> Result<(), String> {
        Outputs::apply(self, changes)
    }
    fn take_outcome(&self) -> Option<Outcome> {
        Outputs::take_outcome(self)
    }
}

pub(super) enum Phase {
    Idle,
    /// Asked of the compositor, not answered. `before` is what to go
    /// back to should it take and not be kept; `None` when this is the
    /// going back.
    Applying { wanted: Vec<Draft>, before: Option<Vec<Draft>> },
    /// It took, and is put back at `until` (seconds on the window's
    /// clock) unless it is kept first.
    Trial { wanted: Vec<Draft>, before: Vec<Draft>, until: f64 },
}

/// Something to say under the buttons until the next thing happens.
pub(super) struct Notice {
    pub text: String,
    pub good: bool,
}

pub struct MonitorsState {
    waker: Option<Waker>,
    pub(super) source: Option<Box<dyn Source>>,
    /// The generation of the source's view we hold.
    seen: u64,
    pub(super) link: Link,
    pub(super) heads: Vec<Head>,
    /// What is live, and what the page has made of it. One of each a
    /// head, in the heads' order.
    pub(super) base: Vec<Draft>,
    pub(super) draft: Vec<Draft>,
    /// The draft as it was last fitted together: what an edit is an edit
    /// of, so a monitor that grows keeps its neighbours on their sides.
    fitted: Vec<Draft>,
    /// Nothing has been edited: the draft is whatever is live.
    follow: bool,
    /// Which monitor's settings show.
    pub(super) selected: usize,
    pub(super) phase: Phase,
    pub(super) notice: Option<Notice>,
    pub(super) canvas: Canvas,
    pub identify: Identify,
}

impl Default for MonitorsState {
    fn default() -> Self {
        Self { waker: None, source: None, seen: 0, link: Link::Connecting, heads: Vec::new(), base: Vec::new(), draft: Vec::new(), fitted: Vec::new(), follow: true, selected: 0, phase: Phase::Idle, notice: None, canvas: Canvas::default(), identify: Identify::default() }
    }
}

impl MonitorsState {
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Start listening to the compositor, the first time the page shows.
    /// Without a waker (a headless test) there is nothing to listen with.
    pub(super) fn connect(&mut self) {
        if self.source.is_none()
            && let Some(waker) = &self.waker
        {
            self.source = Some(Box::new(Outputs::start(waker.clone())));
        }
    }

    /// Let go of a connection that ended and make another.
    pub(super) fn reconnect(&mut self) {
        (self.source, self.seen, self.link) = (None, 0, Link::Connecting);
        self.connect();
    }

    /// Take in what the compositor said since the last frame and count
    /// the trial down. `Some` is how soon to be called again.
    pub fn tick(&mut self, cfg: &[Monitor], now: f64) -> Option<f64> {
        let source = self.source.as_ref()?;
        if let Some(view) = source.newer_than(self.seen) {
            (self.seen, self.link, self.heads) = (view.generation, view.link, view.heads);
        }
        let outcome = source.take_outcome();
        self.rebase(cfg);
        if let Some(outcome) = outcome {
            self.answered(outcome, now);
        }
        if matches!(self.phase, Phase::Trial { until, .. } if now >= until) {
            self.go_back();
        }
        matches!(self.phase, Phase::Trial { .. }).then_some(COUNT_EVERY)
    }

    /// Work out again what is live. The draft goes with it while nothing
    /// has been edited, and whenever the monitors themselves changed: an
    /// edit to a monitor that is gone is no edit.
    fn rebase(&mut self, cfg: &[Monitor]) {
        let base = draft::base(&self.heads, cfg);
        if base == self.base {
            return;
        }
        let same = base.len() == self.draft.len() && base.iter().zip(&self.draft).all(|(b, d)| b.name == d.name);
        self.base = base;
        if self.follow || !same {
            self.settle_on_live();
        }
        self.selected = self.selected.min(self.base.len().saturating_sub(1));
    }

    /// Drop every edit: the draft is what is live.
    fn settle_on_live(&mut self) {
        self.draft = self.base.clone();
        self.fitted = self.base.clone();
        self.follow = true;
        self.canvas.let_go();
    }

    fn say(&mut self, text: &str, good: bool) {
        self.notice = Some(Notice { text: text.to_owned(), good });
    }

    /// The compositor answered an apply. The view is already the fresh
    /// one: the source tells no outcome before it.
    fn answered(&mut self, outcome: Outcome, now: f64) {
        match (std::mem::replace(&mut self.phase, Phase::Idle), outcome) {
            (Phase::Applying { wanted, before: Some(before) }, Outcome::Succeeded) if self.link == Link::Ready => {
                self.settle_on_live();
                self.phase = Phase::Trial { wanted, before, until: now + TRIAL_SECS };
            }
            (Phase::Applying { before: Some(_), .. }, Outcome::Succeeded) => {
                self.settle_on_live();
                self.say("That took, but the compositor went quiet right after, so it can't be put back from here. It was not saved: a restart undoes it.", false);
            }
            (Phase::Applying { before: Some(_), .. }, Outcome::Failed) => self.say("The compositor couldn't do that. Nothing was saved.", false),
            (Phase::Applying { before: Some(_), .. }, Outcome::Cancelled) => {
                self.settle_on_live();
                self.say("The monitors changed while that was going in. Have a look and try again.", false);
            }
            (Phase::Applying { before: None, .. }, Outcome::Succeeded) => {
                self.settle_on_live();
                self.say("Back to how it was.", true);
            }
            (Phase::Applying { before: None, .. }, _) => {
                self.settle_on_live();
                self.say("It couldn't be put back. Nothing was saved, so a restart undoes it.", false);
            }
            // An answer nobody was waiting for.
            (phase, _) => self.phase = phase,
        }
    }

    /// The user changed the draft: it is theirs now, and the monitors
    /// are fitted back together around what changed.
    pub(super) fn edited(&mut self) {
        self.follow = false;
        self.notice = None;
        draft::refit(&mut self.draft, &self.heads, &self.fitted);
        self.fitted = self.draft.clone();
    }

    /// What decides which parts of the page show. When a press changes
    /// it, the page is drawn again at once rather than at the next move
    /// of the pointer.
    pub(super) fn showing(&self) -> (u8, bool, bool, usize) {
        let phase = match self.phase {
            Phase::Idle => 0,
            Phase::Applying { .. } => 1,
            Phase::Trial { .. } => 2,
        };
        (phase, self.dirty(), self.notice.is_some(), self.selected)
    }

    /// Whether Apply has anything to do.
    pub(super) fn dirty(&self) -> bool {
        self.draft != self.base
    }

    pub(super) fn idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    /// Whole seconds left of the trial, counted so the last one shown is 1.
    pub(super) fn seconds_left(&self, now: f64) -> Option<u64> {
        match self.phase {
            Phase::Trial { until, .. } => Some((until - now).ceil().max(0.0) as u64),
            _ => None,
        }
    }

    pub(super) fn reset(&mut self) {
        self.settle_on_live();
        self.notice = None;
    }

    /// Make the draft so. What only the file knows (the main monitor,
    /// which may vary its refresh) is written at once, and `true` comes
    /// back for the file to be saved; anything the compositor has to do
    /// is asked of it and goes on trial when it has.
    pub(super) fn apply(&mut self, cfg: &mut Vec<Monitor>) -> bool {
        if !self.idle() || !self.dirty() {
            return false;
        }
        self.notice = None;
        if !draft::moves_the_desktop(&self.draft, &self.base) {
            draft::keep_flags(cfg, &self.draft);
            self.base = draft::base(&self.heads, cfg);
            self.settle_on_live();
            self.say("Saved.", true);
            return true;
        }
        let Some(source) = &self.source else { return false };
        match source.apply(draft::changes(&self.draft, &self.base)) {
            Ok(()) => self.phase = Phase::Applying { wanted: self.draft.clone(), before: Some(self.base.clone()) },
            Err(why) => self.say(&format!("Not applied: {why}."), false),
        }
        false
    }

    /// Keep the setup on trial: it goes into the file. `true` when it
    /// did, for the file to be saved.
    pub(super) fn keep(&mut self, cfg: &mut Vec<Monitor>) -> bool {
        let Phase::Trial { wanted, .. } = std::mem::replace(&mut self.phase, Phase::Idle) else { return false };
        draft::keep(cfg, &self.heads, &self.base, &wanted);
        self.base = draft::base(&self.heads, cfg);
        self.settle_on_live();
        self.say("Kept.", true);
        true
    }

    /// Put the setup on trial back as it was. Told as the way from what
    /// was asked for to what was there before: that holds whatever has
    /// been heard from the compositor since.
    pub(super) fn go_back(&mut self) {
        let Phase::Trial { wanted, before, .. } = std::mem::replace(&mut self.phase, Phase::Idle) else { return };
        let Some(source) = &self.source else { return };
        match source.apply(draft::changes(&before, &wanted)) {
            Ok(()) => self.phase = Phase::Applying { wanted: before, before: None },
            Err(why) => self.say(&format!("It couldn't be put back: {why}. Nothing was saved, so a restart undoes it."), false),
        }
    }

    /// `2 · HDMI-A-1`: a monitor by its number and its name.
    pub(super) fn label(&self, i: usize) -> String {
        format!("{} · {}", i + 1, self.draft.get(i).map_or("", |d| d.name.as_str()))
    }
}

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
