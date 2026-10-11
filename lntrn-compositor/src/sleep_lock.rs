//! Lock the screen before the machine sleeps, whoever asked for the sleep.
//!
//! logind announces every suspend or hibernate with `PrepareForSleep(true)`
//! and holds it back for as long as somebody holds a "delay" inhibitor. The
//! compositor keeps one: when sleep is announced it puts the lock screen up,
//! and lets go of the inhibitor only once the lock is confirmed (or a grace
//! period runs out, so a broken locker can't keep a closed laptop awake).
//! That covers the lid, the idle timer, the Command Center's Sleep button and
//! a `systemctl suspend` typed in a terminal alike.
//!
//! The bus is watched from its own thread; nothing here ever blocks the
//! compositor's event loop on D-Bus.

use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use lntrn_dbus::{encode_string, BodyReader, Connection};
use smithay::reexports::calloop::channel::{channel, Event, Sender};

use crate::Lantern;

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";

/// How long a sleep waits for the lock screen to come up. logind gives up on
/// a delay inhibitor by itself after 5 s (`InhibitDelayMaxSec`); staying under
/// that keeps the decision ours.
const LOCK_GRACE: Duration = Duration::from_secs(3);

/// Longest wait between attempts to reach a bus that keeps failing.
const RECONNECT_MAX: Duration = Duration::from_secs(60);

/// What the logind thread tells the event loop.
enum SleepEvent {
    /// A fresh delay inhibitor. Dropping it lets a pending sleep go ahead.
    Inhibitor(OwnedFd),
    /// `PrepareForSleep`: true going down, false on the way back up.
    PrepareForSleep(bool),
}

/// The sleep inhibitor, and a sleep that is waiting on the lock screen.
#[derive(Default)]
pub struct SleepLock {
    inhibitor: Option<OwnedFd>,
    /// Set while a sleep waits for the lock: when to stop waiting.
    waiting_until: Option<Instant>,
}

impl SleepLock {
    /// The session lock was just confirmed: nothing is visible any more, so a
    /// sleep that was waiting for that can go ahead.
    pub fn lock_confirmed(&mut self) {
        if self.waiting_until.take().is_some() {
            self.release("the lock screen is up");
        }
    }

    /// Called once a second: give up on a lock screen that never came.
    pub fn tick(&mut self) {
        if self.waiting_until.is_some_and(|deadline| Instant::now() >= deadline) {
            self.waiting_until = None;
            tracing::warn!("sleep: the lock screen didn't come up in time");
            self.release("gave up waiting for the lock screen");
        }
    }

    fn wait_for_lock(&mut self) {
        self.waiting_until = Some(Instant::now() + LOCK_GRACE);
    }

    fn release(&mut self, why: &str) {
        if self.inhibitor.take().is_some() {
            tracing::info!("sleep: letting the machine sleep ({why})");
        }
    }
}

impl Lantern {
    /// logind is about to put the machine to sleep and is waiting on us.
    fn prepare_for_sleep(&mut self) {
        match &self.session_lock {
            // Locked and confirmed already: nothing to wait for.
            Some(lock) if lock.pending_locker.is_none() => self.sleep_lock.release("already locked"),
            // A locker is on its way up; its confirmation releases the sleep.
            Some(_) => self.sleep_lock.wait_for_lock(),
            None => {
                crate::power::spawn_lockscreen();
                self.sleep_lock.wait_for_lock();
            }
        }
    }
}

/// Start watching logind and hook its events into the event loop. Call once
/// after `Lantern::new`.
pub fn install(state: &mut Lantern) {
    let (tx, rx) = channel();
    let res = state
        .loop_handle
        .insert_source(rx, |event, _, state: &mut Lantern| {
            let Event::Msg(event) = event else {
                return;
            };
            match event {
                SleepEvent::Inhibitor(fd) => state.sleep_lock.inhibitor = Some(fd),
                SleepEvent::PrepareForSleep(true) => {
                    tracing::info!("sleep: the machine is about to sleep, locking first");
                    state.prepare_for_sleep();
                }
                // Awake again; the thread is already fetching a new inhibitor.
                SleepEvent::PrepareForSleep(false) => state.sleep_lock.waiting_until = None,
            }
        });
    if let Err(e) = res {
        tracing::warn!(?e, "failed to install the logind sleep channel");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("logind-sleep".into())
        .spawn(move || watch_logind(tx));
    if let Err(e) = spawned {
        tracing::warn!(?e, "failed to start the logind sleep watcher");
    }
}

/// Thread body: stay connected to logind for as long as the compositor runs.
fn watch_logind(tx: Sender<SleepEvent>) {
    let mut wait = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let err = match listen(&tx) {
            Ok(()) => return, // the event loop is gone
            Err(err) => err,
        };
        // A connection that held for a while was a healthy one.
        if started.elapsed() >= RECONNECT_MAX {
            wait = Duration::from_secs(1);
        }
        tracing::warn!("sleep: no logind connection ({err}); the screen won't lock before sleep until it's back, retrying in {wait:?}");
        std::thread::sleep(wait);
        wait = (wait * 2).min(RECONNECT_MAX);
    }
}

/// One connection's worth of listening. `Ok` when the compositor's event loop
/// went away, `Err` when the bus did.
fn listen(tx: &Sender<SleepEvent>) -> std::io::Result<()> {
    let mut conn = Connection::connect_system_with_fds()?;
    conn.add_match(&format!(
        "type='signal',sender='{LOGIND}',path='{LOGIND_PATH}',interface='{LOGIND_MANAGER}',member='PrepareForSleep'"
    ));
    let mut inhibit_call = Some(request_inhibitor(&mut conn));

    loop {
        let msg = conn.read_next()?;
        // Only a broadcast counts: those reach us through the match rule
        // above, which the bus honours for logind alone. A signal addressed
        // straight to us could be from anyone.
        let from_logind = msg.is_signal() && msg.destination.is_empty();
        if from_logind && msg.interface == LOGIND_MANAGER && msg.member == "PrepareForSleep" {
            let going_down = BodyReader::new(&msg.body, &msg.signature).read_bool();
            if tx.send(SleepEvent::PrepareForSleep(going_down)).is_err() {
                return Ok(());
            }
            if !going_down {
                // The inhibitor was spent letting the machine sleep. Take
                // another for next time.
                inhibit_call = Some(request_inhibitor(&mut conn));
            }
        } else if !msg.is_signal() && inhibit_call == Some(msg.reply_serial) {
            inhibit_call = None;
            let refused = msg.is_error();
            match msg.fds.into_iter().next() {
                Some(fd) if !refused => {
                    if tx.send(SleepEvent::Inhibitor(fd)).is_err() {
                        return Ok(());
                    }
                }
                _ => tracing::warn!("sleep: logind refused a sleep inhibitor; the lock may lose the race with sleep"),
            }
        }
    }
}

/// Ask logind for a delay inhibitor on sleep. The reply carries it as a file
/// descriptor. Returns the call's serial to match that reply by.
fn request_inhibitor(conn: &mut Connection) -> u32 {
    let mut body = Vec::new();
    for arg in ["sleep", "Lantern", "Lock the screen before sleep", "delay"] {
        encode_string(&mut body, arg);
    }
    conn.method_call(LOGIND, LOGIND_PATH, LOGIND_MANAGER, "Inhibit", "ssss", &body)
}
