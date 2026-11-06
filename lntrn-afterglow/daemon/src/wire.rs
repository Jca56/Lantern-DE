//! The protocol between the window and the helper: one JSON object a
//! line, each way. A request names its `op`; a reply says `"ok": true`
//! and what was asked for, or `"ok": false` and an `error`.
//!
//! - `hello` — the helper's protocol, whether it is root, and this
//!   machine's [`Limits`].
//! - `status` — what is applied, the trial's seconds left, what the
//!   package draws and what the fan curve is doing.
//! - `apply` — set a whole [`Tune`]; with `trial`, it is undone after
//!   [`crate::TRIAL_SECONDS`] unless kept.
//! - `keep`, `revert` — end the trial one way or the other.
//! - `reset` — everything back to stock.

use lntrn_data::json::{self, JsonLimits};
use lntrn_data::{Doc, Map};

use crate::tune::{Limits, Tune};

/// The longest line either side reads: a tune is well under a kilobyte.
pub const MAX_LINE: usize = 64 * 1024;

/// What a request's tree may cost to parse.
const LIMITS: JsonLimits = JsonLimits { max_bytes: 1024 * 1024 };

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Hello,
    Status,
    Apply { tune: Tune, trial: bool },
    Keep,
    Revert,
    Reset,
}

/// The helper's answer to `hello`.
#[derive(Clone, Debug, PartialEq)]
pub struct Hello {
    pub protocol: i64,
    /// Whether it runs as root, so its changes take.
    pub root: bool,
    /// Whether it only says what it would do (`--dry-run`).
    pub dry_run: bool,
    pub limits: Limits,
}

/// The helper's answer to `status`.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// What the hardware is set to, as the helper set it or found it.
    pub applied: Tune,
    /// Seconds before the trial is undone; `None` with no trial running.
    pub trial: Option<u64>,
    /// How many trials have run out and been undone since the helper
    /// started: the window sees it rise and says so.
    pub reverted: u64,
    /// What the processor package draws, watts.
    pub package_watts: Option<f32>,
    /// The speed the fan curve has the fans at.
    pub fan_percent: Option<u32>,
    /// Something wrong the user should hear about: the curve gave up.
    pub trouble: Option<String>,
}

fn op(name: &str) -> Map {
    let mut m = Map::new();
    m.insert("op", Doc::Str(name.to_owned()));
    m
}

impl Request {
    /// The request as its line, without the newline.
    pub fn to_line(&self) -> String {
        let m = match self {
            Request::Hello => op("hello"),
            Request::Status => op("status"),
            Request::Apply { tune, trial } => {
                let mut m = op("apply");
                m.insert("tune", tune.to_doc());
                m.insert("trial", Doc::Bool(*trial));
                m
            }
            Request::Keep => op("keep"),
            Request::Revert => op("revert"),
            Request::Reset => op("reset"),
        };
        json::write(&Doc::Map(m))
    }

    pub fn parse(line: &str) -> Result<Request, String> {
        let (doc, _) = json::parse_with(line, LIMITS).map_err(|e| format!("not a request: {e}"))?;
        match doc.get("op").and_then(Doc::as_str) {
            Some("hello") => Ok(Request::Hello),
            Some("status") => Ok(Request::Status),
            Some("apply") => {
                let tune = Tune::from_doc(doc.get("tune").ok_or("apply needs a tune")?)?;
                Ok(Request::Apply { tune, trial: doc.get("trial").and_then(Doc::as_bool).unwrap_or(false) })
            }
            Some("keep") => Ok(Request::Keep),
            Some("revert") => Ok(Request::Revert),
            Some("reset") => Ok(Request::Reset),
            Some(other) => Err(format!("unknown request \"{other}\"")),
            None => Err("a request needs an \"op\"".to_owned()),
        }
    }
}

/// A reply saying it worked, with `entries`.
pub fn ok(entries: Vec<(&str, Doc)>) -> String {
    let mut m = Map::new();
    m.insert("ok", Doc::Bool(true));
    for (k, v) in entries {
        m.insert(k, v);
    }
    json::write(&Doc::Map(m))
}

