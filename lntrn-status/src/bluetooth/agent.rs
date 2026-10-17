//! The pairing agent: BlueZ's `org.bluez.Agent1`, on the Bluetooth
//! thread's own connection and the desktop's default for as long as it
//! runs, so it hears pairing started from here and from the other end.
//! BlueZ asks it what pairing takes: whether a code both ends show
//! matches, a PIN or passkey to type here; or it says a code to type on
//! the device. A question waits for the person ([`Question`] on the
//! board) and is answered when they do; BlueZ gives up on its own after a
//! while, and says so (`Cancel`). Pairing started from here needs no "may
//! it?", started from the other end it does. Only BlueZ is answered:
//! anyone else on the bus is turned down.

use lntrn_dbus::{BodyReader, Message, Value, encode_string, encode_u32};

use super::{Asked, BLUEZ, BluetoothState, Question, Reply, read};
use crate::bus::{Answer, Bus, CallError};

pub(super) const AGENT_PATH: &str = "/org/lantern/status/bt_agent";
const AGENT: &str = "org.bluez.Agent1";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
const REJECTED: &str = "org.bluez.Error.Rejected";
const CANCELED: &str = "org.bluez.Error.Canceled";
const DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
const UNKNOWN: &str = "org.freedesktop.DBus.Error.UnknownMethod";

/// A question waiting on the person: whose call to answer, and how.
struct Waiting {
    id: u64,
    serial: u32,
    sender: String,
    kind: Asked,
}

/// What taking one of BlueZ's calls comes to.
#[derive(Default)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(super) struct Taken {
    /// Said to BlueZ at once; none while the person decides.
    pub now: Option<Answer>,
    /// What the board's question becomes, when it changes.
    pub question: Option<Option<Question>>,
    /// A question still waiting that this one takes the place of: turned
    /// down, by its call's serial and sender.
    pub stale: Option<(u32, String)>,
}

#[derive(Default)]
pub(super) struct Agent {
    /// BlueZ's unique name: the one caller answered.
    bluez: Option<String>,
    waiting: Option<Waiting>,
    next: u64,
}

impl Agent {
    /// Be the desktop's agent on `bus`; again each time BlueZ starts over.
    pub(super) fn register(&mut self, bus: &mut Bus) -> Result<(), CallError> {
        (self.bluez, self.waiting) = (Some(bus.owner(BLUEZ)?), None);
        let mut body = Vec::new();
        encode_string(&mut body, AGENT_PATH);
        encode_string(&mut body, "KeyboardDisplay");
        match bus.call(BLUEZ, "/org/bluez", AGENT_MANAGER, "RegisterAgent", "os", &body) {
            Err(CallError::Refused { name, .. }) if name.ends_with("AlreadyExists") => {}
            other => other.map(|_| ())?,
        }
        let mut path = Vec::new();
        encode_string(&mut path, AGENT_PATH);
        bus.call(BLUEZ, "/org/bluez", AGENT_MANAGER, "RequestDefaultAgent", "o", &path).map(|_| ())
    }

    /// BlueZ is gone: nothing it asked waits any more.
    pub(super) fn forget(&mut self) {
        (self.bluez, self.waiting) = (None, None);
    }

