//! Joining a network, on a connection and a thread of its own: connecting
//! takes as long as it takes, and the main thread follows iwd meanwhile.
//!
//! With a password, that connection is iwd's agent while it waits. iwd
//! asks the agent of whoever asked it to connect for the password
//! (`RequestPassphrase`), is told it, and the agent goes with the join.
//! Only iwd is told: a question from anyone else on the bus, or about
//! another network, is turned down.

use std::sync::Arc;
use std::time::Duration;

use lntrn_dbus::{BodyReader, Message, Value, encode_string};

use super::{IWD, NETWORK, STATION, Target, WifiState, read};
use crate::board::Board;
use crate::bus::{Answer, Bus, CallError};
use crate::secret::Secret;

const AGENT_MANAGER: &str = "net.connman.iwd.AgentManager";
const AGENT: &str = "net.connman.iwd.Agent";
/// Where iwd finds our agent, on the join's connection.
const AGENT_PATH: &str = "/org/lantern/status/wifi_agent";
/// What an agent says to a question it won't answer, and to a stranger.
const CANCELED: &str = "net.connman.iwd.Agent.Error.Canceled";
const DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
/// How long a join is waited on: past this, iwd's own signals say how it
/// ends.
const PATIENCE: Duration = Duration::from_secs(60);