/// A reply saying it didn't, and why.
pub fn fail(error: &str) -> String {
    let mut m = Map::new();
    m.insert("ok", Doc::Bool(false));
    m.insert("error", Doc::Str(error.to_owned()));
    json::write(&Doc::Map(m))
}

/// A reply's document when it worked, its error when it didn't.
pub fn reply(line: &str) -> Result<Doc, String> {
    let (doc, _) = json::parse_with(line, LIMITS).map_err(|e| format!("the helper said something odd: {e}"))?;
    match doc.get("ok").and_then(Doc::as_bool) {
        Some(true) => Ok(doc),
        Some(false) => Err(doc.get("error").and_then(Doc::as_str).unwrap_or("it failed").to_owned()),
        None => Err("the helper's reply has no \"ok\"".to_owned()),
    }
}

fn opt_int(v: Option<i64>) -> Doc {
    v.map_or(Doc::Null, Doc::Int)
}

impl Hello {
    pub fn entries(&self) -> Vec<(&'static str, Doc)> {
        vec![("protocol", Doc::Int(self.protocol)), ("root", Doc::Bool(self.root)), ("dry_run", Doc::Bool(self.dry_run)), ("limits", self.limits.to_doc())]
    }

    pub fn from_doc(d: &Doc) -> Result<Hello, String> {
        Ok(Hello {
            protocol: d.get("protocol").and_then(Doc::as_i64).ok_or("hello has no protocol")?,
            root: d.get("root").and_then(Doc::as_bool).unwrap_or(false),
            dry_run: d.get("dry_run").and_then(Doc::as_bool).unwrap_or(false),
            limits: Limits::from_doc(d.get("limits").ok_or("hello has no limits")?)?,
        })
    }
}

impl Status {
    pub fn entries(&self) -> Vec<(&'static str, Doc)> {
        vec![
            ("applied", self.applied.to_doc()),
            ("trial", opt_int(self.trial.map(|s| s as i64))),
            ("reverted", Doc::Int(self.reverted as i64)),
            ("package_watts", self.package_watts.map_or(Doc::Null, |w| Doc::Float(f64::from(w)))),
            ("fan_percent", opt_int(self.fan_percent.map(i64::from))),
            ("trouble", self.trouble.clone().map_or(Doc::Null, Doc::Str)),
        ]
    }

    pub fn from_doc(d: &Doc) -> Result<Status, String> {
        let count = |key: &str| d.get(key).and_then(Doc::as_i64).and_then(|v| u64::try_from(v).ok());
        Ok(Status {
            applied: Tune::from_doc(d.get("applied").ok_or("status has nothing applied")?)?,
            trial: count("trial"),
            reverted: count("reverted").unwrap_or(0),
            package_watts: d.get("package_watts").and_then(Doc::as_f64).map(|w| w as f32),
            fan_percent: count("fan_percent").and_then(|p| u32::try_from(p).ok()),
            trouble: d.get("trouble").and_then(Doc::as_str).map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tune::tests::limits;

    #[test]
    fn every_request_survives_its_line() {
        let tune = limits().stock;
        for r in [Request::Hello, Request::Status, Request::Apply { tune, trial: true }, Request::Keep, Request::Revert, Request::Reset] {
            let line = r.to_line();
            assert!(!line.contains('\n'));
            assert_eq!(Request::parse(&line), Ok(r));
        }
        assert!(Request::parse("{\"op\":\"format_disk\"}").unwrap_err().contains("format_disk"));
        assert!(Request::parse("{}").is_err());
        assert!(Request::parse("not json").is_err());
        assert!(Request::parse("{\"op\":\"apply\"}").is_err());
    }

    #[test]
    fn replies_carry_their_answer_or_their_error() {
        let hello = Hello { protocol: 1, root: true, dry_run: false, limits: limits() };
        assert_eq!(Hello::from_doc(&reply(&ok(hello.entries())).unwrap()), Ok(hello));
        let status = Status { applied: limits().stock, trial: Some(12), reverted: 3, package_watts: Some(95.5), fan_percent: None, trouble: Some("hot".into()) };
        assert_eq!(Status::from_doc(&reply(&ok(status.entries())).unwrap()), Ok(status));
        assert_eq!(reply(&fail("nope")), Err("nope".to_owned()));
        assert!(reply("{}").is_err());
    }
}
