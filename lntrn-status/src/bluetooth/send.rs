//! Sending a file over Bluetooth: a session with obexd per file, on a
//! connection and a thread of its own, which obexd ties the transfer to
//! (it ends with the connection, and only it may stop the transfer). Its
//! progress comes from the transfer's own properties; a notification says
//! how it went.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lntrn_dbus::{Value, align_to, encode_dict_entry_sv, encode_string, encode_u32, encode_variant_string};

use super::files::{DENIED, NEXT, OBEX, POLL, ROOT, TRANSFER, end, number, properties, tell, text, update};
use super::{BluetoothState, Ended, Transfer};
use crate::board::Board;
use crate::bus::{Answer, Bus, CallError};
use crate::sys;

const CLIENT: &str = "org.bluez.obex.Client1";
const PUSH: &str = "org.bluez.obex.ObjectPush1";
/// How long a send waits on a device doing nothing (its person never
/// saying yes).
const IDLE: Duration = Duration::from_secs(120);
/// Connecting to a device to send it something.
const CONNECTING: Duration = Duration::from_secs(60);
/// What a transfer stopped from here fails with.
const STOPPED: &str = "stopped here";

/// Sends asked to stop, by their transfer's path (or the placeholder it
/// had until obexd named it).
pub(super) type Stops = Arc<Mutex<HashSet<String>>>;

/// Send `file` to the device at `address`, from a thread of its own; the
/// board follows it as it goes, and `stops` can stop it.
pub(super) fn send(address: String, file: PathBuf, board: Arc<Board<BluetoothState>>, stops: Stops) {
    let name = file.file_name().map_or_else(|| "File".to_owned(), |n| n.to_string_lossy().into_owned());
    let size = std::fs::metadata(&file).map_or(0, |m| m.len());
    let placeholder = format!("sending:{}", NEXT.fetch_add(1, Ordering::Relaxed));
    board.update(|s| s.transfers.push(Transfer { path: placeholder.clone(), device: address.clone(), name, size, done: 0, incoming: false, end: None }));
    let theirs = board.clone();
    let spawned = std::thread::Builder::new().name("bluetooth-send".into()).spawn(move || {
        let mut path = placeholder.clone();
        let stopped = |path: &str| stops.lock().is_ok_and(|s| s.contains(path) || s.contains(&placeholder));
        let result = Bus::session().map_err(CallError::Bus).and_then(|mut bus| {
            bus.listen(OBEX);
            let session = bus.call_serving(OBEX, ROOT, CLIENT, "CreateSession", "sa{sv}", &session_args(&address), CONNECTING, |_| Answer::Refuse(DENIED, "No agent here"))?;
            let session = session.first().and_then(Value::as_str).unwrap_or_default().to_owned();
            let mut body = Vec::new();
            encode_string(&mut body, &file.to_string_lossy());
            // Stopped while it was connecting: nothing goes.
            let sent = if stopped(&path) { Err(stop()) } else { bus.call(OBEX, &session, PUSH, "SendFile", "s", &body) }.and_then(|reply| {
                let transfer = reply.first().and_then(Value::as_str).unwrap_or_default().to_owned();
                update(&theirs, &path, |t| t.path = transfer.clone());
                path = transfer;
                watch(&mut bus, &path, &theirs, &stopped)
            });
            let mut session_path = Vec::new();
            encode_string(&mut session_path, &session);
            let _ = bus.call(OBEX, ROOT, CLIENT, "RemoveSession", "o", &session_path);
            sent
        });
        let to = theirs.get().devices.iter().find(|d| d.address.eq_ignore_ascii_case(&address)).map_or_else(|| address.clone(), |d| d.name.clone());
        let name = theirs.get().transfers.iter().find(|t| t.path == path).map(|t| t.name.clone()).unwrap_or_default();
        if let Ok(mut s) = stops.lock() {
            s.remove(&path);
            s.remove(&placeholder);
        }
        match result {
            Ok(()) => {
                end(&theirs, &path, Ended::Sent);
                tell(&format!("Sent {name}"), &format!("To {to}"));
            }
            Err(CallError::Refused { name, .. }) if name == STOPPED => end(&theirs, &path, Ended::Stopped),
            Err(e) => {
                let why = sending_wording(&e);
                tell(&format!("Couldn't send {name}"), &format!("To {to}: {why}"));
                end(&theirs, &path, Ended::Failed(why));
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("[lntrn-status] bluetooth: can't send: no thread: {e}");
    }
}

/// What a transfer stopped from here fails with.
fn stop() -> CallError {
    CallError::Refused { name: STOPPED.into(), why: "Stopped".into() }
}

/// Follow the transfer at `path` on `bus` until it ends: how far it got,
/// from its signals and from reading it every so often; until `stopped`
/// says it is to stop.
fn watch(bus: &mut Bus, path: &str, board: &Board<BluetoothState>, stopped: &dyn Fn(&str) -> bool) -> Result<(), CallError> {
    let (mut moved, mut last) = (Instant::now(), 0);
    loop {
        if stopped(path) {
            let _ = bus.call(OBEX, path, TRANSFER, "Cancel", "", &[]);
            return Err(stop());
        }
        if sys::wait(&[bus.fd()], Some(POLL))[0] {
            // Its signals only wake us: the reading below says it all.
            bus.drain().map_err(CallError::Bus)?;
        }
        let props = properties(bus, path, TRANSFER)?;
        let done = number(&props, "Transferred");
        update(board, path, |t| t.done = done);
        match text(&props, "Status").as_deref() {
            Some("complete") => return Ok(()),
            Some("error") => return Err(CallError::Refused { name: String::new(), why: "It stopped going".into() }),
            _ => {}
        }
        if done != last {
            (moved, last) = (Instant::now(), done);
        } else if moved.elapsed() > IDLE {
            let _ = bus.call(OBEX, path, TRANSFER, "Cancel", "", &[]);
            return Err(CallError::Refused { name: String::new(), why: "It never took it".into() });
        }
    }
}

/// Why a send failed, as said.
fn sending_wording(e: &CallError) -> String {
    let why = e.to_string();
    match () {
        _ if why.contains("Unable to find service record") => "It doesn't take files".to_owned(),
        _ if why.contains("Host is down") || why.contains("Connection refused") || why.contains("Timed out") || why.contains("timed out") => "It didn't answer: is it on and nearby?".to_owned(),
        _ => why,
    }
}

/// `CreateSession`'s arguments: the device, and that it's a push.
fn session_args(address: &str) -> Vec<u8> {
    let mut body = Vec::new();
    encode_string(&mut body, address);
    let mut entries = Vec::new();
    encode_dict_entry_sv(&mut entries, "Target", |b| encode_variant_string(b, "opp"));
    align_to(&mut body, 4);
    encode_u32(&mut body, entries.len() as u32);
    align_to(&mut body, 8);
    body.extend_from_slice(&entries);
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_dbus::BodyReader;

    #[test]
    fn a_push_is_asked_for_by_address_and_kind() {
        let body = session_args("AA:BB:CC:DD:EE:FF");
        let values = BodyReader::new(&body, "sa{sv}").read_all();
        assert_eq!(values[0].as_str(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(values[1].as_dict().and_then(|d| d.get("Target")).and_then(Value::as_str), Some("opp"));
    }
}
