//! Glue between `TextHandler` and `LspManager`. This file owns the logic
//! for: flushing didOpen/didChange/didSave as documents change, dispatching
//! inbound `ServerMessage`s arriving on each tick, and kicking off LSP
//! requests from key/mouse events (hover, completion, goto-definition).
//!
//! Separated from `main.rs` so the UI loop stays focused and so each LSP
//! feature can grow without inflating one giant file.

use std::time::{Duration, Instant};

use serde_json::Value;

use super::client::PendingKind;
use super::protocol::{
    CompletionItem as ProtoCompletionItem, HoverParams, DefinitionParams, CompletionParams,
    CompletionContext, Position, TextDocumentIdentifier, TextDocumentPositionParams,
};
use super::{CompletionItem, ServerId, ServerMessage};
use crate::editor::Editor;
use crate::TextHandler;

/// Debounced Ctrl+hover: when the mouse has been still for a beat with Ctrl
/// held, fire a hover request at the point under the cursor. Called once
/// per `about_to_wait` tick.
pub fn tick_ctrl_hover(handler: &mut TextHandler, now: Instant) {
    let ctrl_held = handler
        .modifiers
        .contains(winit::keyboard::ModifiersState::CONTROL);
    if !ctrl_held
        || handler.hover.visible
        || handler.hover.in_flight.is_some()
        || now.duration_since(handler.last_cursor_move) < Duration::from_millis(250)
    {
        return;
    }
    let Some((cx, cy)) = handler.input.cursor() else { return };
    let Some((line, col)) = handler.doc_pos_at(cx, cy) else { return };
    request_hover_at(handler, line, col, cx, cy + 20.0 * handler.scale);
    // Park last_cursor_move far in the future so we don't refire until the
    // user moves again (which resets it from main.rs).
    handler.last_cursor_move = now + Duration::from_secs(3600);
}

/// Called every frame's user_event tick. Flushes queued document state to
/// the LSP (didOpen on newly loaded files, didChange after edits).
pub fn flush_document_state(handler: &mut TextHandler) {
    for i in 0..handler.tabs.len() {
        // Skip preview tabs and untitled docs.
        if handler.tabs[i].is_preview() {
            continue;
        }
        let Some(path) = handler.tabs[i].file_path.clone() else { continue };

        if handler.tabs[i].lsp_just_opened {
            let lang = handler.tabs[i].language;
            let filename = handler.tabs[i].filename.clone();
            let text = handler.tabs[i].full_text();
            handler.lsp.did_open(lang, &filename, &path, &text);
            handler.tabs[i].lsp_just_opened = false;
            handler.tabs[i].lsp_dirty = false;
        } else if handler.tabs[i].lsp_dirty {
            let text = handler.tabs[i].full_text();
            handler.lsp.did_change(&path, &text);
            handler.tabs[i].lsp_dirty = false;
        }
    }
}

/// Handle all pending inbound messages from every server. Called when the
/// winit user-event `UserEvent::LspMessage` fires.
pub fn drain_inbound(handler: &mut TextHandler) {
    let msgs = handler.lsp.drain();
    for (server_id, msg) in msgs {
        handle_one(handler, server_id, msg);
    }
}

fn handle_one(handler: &mut TextHandler, server_id: ServerId, msg: ServerMessage) {
    match msg {
        ServerMessage::Diagnostics(diag) => {
            handler.lsp.set_diagnostics(diag.uri, diag.diagnostics);
            handler.needs_redraw = true;
        }
        ServerMessage::LogMessage(text) => {
            // Filter pyright noise that doesn't represent real problems:
            //   * "No source files found"  — just no pyrightconfig.json in scope.
            //   * "<default workspace root>" — leftover when the client only
            //     sent rootUri (we now also send workspaceFolders, but old
            //     servers/edge cases can still surface this).
            // The open file still gets full type-checking either way.
            if text.contains("No source files found")
                || text.contains("<default workspace root>")
            {
                return;
            }
            handler.lsp_status = format!("{}: {}", server_id.label(), text);
            handler.needs_redraw = true;
        }
        ServerMessage::Response { id, result, error } => {
            let Some(kind) = handler.lsp.take_pending(server_id, id) else {
                return;
            };
            if let Some((code, msg)) = error {
                eprintln!(
                    "[lntrn-code] lsp {}: error {code}: {msg}",
                    server_id.label()
                );
                return;
            }
            handle_response(handler, server_id, kind, result);
        }
        ServerMessage::Unknown => {}
    }
}

