//! Minimal `notify-send` compatible CLI that calls lntrn-notifyd via D-Bus.
//! As libnotify's: `-A [NAME=]Label` adds an action (and waits), `-w`
//! waits; waiting, the name of the action clicked (its number, when it has
//! no name) is printed, and it ends once the notification closes.

use std::collections::HashMap;
use std::time::Duration;

use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::Value;
use zbus::MatchRule;

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// Waiting gives up after this, whatever the server does.
const WAIT_AT_MOST: Duration = Duration::from_secs(3600);

/// What the command line asks.
#[derive(Debug, Default, PartialEq)]
struct Asked {
    urgency: u8,
    timeout: i32,
    app_name: String,
    summary: String,
    body: String,
    /// Key, label; key then label as the spec lists them.
    actions: Vec<(String, String)>,
    wait: bool,
}

fn parse(args: &[String]) -> Asked {
    let mut asked = Asked { urgency: 1, timeout: -1, ..Asked::default() };
    let mut positional = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        // `--flag=value` and `--flag value` alike.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_owned())),
            _ => (arg, None),
        };
        let mut value = || inline.clone().or_else(|| {
            i += 1;
            args.get(i).cloned()
        });
        match flag {
            "-u" | "--urgency" => {
                asked.urgency = match value().as_deref() {
                    Some("low") => 0,
                    Some("critical") => 2,
                    _ => 1,
                }
            }
            "-t" | "--expire-time" => asked.timeout = value().and_then(|v| v.parse().ok()).unwrap_or(-1),
            "-a" | "--app-name" => asked.app_name = value().unwrap_or_default(),
            "-A" | "--action" => {
                if let Some(v) = value() {
                    let n = asked.actions.len().to_string();
                    let (key, label) = v.split_once('=').map_or((n, v.clone()), |(k, l)| (k.to_owned(), l.to_owned()));
                    asked.actions.push((key, label));
                }
            }
            "-w" | "--wait" => asked.wait = true,
            // Skip other dashed flags we don't support (e.g. -i, -c)
            s if s.starts_with('-') && s.len() > 1 => {
                // If next arg looks like a value for this flag, skip it too
                if inline.is_none() && i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    i += 1;
                }
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }
    asked.summary = positional.first().cloned().unwrap_or_default();
    asked.body = positional.get(1).cloned().unwrap_or_default();
    asked.wait |= !asked.actions.is_empty();
    asked
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: notify-send [OPTIONS] <SUMMARY> [BODY]");
        eprintln!("  -u, --urgency <low|normal|critical>");
        eprintln!("  -t, --expire-time <ms>");
        eprintln!("  -a, --app-name <NAME>");
        eprintln!("  -A, --action <[NAME=]LABEL>  an action; waits, printing the one clicked");
        eprintln!("  -w, --wait                   wait for the notification to close");
        return;
    }

    if let Err(e) = send(&parse(&args)) {
        eprintln!("notify-send: {e}");
        std::process::exit(1);
    }
}

fn send(asked: &Asked) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::session()?;
    // Listening before it's sent, so nothing it says is missed.
    let rule = MatchRule::builder().msg_type(Type::Signal).interface(DEST)?.path(PATH)?.build();
    let signals = asked.wait.then(|| MessageIterator::for_match_rule(rule, &conn, None)).transpose()?;
    let actions: Vec<&str> = asked.actions.iter().flat_map(|(k, l)| [k.as_str(), l.as_str()]).collect();
    let reply = conn.call_method(
        Some(DEST),
        PATH,
        Some(DEST),
        "Notify",
        &(
            asked.app_name.as_str(),
            0u32, // replaces_id
            "",   // app_icon
            asked.summary.as_str(),
            asked.body.as_str(),
            actions,
            HashMap::from([("urgency", Value::from(asked.urgency))]),
            asked.timeout,
        ),
    )?;
    let id: u32 = reply.body().deserialize()?;
    let Some(signals) = signals else { return Ok(()) };
    std::thread::spawn(|| {
        std::thread::sleep(WAIT_AT_MOST);
        std::process::exit(0);
    });
    for msg in signals {
        let msg = msg?;
        let header = msg.header();
        match header.member().map(|m| m.as_str()) {
            Some("ActionInvoked") => {
                let (of, key): (u32, String) = msg.body().deserialize()?;
                if of == id {
                    println!("{key}");
                }
            }
            Some("NotificationClosed") => {
                let (of, _reason): (u32, u32) = msg.body().deserialize()?;
                if of == id {
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn actions_are_named_or_numbered_and_mean_waiting() {
        let asked = parse(&args(&["-a", "Bluetooth", "-i", "bluetooth", "-A", "open=Open", "--action=Show in folder", "Received photo.jpg", "From Pixel 9"]));
        assert_eq!(asked.actions, [("open".to_owned(), "Open".to_owned()), ("1".to_owned(), "Show in folder".to_owned())]);
        assert_eq!((asked.app_name.as_str(), asked.summary.as_str(), asked.body.as_str(), asked.wait), ("Bluetooth", "Received photo.jpg", "From Pixel 9", true));
        let plain = parse(&args(&["-u", "critical", "--expire-time=5000", "Hi"]));
        assert_eq!((plain.urgency, plain.timeout, plain.wait, plain.summary.as_str()), (2, 5000, false, "Hi"));
        assert!(parse(&args(&["-w", "Hi"])).wait);
    }
}