    /// Take the call `msg` made to us, `state` saying what is paired and
    /// what is being paired from here.
    pub(super) fn take(&mut self, msg: &Message, state: &BluetoothState) -> Taken {
        if self.bluez.as_deref() != Some(msg.sender.as_str()) || msg.path != AGENT_PATH || msg.interface != AGENT {
            return Taken { now: Some(Answer::Refuse(DENIED, "Not BlueZ's agent")), ..Taken::default() };
        }
        let args = BodyReader::new(&msg.body, &msg.signature).read_all();
        let device = args.first().and_then(Value::as_str).unwrap_or_default().to_owned();
        let done = |now| Taken { now: Some(now), ..Taken::default() };
        let ok = || Answer::Reply("", Vec::new());
        let kind = match msg.member.as_str() {
            "Release" | "Cancel" => {
                self.waiting = None;
                return Taken { now: Some(ok()), question: Some(None), stale: None };
            }
            "AuthorizeService" if state.devices.iter().any(|d| d.path == device) => return done(ok()),
            "AuthorizeService" => return done(Answer::Refuse(REJECTED, "Not paired")),
            "RequestAuthorization" if state.pairing.as_deref() == Some(device.as_str()) => return done(ok()),
            "RequestAuthorization" => Asked::Allow,
            "RequestConfirmation" => Asked::Confirm(args.get(1).and_then(Value::as_u32).unwrap_or_default()),
            "RequestPinCode" => Asked::Pin,
            "RequestPasskey" => Asked::Passkey,
            // Said, not asked: the code shows until pairing ends.
            "DisplayPasskey" => {
                let code = format!("{:06}", args.get(1).and_then(Value::as_u32).unwrap_or_default());
                let typed = args.get(2).and_then(Value::as_u32).unwrap_or_default();
                return Taken { now: Some(ok()), question: Some(Some(self.question(&device, state, Asked::Show(code, typed)))), stale: None };
            }
            "DisplayPinCode" => {
                let code = args.get(1).and_then(Value::as_str).unwrap_or_default().to_owned();
                return Taken { now: Some(ok()), question: Some(Some(self.question(&device, state, Asked::Show(code, 0)))), stale: None };
            }
            _ => return done(Answer::Refuse(UNKNOWN, "Not a question this agent takes")),
        };
        let question = self.question(&device, state, kind.clone());
        let stale = self.waiting.replace(Waiting { id: question.id, serial: msg.serial, sender: msg.sender.clone(), kind }).map(|w| (w.serial, w.sender));
        Taken { now: None, question: Some(Some(question)), stale }
    }

    fn question(&mut self, device: &str, state: &BluetoothState, kind: Asked) -> Question {
        self.next += 1;
        Question { id: self.next, device: device.to_owned(), name: read::name_of(state, device), kind }
    }