fn handle_response(
    handler: &mut TextHandler,
    server_id: ServerId,
    kind: PendingKind,
    result: Option<Value>,
) {
    match kind {
        PendingKind::Initialize => {
            if let Some(c) = handler.lsp.client_mut(server_id) {
                let _ = c.on_initialized();
            }
            handler.needs_redraw = true;
        }
        PendingKind::Hover { uri: _ } => {
            let text = result
                .as_ref()
                .and_then(extract_hover_text)
                .unwrap_or_default();
            handler.hover.in_flight = None;
            if text.trim().is_empty() {
                handler.hover.clear();
            } else {
                handler.hover.visible = true;
                handler.hover.text = text;
            }
            handler.needs_redraw = true;
        }
        PendingKind::Completion { uri: _, line, col, prefix, tab_id } => {
            handler.completion.in_flight = None;
            // If the user already tabbed away or dismissed, drop this response.
            if handler.tabs[handler.active_tab].tab_id != tab_id {
                return;
            }
            let items = extract_completion_items(result.as_ref());
            if items.is_empty() {
                handler.completion.clear();
                return;
            }
            handler.completion.visible = true;
            handler.completion.items = items;
            handler.completion.prefix_start_col = col as usize;
            handler.completion.line = line as usize;
            handler.completion.prefix = prefix;
            handler.completion.selected = 0;
            handler.completion.tab_id = tab_id;
            handler.needs_redraw = true;
        }
        PendingKind::Definition { uri: _, tab_id, new_tab } => {
            let Some(loc) = result.as_ref().and_then(extract_definition) else {
                return;
            };
            let (target_uri, pos) = loc;
            let Some(target_path) = super::client::uri_to_path(&target_uri) else {
                return;
            };
            open_or_focus(handler, target_path, pos.line as usize, pos.character as usize, new_tab, tab_id);
            handler.needs_redraw = true;
        }
    }
}

fn open_or_focus(
    handler: &mut TextHandler,
    path: std::path::PathBuf,
    line: usize,
    col: usize,
    _force_new: bool,
    _from_tab: u64,
) {
    // If the target is already in a tab, focus it. Otherwise load it.
    for (i, tab) in handler.tabs.iter().enumerate() {
        if tab.file_path.as_ref() == Some(&path) {
            handler.active_tab = i;
            handler.tabs[i].cursor_line = line;
            handler.tabs[i].cursor_col = col;
            return;
        }
    }
    let mut e = Editor::new();
    e.tab_id = handler.next_tab_id;
    handler.next_tab_id += 1;
    if e.load_file(path).is_ok() {
        e.cursor_line = line;
        e.cursor_col = col;
        handler.tabs.push(e);
        handler.active_tab = handler.tabs.len() - 1;
    }
}

// ── Response decoders ───────────────────────────────────────────────────────

fn extract_hover_text(v: &Value) -> Option<String> {
    // Spec allows either Hover { contents: MarkedString | MarkedString[] | MarkupContent }
    // or plain string. We handle the common shapes and fall through on null.
    let contents = v.get("contents")?;
    if let Some(s) = contents.as_str() {
        return Some(s.to_string());
    }
    // MarkupContent { kind, value }
    if let Some(val) = contents.get("value").and_then(|v| v.as_str()) {
        return Some(val.to_string());
    }
    // Array of MarkedString
    if let Some(arr) = contents.as_array() {
        let mut out = String::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(s);
            } else if let Some(s) = item.get("value").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(s);
            }
        }
        return Some(out);
    }
    None
}

