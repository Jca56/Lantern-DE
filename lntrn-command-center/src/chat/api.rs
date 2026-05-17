//! Anthropic Messages API client — multi-turn loop with tool use.
//!
//! Wire-level (no SDK; raw ureq + serde_json):
//!   request:  { model, max_tokens, system, tools, messages }
//!   response: { content: [text|tool_use], stop_reason }
//!
//! When `stop_reason == "tool_use"`, we execute each tool locally, append
//! the assistant's content + a `user` message of `tool_result` blocks,
//! and re-send. Loop until `end_turn`.
//!
//! Prompt caching: cache_control on the last tool in `tools` caches
//! `tools + system` together (render order: tools → system → messages).
//! Already baked into TOOLS_JSON.

use std::sync::mpsc::{channel, Receiver};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::tools;

pub const MODEL: &str = "claude-haiku-4-5";
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const MAX_TOKENS: u32 = 4096;
/// Cap multi-turn agent loops so a stuck model can't burn through tokens.
pub const MAX_TURNS: u32 = 50;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Append this string to the in-flight assistant message.
    Delta(String),
    /// Per-turn token usage from the API response.
    Usage(super::threads::Usage),
    Done,
    Error(String),
}

/// Owned snapshot of one chat message used as request history.
#[derive(Clone, Debug)]
pub struct OwnedMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    #[serde(other)]
    Unknown,
}

/// Start the request loop on a background thread. Returns immediately;
/// events arrive on the receiver.
pub fn spawn_stream(
    api_key: String,
    system_prompt: String,
    history: Vec<OwnedMessage>,
) -> Receiver<StreamEvent> {
    let (tx, rx) = channel();

    thread::spawn(move || {
        // Mutable working set of messages. Each turn we may append the
        // assistant's response + a user turn of tool_result blocks.
        let mut messages: Vec<Value> = history.into_iter()
            .map(|m| json!({ "role": m.role, "content": m.content }))
            .collect();

        let tools_array: Value = match serde_json::from_str(tools::TOOLS_JSON) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!("tool defs: {e}")));
                return;
            }
        };

        // System prompt as a list of blocks so we can attach cache_control
        // to the last block (caches tools+system together).
        let system_blocks = json!([
            { "type": "text", "text": system_prompt, "cache_control": {"type": "ephemeral"} }
        ]);

        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(120))
            .build();

        let mut first_text_in_session = true;

        for turn in 0..MAX_TURNS {
            // Cache the user-side conversation prefix too: stamp
            // cache_control on the last user content block we send. We
            // rebuild messages_for_request each turn from `messages`,
            // attaching the marker to the last user message.
            let messages_for_request = with_user_cache_breakpoint(&messages);

            let req_body = json!({
                "model": MODEL,
                "max_tokens": MAX_TOKENS,
                "system": system_blocks,
                "tools": tools_array,
                "messages": messages_for_request,
            });
            let body_bytes = match serde_json::to_vec(&req_body) {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(format!("serialize: {e}")));
                    return;
                }
            };

            let response = agent
                .post(API_URL)
                .set("x-api-key", &api_key)
                .set("anthropic-version", ANTHROPIC_VERSION)
                .set("content-type", "application/json")
                .send_bytes(&body_bytes);

            let resp_text = match response {
                Ok(r) => match r.into_string() {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(StreamEvent::Error(format!("read: {e}")));
                        return;
                    }
                },
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

            let parsed: ApiResponse = match serde_json::from_str(&resp_text) {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Error(format!(
                        "parse: {e} — body: {}", truncate(&resp_text, 400),
                    )));
                    return;
                }
            };

            // Emit token usage for this turn so the UI can tally per-thread.
            if let Some(u) = parsed.usage.as_ref() {
                let _ = tx.send(StreamEvent::Usage(super::threads::Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_creation_input_tokens: u.cache_creation_input_tokens,
                    cache_read_input_tokens: u.cache_read_input_tokens,
                }));
            }

            // Pull tool_use blocks while emitting text deltas for the UI.
            let mut tool_calls: Vec<(String, String, Value)> = Vec::new();
            for block in &parsed.content {
                match block {
                    ContentBlock::Text { text } => {
                        // Insert a separator if there was a prior tool
                        // result in this turn, so text doesn't collide
                        // with the previous "→ N bytes" line.
                        if !first_text_in_session {
                            let _ = tx.send(StreamEvent::Delta("\n\n".into()));
                        }
                        first_text_in_session = false;
                        if !text.is_empty() {
                            let _ = tx.send(StreamEvent::Delta(text.clone()));
                        }
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push((id.clone(), name.clone(), input.clone()));
                    }
                    ContentBlock::Unknown => {}
                }
            }

            let stop = parsed.stop_reason.as_deref().unwrap_or("");

            // Append the assistant's content verbatim to history so the
            // next API turn sees the same tool_use IDs we just executed.
            // The Anthropic API expects assistant.content to be either a
            // string or a list of typed blocks; we round-trip the JSON.
            let assistant_content = extract_content_value(&resp_text);
            messages.push(json!({ "role": "assistant", "content": assistant_content }));

            if tool_calls.is_empty() || stop != "tool_use" {
                let _ = tx.send(StreamEvent::Done);
                return;
            }

            // Execute tools and assemble the user turn.
            let mut user_blocks = Vec::with_capacity(tool_calls.len());
            for (id, name, input) in tool_calls {
                let summary = tools::summarize_call(&name, &input);
                let _ = tx.send(StreamEvent::Delta(format!(
                    "{sep}🔧 {summary}",
                    sep = if first_text_in_session { "" } else { "\n\n" },
                )));
                first_text_in_session = false;

                let result = tools::execute(&name, &input);
                let preview = tools::summarize_result(&name, &result);
                let _ = tx.send(StreamEvent::Delta(format!("\n   {preview}")));

                user_blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": result.content,
                    "is_error": result.is_error,
                }));
            }
            messages.push(json!({ "role": "user", "content": user_blocks }));

            if turn + 1 == MAX_TURNS {
                let _ = tx.send(StreamEvent::Delta(format!(
                    "\n\n[tool-loop hit {MAX_TURNS}-turn cap]"
                )));
                let _ = tx.send(StreamEvent::Done);
                return;
            }
        }
    });

    rx
}

