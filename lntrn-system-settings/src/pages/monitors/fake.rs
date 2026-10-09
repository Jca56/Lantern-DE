//! A compositor to test against: it holds monitors, takes a setup, and
//! does it (or doesn't) when the test says so.

use std::cell::RefCell;
use std::rc::Rc;

use super::flow::Source;
use crate::outputs::{Change, Head, Link, Mode, Outcome, View};

struct Inner {
    view: View,
    outcome: Option<Outcome>,
    asked: Vec<Vec<Change>>,
    refuse: Option<String>,
}

/// A handle on the stand-in: clones are the same compositor.
#[derive(Clone)]
pub struct Fake(Rc<RefCell<Inner>>);

fn mode(width: i32, height: i32, refresh: i32) -> Mode {
    Mode { width, height, refresh, preferred: false }
}

/// A 4K monitor at 1.4 and a 1080p one beside it, as on the desk.
pub fn two_heads() -> Vec<Head> {
    let dp = vec![mode(3840, 2160, 240_000), Mode { preferred: true, ..mode(3840, 2160, 60_000) }, mode(2560, 1440, 144_000), mode(1920, 1080, 240_000)];
    vec![
        Head { name: "DP-1".into(), enabled: true, position: (0, 0), scale: 358.0 / 256.0, physical_mm: (590, 330), modes: dp, current: Some(0) },
        Head { name: "HDMI-A-1".into(), enabled: true, position: (2743, 0), scale: 1.0, physical_mm: (0, 0), modes: vec![mode(1920, 1080, 100_000), mode(1280, 720, 60_000)], current: Some(0) },
    ]
}

impl Fake {
    pub fn with(heads: Vec<Head>) -> Self {
        Fake(Rc::new(RefCell::new(Inner { view: View { link: Link::Ready, heads, generation: 1 }, outcome: None, asked: Vec::new(), refuse: None })))
    }

    /// Every setup asked for, in order.
    pub fn asked(&self) -> Vec<Vec<Change>> {
        self.0.borrow().asked.clone()
    }

    pub fn heads(&self) -> Vec<Head> {
        self.0.borrow().view.heads.clone()
    }

    /// Turn the next apply down at once, saying `why`.
    pub fn refuse(&self, why: &str) {
        self.0.borrow_mut().refuse = Some(why.to_owned());
    }

    /// Answer the last apply. When it succeeds the monitors become what
    /// it asked for, a scale as it would come back over the wire. Either
    /// way there is a fresh look to go with the answer.
    pub fn answer(&self, outcome: Outcome) {
        let mut s = self.0.borrow_mut();
        if outcome == Outcome::Succeeded
            && let Some(asked) = s.asked.last().cloned()
        {
            for c in asked {
                let Some(h) = s.view.heads.iter_mut().find(|h| h.name == c.name) else { continue };
                h.enabled = c.enabled;
                if !c.enabled {
                    continue;
                }
                h.position = c.position;
                if let Some(mode) = c.mode {
                    h.current = Some(mode);
                }
                if let Some(scale) = c.scale {
                    h.scale = (scale * 256.0).round() / 256.0;
                }
            }
        }
        s.view.generation += 1;
        s.outcome = Some(outcome);
    }

    /// A monitor is plugged in or pulled out: these are the heads now.
    pub fn plug(&self, heads: Vec<Head>) {
        let mut s = self.0.borrow_mut();
        s.view.heads = heads;
        s.view.generation += 1;
    }

    /// The compositor goes away.
    pub fn lose(&self, why: &str) {
        let mut s = self.0.borrow_mut();
        s.view.link = Link::Lost(why.to_owned());
        s.view.generation += 1;
    }
}

impl Source for Fake {
    fn newer_than(&self, seen: u64) -> Option<View> {
        let s = self.0.borrow();
        (s.view.generation != seen).then(|| s.view.clone())
    }

    fn apply(&self, changes: Vec<Change>) -> Result<(), String> {
        let mut s = self.0.borrow_mut();
        if let Some(why) = s.refuse.take() {
            return Err(why);
        }
        s.asked.push(changes);
        Ok(())
    }

    fn take_outcome(&self) -> Option<Outcome> {
        self.0.borrow_mut().outcome.take()
    }
}