    /// The person's `reply` to the question `id`: whose call to answer
    /// (serial, sender), and the answer. `None` when it waits no more.
    pub(super) fn answer(&mut self, id: u64, reply: Reply) -> Option<(u32, String, Answer)> {
        let w = self.waiting.take_if(|w| w.id == id)?;
        let answer = match (&w.kind, reply) {
            (Asked::Pin, Reply::Pin(pin)) => {
                let mut body = Vec::new();
                encode_string(&mut body, &pin);
                Answer::Reply("s", body)
            }
            (Asked::Passkey, Reply::Passkey(key)) => {
                let mut body = Vec::new();
                encode_u32(&mut body, key);
                Answer::Reply("u", body)
            }
            (Asked::Confirm(_) | Asked::Allow, Reply::Yes) => Answer::Reply("", Vec::new()),
            (_, Reply::No) => Answer::Refuse(REJECTED, "Turned down"),
            _ => Answer::Refuse(CANCELED, "Not what was asked"),
        };
        Some((w.serial, w.sender, answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bluetooth::Device;
    use lntrn_dbus::encode_u32;

    const PHONE: &str = "/org/bluez/hci0/dev_12_34_56_78_9A_BC";

    fn call(sender: &str, member: &str, sig: &str, body: Vec<u8>) -> Message {
        Message {
            msg_type: 1,
            serial: 40,
            reply_serial: 0,
            sender: sender.into(),
            path: AGENT_PATH.into(),
            member: member.into(),
            interface: AGENT.into(),
            signature: sig.into(),
            destination: ":1.9".into(),
            error_name: String::new(),
            body,
            fds: Vec::new(),
        }
    }

    fn on_phone(code: Option<u32>) -> Vec<u8> {
        let mut body = Vec::new();
        encode_string(&mut body, PHONE);
        if let Some(code) = code {
            lntrn_dbus::align_to(&mut body, 4);
            encode_u32(&mut body, code);
        }
        body
    }

    fn agent() -> Agent {
        Agent { bluez: Some(":1.2".into()), ..Agent::default() }
    }

    #[test]
    fn a_code_to_confirm_waits_for_the_person_and_their_yes_goes_back() {
        let (mut a, state) = (agent(), BluetoothState::default());
        let taken = a.take(&call(":1.2", "RequestConfirmation", "ou", on_phone(Some(482_913))), &state);
        let Some(Some(q)) = taken.question else { panic!("asks: {taken:?}") };
        assert_eq!((taken.now, q.kind.clone(), q.name.as_str()), (None, Asked::Confirm(482_913), "12:34:56:78:9A:BC"));
        assert!(a.answer(q.id + 1, Reply::Yes).is_none(), "another question's answer");
        assert_eq!(a.answer(q.id, Reply::Yes), Some((40, ":1.2".to_owned(), Answer::Reply("", Vec::new()))));
        assert!(a.answer(q.id, Reply::Yes).is_none(), "answered once");
    }

    /// Against the real BlueZ: our agent is taken as the desktop's, and
    /// let go. Pairs nothing.
    #[test]
    #[ignore = "talks to the system bus and BlueZ"]
    fn bluez_takes_our_agent_and_lets_it_go() {
        let mut bus = Bus::system().unwrap();
        agent().register(&mut bus).unwrap();
        let mut path = Vec::new();
        encode_string(&mut path, AGENT_PATH);
        bus.call(BLUEZ, "/org/bluez", AGENT_MANAGER, "UnregisterAgent", "o", &path).unwrap();
    }

    #[test]
    fn a_pin_or_passkey_typed_here_goes_back_as_asked() {
        let (mut a, state) = (agent(), BluetoothState::default());
        let Some(Some(q)) = a.take(&call(":1.2", "RequestPinCode", "o", on_phone(None)), &state).question else { panic!() };
        let Some((_, _, Answer::Reply("s", body))) = a.answer(q.id, Reply::Pin("0000".into())) else { panic!() };
        assert_eq!(BodyReader::new(&body, "s").read_all()[0].as_str(), Some("0000"));
        let Some(Some(q)) = a.take(&call(":1.2", "RequestPasskey", "o", on_phone(None)), &state).question else { panic!() };
        let Some((_, _, Answer::Reply("u", body))) = a.answer(q.id, Reply::Passkey(123_456)) else { panic!() };
        assert_eq!(BodyReader::new(&body, "u").read_all()[0].as_u32(), Some(123_456));
        let Some(Some(q)) = a.take(&call(":1.2", "RequestPinCode", "o", on_phone(None)), &state).question else { panic!() };
        assert!(matches!(a.answer(q.id, Reply::No), Some((_, _, Answer::Refuse(REJECTED, _)))));
    }

    #[test]
    fn only_bluez_is_answered_and_a_code_to_type_shows_at_once() {
        let (mut a, mut state) = (agent(), BluetoothState::default());
        let stranger = a.take(&call(":1.66", "RequestPinCode", "o", on_phone(None)), &state);
        assert!(matches!(stranger.now, Some(Answer::Refuse(DENIED, _))) && stranger.question.is_none());
        let shown = a.take(&call(":1.2", "DisplayPasskey", "ouq", on_phone(Some(7))), &state);
        assert!(matches!(shown.now, Some(Answer::Reply("", _))));
        assert!(matches!(shown.question, Some(Some(Question { kind: Asked::Show(ref code, 0), .. })) if code == "000007"));
        // Pairing it from here: no "may it?". From its end: asked.
        state.pairing = Some(PHONE.into());
        assert_eq!(a.take(&call(":1.2", "RequestAuthorization", "o", on_phone(None)), &state).now, Some(Answer::Reply("", Vec::new())));
        state.pairing = None;
        assert!(matches!(a.take(&call(":1.2", "RequestAuthorization", "o", on_phone(None)), &state).question, Some(Some(Question { kind: Asked::Allow, .. }))));
        // A service of a paired device's goes; anyone else's doesn't.
        let service = |a: &mut Agent, state: &BluetoothState| a.take(&call(":1.2", "AuthorizeService", "os", on_phone(None)), state).now;
        assert!(matches!(service(&mut a, &state), Some(Answer::Refuse(REJECTED, _))));
        state.devices.push(Device { path: PHONE.into(), ..Device::default() });
        assert_eq!(service(&mut a, &state), Some(Answer::Reply("", Vec::new())));
        // BlueZ giving up clears the question.
        assert_eq!(a.take(&call(":1.2", "Cancel", "", Vec::new()), &state).question, Some(None));
    }
}