/// Extract the raw `content` value from a response body so we can store it
/// verbatim as `assistant.content` for the next turn.
fn extract_content_value(body: &str) -> Value {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("content").cloned())
        .unwrap_or(Value::Array(Vec::new()))
}

/// Return a copy of `messages` with `cache_control` placed on the last
/// content block of the most recent `user` turn. If the last user message
/// has string content, promote it to a `[{type: text, text, cache_control}]`
/// block list so the marker can attach.
fn with_user_cache_breakpoint(messages: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = messages.to_vec();
    // Walk from the end to find the last user message.
    for i in (0..out.len()).rev() {
        let is_user = out[i].get("role").and_then(|r| r.as_str()) == Some("user");
        if !is_user { continue; }
        let content = out[i].get("content").cloned().unwrap_or(Value::Null);
        let new_content = match content {
            Value::String(s) => json!([
                { "type": "text", "text": s, "cache_control": {"type": "ephemeral"} }
            ]),
            Value::Array(mut arr) => {
                if let Some(last) = arr.last_mut() {
                    if let Some(obj) = last.as_object_mut() {
                        obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                    }
                }
                Value::Array(arr)
            }
            other => other,
        };
        if let Some(obj) = out[i].as_object_mut() {
            obj.insert("content".into(), new_content);
        }
        break;
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}…", &s[..max]) }
}

// Backwards-compat constants kept for other modules that may import them.
#[allow(dead_code)]
pub const SYSTEM_PROMPT_LEGACY: &str = "";