/// Whether `password` can be a WiFi network's (WPA): 8 to 63 printable
/// ASCII characters, or the key itself as 64 hex digits.
pub fn fits(password: &str) -> bool {
    let words = (8..=63).contains(&password.len()) && password.bytes().all(|b| (0x20..=0x7e).contains(&b));
    words || (password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Join `target` from a thread of its own, telling iwd `secret` when it
/// asks for it; then say on `board` how it went.
pub(super) fn start(target: Target, secret: Option<Secret>, board: Arc<Board<WifiState>>) {
    let spawned = std::thread::Builder::new().name("wifi-join".into()).spawn(move || {
        let result = Bus::system().map_err(CallError::Bus).and_then(|mut bus| connect(&mut bus, &target, secret.as_ref()));
        let error = match result {
            Ok(()) => None,
            // Still connecting when the wait ran out: iwd's own signals
            // say how it ends.
            Err(CallError::Bus(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => None,
            Err(e) => Some(wording(&e, secret.is_some(), &target)),
        };
        drop(secret);
        board.update(|s| {
            if s.joining.as_ref() == Some(&target) {
                s.joining = None;
                s.failed = error.is_some().then(|| target.clone());
                s.error = error;
            }
        });
    });
    if let Err(e) = spawned {
        eprintln!("[lntrn-status] wifi: can't join: no thread: {e}");
    }
}

/// Ask iwd to join `target` and wait for it, as its agent meanwhile when
/// there is a `secret` to tell it.
fn connect(bus: &mut Bus, target: &Target, secret: Option<&Secret>) -> Result<(), CallError> {
    let (path, iface, member, sig, body) = match target {
        Target::Network(path) => (path.clone(), NETWORK, "Connect", "", Vec::new()),
        Target::Hidden(name) => {
            let objects = bus.objects(IWD)?;
            let station = read::device_path(&objects).filter(|p| objects[p].contains_key(STATION));
            let station = station.ok_or_else(|| CallError::Refused { name: String::new(), why: "WiFi is off".into() })?;
            let mut body = Vec::new();
            encode_string(&mut body, name);
            (station, STATION, "ConnectHiddenNetwork", "s", body)
        }
    };
    let Some(secret) = secret else {
        return bus.call_serving(IWD, &path, iface, member, sig, &body, PATIENCE, |_| Answer::Refuse(DENIED, "No agent here")).map(|_| ());
    };
    let iwd = bus.owner(IWD)?;
    let mut agent = Vec::new();
    encode_string(&mut agent, AGENT_PATH);
    bus.call(IWD, "/net/connman/iwd", AGENT_MANAGER, "RegisterAgent", "o", &agent)?;
    let network = match target {
        Target::Network(path) => Some(path.as_str()),
        // Its path isn't known until iwd finds it.
        Target::Hidden(_) => None,
    };
    let result = bus.call_serving(IWD, &path, iface, member, sig, &body, PATIENCE, |msg| answer(msg, &iwd, network, secret));
    // It goes with the connection anyway; said, so iwd needn't notice.
    let _ = bus.call(IWD, "/net/connman/iwd", AGENT_MANAGER, "UnregisterAgent", "o", &agent);
    result.map(|_| ())
}

/// What our agent says to `msg`: `secret`, when iwd (`iwd`, its unique
/// name) asks for the password of `network` (of any, joining a hidden
/// one); nothing to its goodbyes; no to anything else, or anyone else.
fn answer(msg: &Message, iwd: &str, network: Option<&str>, secret: &Secret) -> Answer {
    if msg.sender != iwd || msg.path != AGENT_PATH || msg.interface != AGENT {
        return Answer::Refuse(DENIED, "Not iwd's agent");
    }
    match msg.member.as_str() {
        "RequestPassphrase" => {
            let asked = BodyReader::new(&msg.body, &msg.signature).read_all();
            if network.is_some_and(|ours| asked.first().and_then(Value::as_str) != Some(ours)) {
                return Answer::Refuse(CANCELED, "Not the network being joined");
            }
            let mut body = Vec::new();
            encode_string(&mut body, secret.as_str());
            Answer::Reply("s", body)
        }
        "Release" | "Cancel" => Answer::Reply("", Vec::new()),
        _ => Answer::Refuse(CANCELED, "Only a password is told here"),
    }
}

/// Why a join failed, as the popup says it: by iwd's error's name.
fn wording(error: &CallError, password: bool, target: &Target) -> String {
    let CallError::Refused { name, why } = error else { return error.to_string() };
    let hidden = matches!(target, Target::Hidden(_));
    let said = match name.rsplit('.').next().unwrap_or_default() {
        "Failed" if password => "That password didn't work",
        "InvalidFormat" => "A WiFi password is 8 to 63 characters",
        "NoAgent" => "It needs a password",
        "NotFound" if hidden => "No hidden network by that name nearby",
        "NotHidden" => "That one isn't hidden: pick it from the list",
        "AlreadyProvisioned" => "That one is saved: it joins by itself",
        "ServiceSetOverlap" => "More than one network goes by that name",
        "InProgress" | "Busy" => "WiFi is busy: try again in a moment",
        "Aborted" => "Stopped before it joined",
        "NotSupported" => "This kind of network can't be joined from here",
        _ => return short(why),
    };
    said.to_owned()
}

/// iwd's reasons, made short: `net.connman.iwd.Failed: Operation failed`
/// reads as "Operation failed".
fn short(why: &str) -> String {
    why.rsplit_once(": ").map_or(why, |(_, tail)| tail).lines().next().unwrap_or(why).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(sender: &str, member: &str, network: &str) -> Message {
        let mut body = Vec::new();
        encode_string(&mut body, network);
        Message {
            msg_type: 1,
            serial: 7,
            reply_serial: 0,
            sender: sender.into(),
            path: AGENT_PATH.into(),
            member: member.into(),
            interface: AGENT.into(),
            signature: "o".into(),
            destination: ":1.40".into(),
            error_name: String::new(),
            body,
            fds: Vec::new(),
        }
    }

    #[test]
    fn the_password_goes_to_iwd_alone_and_only_for_the_network_joined() {
        let secret = Secret::new("correct horse".into());
        let net = Some("/net/connman/iwd/0/4/43616665_psk");
        let told = |msg: &Message, network| match answer(msg, ":1.3", network, &secret) {
            Answer::Reply("s", body) => BodyReader::new(&body, "s").read_all()[0].as_str().map(str::to_owned),
            _ => None,
        };
        assert_eq!(told(&asked(":1.3", "RequestPassphrase", net.unwrap()), net).as_deref(), Some("correct horse"));
        assert_eq!(told(&asked(":1.99", "RequestPassphrase", net.unwrap()), net), None, "someone else on the bus asking");
        assert_eq!(told(&asked(":1.3", "RequestPassphrase", "/net/connman/iwd/0/4/6f74686572_psk"), net), None, "another network");
        assert_eq!(told(&asked(":1.3", "RequestPassphrase", "/net/connman/iwd/0/4/68696464656e_psk"), None).as_deref(), Some("correct horse"), "a hidden one's, found");
        assert_eq!(told(&asked(":1.3", "RequestUserNameAndPassword", net.unwrap()), net), None, "a company login");
        assert!(matches!(answer(&asked(":1.3", "Release", ""), ":1.3", net, &secret), Answer::Reply("", _)));
    }

    /// Against the real iwd: our connection is let be its agent, and
    /// stops being one. Joins nothing.
    #[test]
    #[ignore = "talks to the system bus and iwd"]
    fn iwd_takes_our_agent_and_lets_it_go() {
        let mut bus = Bus::system().unwrap();
        assert!(bus.owner(IWD).unwrap().starts_with(':'));
        let mut agent = Vec::new();
        encode_string(&mut agent, AGENT_PATH);
        bus.call(IWD, "/net/connman/iwd", AGENT_MANAGER, "RegisterAgent", "o", &agent).unwrap();
        bus.call(IWD, "/net/connman/iwd", AGENT_MANAGER, "UnregisterAgent", "o", &agent).unwrap();
    }

    #[test]
    fn a_password_fits_a_wifi_network_or_doesnt() {
        assert!(fits("12345678") && fits(&"x".repeat(63)) && fits(&"ab".repeat(32)));
        assert!(!fits("1234567") && !fits(&"x".repeat(64)) && !fits("pässword"));
    }

    #[test]
    fn iwds_errors_read_as_said() {
        let refused = |name: &str, why: &str| CallError::Refused { name: name.into(), why: why.into() };
        let net = Target::Network("/n".into());
        let hidden = Target::Hidden("Attic".into());
        assert_eq!(wording(&refused("net.connman.iwd.Failed", "Operation failed"), true, &net), "That password didn't work");
        assert_eq!(wording(&refused("net.connman.iwd.Failed", "Operation failed"), false, &net), "Operation failed");
        assert_eq!(wording(&refused("net.connman.iwd.NotFound", "Object not found"), true, &hidden), "No hidden network by that name nearby");
        assert_eq!(wording(&refused("net.connman.iwd.NoAgent", "No Agent registered"), false, &hidden), "It needs a password");
        assert_eq!(short("net.connman.iwd.Failed: Operation failed"), "Operation failed");
        assert_eq!(short("No agent"), "No agent");
    }
}