fn extract_completion_items(v: Option<&Value>) -> Vec<CompletionItem> {
    let Some(v) = v else { return Vec::new() };
    let items_arr = if v.is_array() {
        v.as_array()
    } else {
        v.get("items").and_then(|i| i.as_array())
    };
    let Some(arr) = items_arr else { return Vec::new() };
    let mut out = Vec::with_capacity(arr.len().min(64));
    for raw in arr.iter().take(256) {
        if let Ok(p) = serde_json::from_value::<ProtoCompletionItem>(raw.clone()) {
            out.push(CompletionItem::from_proto(p));
        }
    }
    out
}

fn extract_definition(v: &Value) -> Option<(String, Position)> {
    // Definition result: Location | Location[] | LocationLink[] | null
    let one = if v.is_array() {
        v.as_array()?.first()?
    } else {
        v
    };
    // Location { uri, range }
    if let (Some(uri), Some(range)) = (
        one.get("uri").and_then(|u| u.as_str()),
        one.get("range"),
    ) {
        let pos = range.get("start")?;
        let line = pos.get("line")?.as_u64()? as u32;
        let character = pos.get("character")?.as_u64()? as u32;
        return Some((uri.to_string(), Position { line, character }));
    }
    // LocationLink { targetUri, targetSelectionRange | targetRange }
    if let Some(uri) = one.get("targetUri").and_then(|u| u.as_str()) {
        let range = one.get("targetSelectionRange")
            .or_else(|| one.get("targetRange"))?;
        let pos = range.get("start")?;
        let line = pos.get("line")?.as_u64()? as u32;
        let character = pos.get("character")?.as_u64()? as u32;
        return Some((uri.to_string(), Position { line, character }));
    }
    None
}

// ── Request senders ─────────────────────────────────────────────────────────

/// Figure out (doc_line, byte_col) -> LSP Position (utf-16-ish; we send utf-8
/// offsets and let rust-analyzer cope — it tolerates that in default mode).
fn lsp_position(editor: &Editor) -> Position {
    Position {
        line: editor.cursor_line as u32,
        character: utf16_column(&editor.lines[editor.cursor_line], editor.cursor_col),
    }
}

/// LSP wants utf-16 character offsets on a line by default. Convert our
/// byte column.
fn utf16_column(line: &str, byte_col: usize) -> u32 {
    let slice = &line[..byte_col.min(line.len())];
    slice.encode_utf16().count() as u32
}

/// Send a hover request at an explicit document position (line/col). The
/// caller chose the position — typically "what's under the mouse" for
/// Ctrl+hover, or "the caret" for a keyboard-triggered hover.
pub fn request_hover_at(
    handler: &mut TextHandler,
    line: usize,
    col: usize,
    anchor_x: f32,
    anchor_y: f32,
) {
    let editor = &handler.tabs[handler.active_tab];
    let Some(path) = editor.file_path.clone() else { return };
    let Some(server_id) = super::server_for_language(editor.language, &editor.filename) else {
        return;
    };
    let line_str = editor.lines.get(line).map(|s| s.as_str()).unwrap_or("");
    let character = utf16_column(line_str, col);
    let uri = super::client::path_to_uri(&path);
    let Some(client) = handler.lsp.client_mut(server_id) else { return };
    let params: HoverParams = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: Position { line: line as u32, character },
    };
    let Ok(id) = client.send_request("textDocument/hover", &params) else {
        return;
    };
    client.pending.insert(id, PendingKind::Hover { uri });
    handler.hover.in_flight = Some(id);
    handler.hover.anchor_x = anchor_x;
    handler.hover.anchor_y = anchor_y;
}

