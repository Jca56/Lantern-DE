//! Line protocol between the GUI and lntrn-afterglowd.
//!
//! One request line in, one response line out. Responses are either
//! `ok`, `ok key=value;key=value;...`, or `err <message>`. Values must not
//! contain ';' or newlines (hardware names and sysfs tokens never do).

use std::collections::HashMap;
use std::fmt::Display;

#[derive(Default)]
pub struct Fields(String);

impl Fields {
    pub fn new() -> Fields {
        Fields(String::new())
    }

    pub fn push(&mut self, key: &str, value: impl Display) {
        if !self.0.is_empty() {
            self.0.push(';');
        }
        self.0.push_str(key);
        self.0.push('=');
        self.0.push_str(&value.to_string());
    }

    pub fn into_ok(self) -> String {
        if self.0.is_empty() {
            "ok".to_string()
        } else {
            format!("ok {}", self.0)
        }
    }
}

pub fn err(msg: impl Display) -> String {
    format!("err {msg}")
}

/// Parses a response line into its fields, or the error message.
pub fn parse(line: &str) -> Result<HashMap<String, String>, String> {
    if let Some(rest) = line.strip_prefix("err") {
        return Err(rest.trim().to_string());
    }
    let Some(rest) = line.strip_prefix("ok") else {
        return Err(format!("malformed response: {line}"));
    };
    let mut map = HashMap::new();
    for pair in rest.trim().split(';') {
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(k.to_string(), v.to_string());
        }
    }
    Ok(map)
}

pub fn get_num<T: std::str::FromStr>(map: &HashMap<String, String>, key: &str) -> Option<T> {
    map.get(key)?.parse().ok()
}
