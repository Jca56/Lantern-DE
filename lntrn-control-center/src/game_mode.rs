//! Game Mode: the compositor's (the primary monitor at its native scale,
//! so a full-screen game gets every pixel), read from its gaming socket
//! on a thread of its own (`gaming:<0|1>`, on connecting and on every
//! change, Super+G's too) and flipped with `toggle`, as the Command
//! Center's tile does.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::log_warn;

unsafe extern "C" {
    fn getuid() -> u32;
}

/// Between attempts to reach the socket.
const RETRY: Duration = Duration::from_secs(2);
const WRITE_TIMEOUT: Duration = Duration::from_millis(100);
/// What `state` holds: not heard yet, off, on.
const UNKNOWN: u8 = 0;
const OFF: u8 = 1;
const ON: u8 = 2;

fn socket_path() -> PathBuf {
    // SAFETY: a plain syscall that can't fail.
    PathBuf::from(format!("/run/user/{}/lntrn-gaming.sock", unsafe { getuid() }))
}

#[derive(Clone, Default)]
pub struct GameMode {
    state: Arc<AtomicU8>,
    writer: Arc<Mutex<Option<UnixStream>>>,
}

impl GameMode {
    /// Follow the compositor's word from a thread of its own, waking the
    /// loop when it changes.
    pub fn start(&self, waker: Waker) {
        let (state, writer) = (self.state.clone(), self.writer.clone());
        let spawned = std::thread::Builder::new().name("game-mode".into()).spawn(move || {
            loop {
                if let Ok(stream) = UnixStream::connect(socket_path()) {
                    *writer.lock().unwrap_or_else(PoisonError::into_inner) = stream.try_clone().ok().filter(|w| w.set_write_timeout(Some(WRITE_TIMEOUT)).is_ok());
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { break };
                        let now = match line.trim() {
                            "gaming:1" => ON,
                            "gaming:0" => OFF,
                            _ => continue,
                        };
                        if state.swap(now, Ordering::AcqRel) != now {
                            waker.wake();
                        }
                    }
                    *writer.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    state.store(UNKNOWN, Ordering::Release);
                    waker.wake();
                }
                std::thread::sleep(RETRY);
            }
        });
        if let Err(e) = spawned {
            log_warn!("game mode: no thread: {e}; its button stays dim");
        }
    }

    /// On or off, once the compositor has said.
    pub fn on(&self) -> Option<bool> {
        match self.state.load(Ordering::Acquire) {
            ON => Some(true),
            OFF => Some(false),
            _ => None,
        }
    }

    /// Ask the compositor to flip it; it says back what it became.
    pub fn toggle(&self) {
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(w) = writer.as_mut()
            && let Err(e) = w.write_all(b"toggle\n")
        {
            log_warn!("game mode: can't ask the compositor: {e}");
        }
    }
}