pub fn request_completion(handler: &mut TextHandler, anchor_x: f32, anchor_y: f32) {
    let tab_idx = handler.active_tab;
    let editor = &handler.tabs[tab_idx];
    let Some(path) = editor.file_path.clone() else { return };
    let Some(server_id) = super::server_for_language(editor.language, &editor.filename) else {
        return;
    };

    let line_str = &editor.lines[editor.cursor_line];
    let (prefix_start, prefix) = prefix_before_cursor(line_str, editor.cursor_col);

    let uri = super::client::path_to_uri(&path);
    let pos = Position {
        line: editor.cursor_line as u32,
        character: utf16_column(line_str, editor.cursor_col),
    };
    let tab_id = editor.tab_id;

    let Some(client) = handler.lsp.client_mut(server_id) else { return };
    let params = CompletionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: pos,
        context: CompletionContext {
            trigger_kind: 1, // Invoked
            trigger_character: None,
        },
    };
    let Ok(id) = client.send_request("textDocument/completion", &params) else {
        return;
    };
    client.pending.insert(
        id,
        PendingKind::Completion {
            uri,
            line: pos.line,
            col: prefix_start as u32,
            prefix: prefix.clone(),
            tab_id,
        },
    );
    handler.completion.in_flight = Some(id);
    handler.completion.anchor_x = anchor_x;
    handler.completion.anchor_y = anchor_y;
    handler.completion.prefix = prefix;
    handler.completion.prefix_start_col = prefix_start;
    handler.completion.line = pos.line as usize;
    handler.completion.tab_id = tab_id;
}

/// Find the start of the identifier prefix directly before the cursor.
fn prefix_before_cursor(line: &str, byte_col: usize) -> (usize, String) {
    let end = byte_col.min(line.len());
    let bytes = line.as_bytes();
    let mut start = end;
    while start > 0 {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || c == b'_' {
            start -= 1;
        } else {
            break;
        }
    }
    (start, line[start..end].to_string())
}

pub fn request_definition(handler: &mut TextHandler, new_tab: bool) {
    let tab_idx = handler.active_tab;
    let editor = &handler.tabs[tab_idx];
    let Some(path) = editor.file_path.clone() else { return };
    let Some(server_id) = super::server_for_language(editor.language, &editor.filename) else {
        return;
    };
    let uri = super::client::path_to_uri(&path);
    let pos = lsp_position(editor);
    let tab_id = editor.tab_id;
    let Some(client) = handler.lsp.client_mut(server_id) else { return };
    let params: DefinitionParams = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: pos,
    };
    let Ok(id) = client.send_request("textDocument/definition", &params) else {
        return;
    };
    client.pending.insert(id, PendingKind::Definition { uri, tab_id, new_tab });
}

/// Called from save_file_dialog after we've written to disk.
pub fn notify_did_save(handler: &mut TextHandler) {
    let editor = &handler.tabs[handler.active_tab];
    let Some(path) = editor.file_path.clone() else { return };
    let text = editor.full_text();
    handler.lsp.did_save(&path, &text);
}

/// Called when a tab is closed (or replaced) so the server stops indexing it.
pub fn notify_did_close(handler: &mut TextHandler, tab_idx: usize) {
    let Some(path) = handler.tabs[tab_idx].file_path.clone() else { return };
    handler.lsp.did_close(&path);
}

/// Apply a completion item at the current cursor. Replaces the typed prefix
/// with the item's insert text.
pub fn accept_completion(handler: &mut TextHandler, item: &CompletionItem) {
    let ed = &mut handler.tabs[handler.active_tab];
    let line_idx = ed.cursor_line;
    let line_len = ed.lines[line_idx].len();
    let prefix_start = handler.completion.prefix_start_col.min(line_len);
    let cur_col = ed.cursor_col.min(line_len);
    if prefix_start < cur_col {
        ed.lines[line_idx].replace_range(prefix_start..cur_col, "");
        ed.cursor_col = prefix_start;
    }
    ed.insert_str(item.insert());
    handler.completion.clear();
}

