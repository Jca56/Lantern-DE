//! Anthropic Messages API client — streaming SSE over a background thread.
//!
//! Spawn a request with `spawn_stream`; receive `StreamEvent`s on the
//! returned channel from the GUI thread (poll non-blocking per frame).

use std::io::{BufRead, BufReader};
use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const MODEL: &str = "claude-haiku-4-5";
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const MAX_TOKENS: u32 = 4096;
pub const SYSTEM_PROMPT: &str =
    "You are a helpful assistant inside the Lantern DE command center. \
     Answer concisely. Use markdown for formatting and fenced code blocks for code.";

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Delta(String),
    Done,
    Error(String),
}

#[derive(Serialize)]
struct ApiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ApiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    stream: bool,
    system: &'a str,
    messages: Vec<ApiMessage<'a>>,
}

#[derive(Deserialize)]
struct SseEnvelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<SseDelta>,
    #[serde(default)]
    error: Option<SseError>,
}

#[derive(Deserialize)]
struct SseDelta {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct SseError {
    #[serde(default)]
    #[serde(rename = "type")]
    _kind: String,
    #[serde(default)]
    message: String,
}

/// Owned snapshot of one chat message so the background thread doesn't
/// need a reference into ChatState.
#[derive(Clone)]
pub struct OwnedMessage {
    pub role: String,
    pub content: String,
}

/// Start a streaming request. Returns immediately; events arrive on the rx.
pub fn spawn_stream(api_key: String, history: Vec<OwnedMessage>) -> Receiver<StreamEvent> {
    let (tx, rx) = channel();

    thread::spawn(move || {
        let messages: Vec<ApiMessage> = history.iter()
            .map(|m| ApiMessage { role: &m.role, content: &m.content })
            .collect();

        let req = ApiRequest {
            model: MODEL,
            max_tokens: MAX_TOKENS,
            stream: true,
            system: SYSTEM_PROMPT,
            messages,
        };
        let body = match serde_json::to_vec(&req) {
            Ok(b) => b,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!("serialize: {e}")));
                return;
            }
        };

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(120))
            .build();

        let response = agent
            .post(API_URL)
            .set("x-api-key", &api_key)
            .set("anthropic-version", ANTHROPIC_VERSION)
            .set("content-type", "application/json")
            .set("accept", "text/event-stream")
            .send_bytes(&body);

        let response = match response {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                let _ = tx.send(StreamEvent::Error(
                    format!("HTTP {code}: {}", truncate(&body, 400)),
                ));
                return;
            }
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!("request: {e}")));
                return;
            }
        };

        let mut reader = BufReader::new(response.into_reader());
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(format!("read: {e}")));
                    return;
                }
            }
            let trimmed = line.trim_end_matches(|c| c == '\r' || c == '\n');
            let Some(data) = trimmed.strip_prefix("data: ") else { continue };
            if data == "[DONE]" { break; }

            match serde_json::from_str::<SseEnvelope>(data) {
                Ok(env) => match env.kind.as_str() {
                    "content_block_delta" => {
                        if let Some(d) = env.delta {
                            if d.kind == "text_delta" && !d.text.is_empty() {
                                if tx.send(StreamEvent::Delta(d.text)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    "message_stop" => break,
                    "error" => {
                        let msg = env.error.map(|e| e.message)
                            .unwrap_or_else(|| "unknown error".into());
                        let _ = tx.send(StreamEvent::Error(msg));
                        return;
                    }
                    _ => {}
                },
                Err(_) => {
                    // ignore unparseable lines (ping, comments, etc.)
                }
            }
        }

        let _ = tx.send(StreamEvent::Done);
    });

    rx
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}…", &s[..max]) }
}
