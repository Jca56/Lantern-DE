//! Pairing, connecting and disconnecting, each on a connection and a
//! thread of its own: they take as long as the device does, and the
//! Bluetooth thread follows BlueZ meanwhile (answering pairing's
//! questions, as the desktop's agent). A device paired from here is
//! trusted, so it comes back on its own, and connected.

use std::sync::Arc;
use std::time::Duration;

use super::{BLUEZ, BluetoothState, DEVICE};
use crate::board::Board;
use crate::bus::{Answer, Bus, CallError};

/// How long pairing is waited on: the person may be reading a code off
/// the device. Then connecting.
const PAIRING: Duration = Duration::from_secs(90);
const CONNECTING: Duration = Duration::from_secs(30);
const DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";

/// Pair the device at `path`, trust it and connect it; say on `board`
/// how it went.
pub(super) fn pair(path: String, board: Arc<Board<BluetoothState>>) {
    board.update(|s| {
        (s.pairing, s.error, s.failed) = (Some(path.clone()), None, None);
        if !s.busy.contains(&path) {
            s.busy.push(path.clone());
        }
    });
    let mine = path.clone();
    let theirs = board.clone();
    let spawned = std::thread::Builder::new().name("bluetooth-pair".into()).spawn(move || {
        let refuse = |_: &lntrn_dbus::Message| Answer::Refuse(DENIED, "No agent here");
        let result = Bus::system().map_err(CallError::Bus).and_then(|mut b| {
            b.call_serving(BLUEZ, &path, DEVICE, "Pair", "", &[], PAIRING, refuse)?;
            // Paired: whatever comes of the rest, it stays so.
            let _ = b.set_bool(BLUEZ, &path, DEVICE, "Trusted", true);
            let _ = b.call_serving(BLUEZ, &path, DEVICE, "Connect", "", &[], CONNECTING, refuse);
            Ok(())
        });
        let error = result.err().map(|e| wording(&e, true));
        theirs.update(|s| {
            s.busy.retain(|p| *p != path);
            s.pairing = None;
            if s.question.as_ref().is_some_and(|q| q.device == path) {
                s.question = None;
            }
            s.failed = error.is_some().then(|| path.clone());
            s.error = error;
        });
    });
    if let Err(e) = spawned {
        eprintln!("[lntrn-status] bluetooth: can't pair: no thread: {e}");
        board.update(|s| (s.pairing, s.busy) = (None, s.busy.iter().filter(|p| **p != mine).cloned().collect()));
    }
}

/// Connect the device at `path`, or disconnect it (`on` false).
pub(super) fn connect(path: String, on: bool, board: Arc<Board<BluetoothState>>) {
    board.update(|s| {
        (s.error, s.failed) = (None, None);
        if !s.busy.contains(&path) {
            s.busy.push(path.clone());
        }
    });
    let (theirs, mine) = (board.clone(), path.clone());
    let spawned = std::thread::Builder::new().name("bluetooth-connect".into()).spawn(move || {
        let member = if on { "Connect" } else { "Disconnect" };
        let result = Bus::system().map_err(CallError::Bus).and_then(|mut b| b.call_serving(BLUEZ, &path, DEVICE, member, "", &[], CONNECTING, |_| Answer::Refuse(DENIED, "No agent here")));
        let error = match result {
            Ok(_) => None,
            // Still at it when the wait ran out: BlueZ's signals say how
            // it ends.
            Err(CallError::Bus(_)) => None,
            Err(e) => Some(wording(&e, false)),
        };
        theirs.update(|s| {
            s.busy.retain(|p| *p != path);
            s.failed = error.is_some().then(|| path.clone());
            s.error = error;
        });
    });
    if let Err(e) = spawned {
        eprintln!("[lntrn-status] bluetooth: can't connect: no thread: {e}");
        board.update(|s| s.busy.retain(|p| *p != mine));
    }
}

/// Why pairing (`pairing`) or connecting failed, in the popup's words:
/// by BlueZ's error's name, and what its reason says.
pub(super) fn wording(error: &CallError, pairing: bool) -> String {
    let CallError::Refused { name, why } = error else {
        return if pairing { "It didn't answer in time".to_owned() } else { error.to_string() };
    };
    let said = match name.rsplit('.').next().unwrap_or_default() {
        "AuthenticationFailed" => "The codes didn't match, or it said no",
        "AuthenticationRejected" => "Turned down",
        "AuthenticationCanceled" => "Stopped before it paired",
        "AuthenticationTimeout" => "No answer in time",
        "AlreadyExists" => "It's paired already",
        "InProgress" => "It's busy pairing already",
        "ConnectionAttemptFailed" => "It didn't answer: is it in pairing mode?",
        _ if why.contains("Page Timeout") || why.contains("Host is down") => "It didn't answer: is it on and nearby?",
        "NotReady" => "Bluetooth isn't ready yet",
        _ => return short(why),
    };
    said.to_owned()
}

/// BlueZ's reasons, made short.
pub(super) fn short(why: &str) -> String {
    why.rsplit_once(": ").map_or(why, |(_, tail)| tail).lines().next().unwrap_or(why).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluez_errors_read_as_said() {
        let refused = |name: &str, why: &str| CallError::Refused { name: name.into(), why: why.into() };
        assert_eq!(wording(&refused("org.bluez.Error.AuthenticationFailed", "Authentication Failed"), true), "The codes didn't match, or it said no");
        assert_eq!(wording(&refused("org.bluez.Error.Failed", "Page Timeout"), true), "It didn't answer: is it on and nearby?");
        assert_eq!(wording(&refused("org.bluez.Error.Failed", "br-connection-profile-unavailable"), false), "br-connection-profile-unavailable");
        assert_eq!(wording(&CallError::Bus(std::io::ErrorKind::TimedOut.into()), true), "It didn't answer in time");
        assert_eq!(short("org.bluez.Error.Blocked: Blocked through rfkill"), "Blocked through rfkill");
    }
}
