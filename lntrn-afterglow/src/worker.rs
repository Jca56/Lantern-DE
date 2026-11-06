//! The sampler's thread: once a second it reads the hardware and asks
//! the helper how things stand, leaves the newest look where the window
//! can pick it up, and wakes the window. It is also the one that talks to
//! the helper for the window, so an Apply never stalls a frame.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use lntrn_afterglowd::PROTOCOL;
use lntrn_afterglowd::client::Client;
use lntrn_afterglowd::tune::Tune;
use lntrn_afterglowd::wire::Hello;
use lntrn_app::Waker;

use crate::sample::{Frame, Helper, Sampler};

/// How long after starting the first look is taken: long enough for the
/// load counters to have moved.
const FIRST_LOOK: Duration = Duration::from_millis(300);
const BEAT: Duration = Duration::from_secs(1);

/// What the window asks of the thread.
pub enum Cmd {
    /// Try a tune: it undoes itself unless kept.
    Apply(Tune),
    Keep,
    Revert,
    Reset,
    /// Look now, whatever the beat.
    Now,
}

#[derive(Default)]
struct Shared {
    frame: Mutex<Option<Arc<Frame>>>,
    /// What the window should say: how a command went.
    said: Mutex<Vec<String>>,
    /// A command is on its way to the helper.
    busy: AtomicBool,
    waker: OnceLock<Waker>,
}

pub struct Link {
    /// `None` when there is no thread: a frame fixed for a test.
    tx: Option<Sender<Cmd>>,
    shared: Arc<Shared>,
    /// What a test's fixed link was sent.
    #[cfg(test)]
    pub sent: std::cell::RefCell<Vec<String>>,
}

impl Link {
    /// Start the thread, asking the helper at `socket`.
    pub fn spawn(socket: PathBuf) -> Link {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared::default());
        let theirs = shared.clone();
        if let Err(e) = std::thread::Builder::new().name("sampler".into()).spawn(move || run(rx, theirs, socket)) {
            lntrn_core::log_error!("the sampler could not start: {e}");
        }
        Link {
            tx: Some(tx),
            shared,
            #[cfg(test)]
            sent: Default::default(),
        }
    }

    /// No thread: `frame` is all there ever is to show.
    #[cfg(test)]
    pub fn fixed(frame: Frame) -> Link {
        let shared = Shared::default();
        *shared.frame.lock().unwrap() = Some(Arc::new(frame));
        Link { tx: None, shared: Arc::new(shared), sent: Default::default() }
    }

    /// Show `frame` from now on: a test's helper changing its mind.
    #[cfg(test)]
    pub fn show(&self, frame: Frame) {
        *self.shared.frame.lock().unwrap() = Some(Arc::new(frame));
    }

    pub fn set_waker(&self, waker: Waker) {
        let _ = self.shared.waker.set(waker);
    }

    pub fn send(&self, cmd: Cmd) {
        #[cfg(test)]
        self.sent.borrow_mut().push(match &cmd {
            Cmd::Apply(_) => "apply",
            Cmd::Keep => "keep",
            Cmd::Revert => "revert",
            Cmd::Reset => "reset",
            Cmd::Now => "now",
        }.to_owned());
        if !matches!(cmd, Cmd::Now) {
            self.shared.busy.store(true, Ordering::SeqCst);
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        } else {
            self.shared.busy.store(false, Ordering::SeqCst);
        }
    }

    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.shared.frame.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// What the thread has to say, once each.
    pub fn take_said(&self) -> Vec<String> {
        std::mem::take(&mut *self.shared.said.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::SeqCst)
    }
}

/// Ask the helper how things stand: `hello` once per connection (a
/// restarted helper may know different limits), `status` every time.
fn ask(client: &mut Client, hello: &mut Option<Arc<Hello>>) -> Helper {
    if !client.is_connected() {
        *hello = None;
    }
    if hello.is_none() {
        match client.hello() {
            Ok(h) if h.protocol == PROTOCOL => *hello = Some(Arc::new(h)),
            Ok(h) => return Helper::Away(format!("the helper is a different version (protocol {}, this window speaks {PROTOCOL}): update both", h.protocol)),
            Err(e) => return Helper::Away(e),
        }
    }
    match client.status() {
        Ok(status) => Helper::Here { hello: hello.clone().expect("asked above"), status },
        Err(e) => {
            *hello = None;
            Helper::Away(e)
        }
    }
}

/// Carry out `cmd`, and what to say about it (nothing when the bar says
/// it already).
fn carry_out(client: &mut Client, cmd: Cmd) -> Option<String> {
    let not_all = |problems: Vec<String>| (!problems.is_empty()).then(|| format!("Not all of it took: {}", problems.join("; ")));
    match cmd {
        Cmd::Apply(tune) => client.apply(&tune, true).map_or_else(|e| Some(format!("Not applied: {e}")), not_all),
        Cmd::Keep => client.keep().err().map(|e| format!("Couldn't keep it: {e}")),
        Cmd::Revert => client.revert().err().map(|e| format!("Couldn't go back: {e}")),
        Cmd::Reset => match client.reset() {
            Ok(problems) => not_all(problems).or_else(|| Some("Back to stock.".to_owned())),
            Err(e) => Some(format!("Couldn't reset: {e}")),
        },
        Cmd::Now => None,
    }
}

fn run(rx: Receiver<Cmd>, shared: Arc<Shared>, socket: PathBuf) {
    let mut sampler = Sampler::new();
    let mut client = Client::new(socket);
    let mut hello = None;
    let mut due = Instant::now() + FIRST_LOOK;
    loop {
        let cmd = match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(cmd) => Some(cmd),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let commanded = cmd.is_some();
        if let Some(said) = cmd.and_then(|c| carry_out(&mut client, c)) {
            shared.said.lock().unwrap_or_else(|e| e.into_inner()).push(said);
        }
        let began = Instant::now();
        let frame = sampler.sample(ask(&mut client, &mut hello));
        *shared.frame.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(frame));
        if commanded {
            shared.busy.store(false, Ordering::SeqCst);
        } else {
            due = began + BEAT;
        }
        if let Some(waker) = shared.waker.get() {
            waker.wake();
        }
    }
}
