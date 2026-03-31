//! Minimal `notify-send` compatible CLI that calls lntrn-notifyd via D-Bus.

use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: notify-send [OPTIONS] <SUMMARY> [BODY]");
        eprintln!("  -u, --urgency <low|normal|critical>");
        eprintln!("  -t, --expire-time <ms>");
        eprintln!("  -a, --app-name <NAME>");
        return;
    }

    let mut urgency: u8 = 1; // normal
    let mut timeout: i32 = -1;
    let mut app_name = String::new();
    let mut positional = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-u" | "--urgency" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    urgency = match v.as_str() {
                        "low" => 0,
                        "critical" => 2,
                        _ => 1,
                    };
                }
            }
            "-t" | "--expire-time" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    timeout = v.parse().unwrap_or(-1);
                }
            }
            "-a" | "--app-name" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    app_name = v.clone();
                }
            }
            // Skip other dashed flags we don't support (e.g. -i, -c)
            s if s.starts_with('-') => {
                // If next arg looks like a value for this flag, skip it too
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    i += 1;
                }
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    let summary = positional.first().map(|s| s.as_str()).unwrap_or("");
    let body = positional.get(1).map(|s| s.as_str()).unwrap_or("");

    if let Err(e) = send_notification(&app_name, summary, body, urgency, timeout) {
        eprintln!("notify-send: {e}");
        std::process::exit(1);
    }
}

fn send_notification(
    app_name: &str,
    summary: &str,
    body: &str,
    urgency: u8,
    timeout: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::session()?;
    let proxy = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            app_name,
            0u32, // replaces_id
            "",    // app_icon
            summary,
            body,
            Vec::<String>::new(), // actions
            HashMap::from([("urgency", Value::from(urgency))]),
            timeout,
        ),
    )?;
    let _id: u32 = proxy.body().deserialize()?;
    Ok(())
}
