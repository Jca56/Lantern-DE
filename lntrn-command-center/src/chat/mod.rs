//! Claude-powered chatbot tab.
//!
//! Modules:
//! - `keychain` — reads the Anthropic API key from lntrn-keychain.
//! - `api`      — background-thread streaming HTTP client.
//! - `threads`  — JSON persistence of conversations.
//! - `markdown` — block + inline parser for assistant replies.
//! - `highlight`— per-language tokenizer for fenced code.
//! - `render`   — drawing routines.
//! - `input`    — keyboard handling.

#![allow(dead_code)] // hover-delete + thread-delete UI lands in the next iteration

pub mod api;
pub mod highlight;
pub mod input;
pub mod keychain;
pub mod markdown;
pub mod render;
pub mod threads;

use std::sync::mpsc::{Receiver, TryRecvError};

use api::{OwnedMessage, StreamEvent};
pub use threads::{Message, Role, Thread};

pub struct ChatState {
    pub threads: Vec<Thread>,
    pub active: Option<usize>,
    pub draft: crate::search::input::Input,
    pub streaming: bool,
    pub pending: String,
    pub stream_rx: Option<Receiver<StreamEvent>>,
    pub api_key: Option<String>,
    pub key_error: Option<String>,
    pub last_error: Option<String>,
    pub messages_scroll: f32,
    pub messages_scroll_max: f32,
    pub sidebar_scroll: f32,
    pub sidebar_scroll_max: f32,
    pub hover_thread: Option<usize>,
    pub hover_new_thread: bool,
    pub hover_send: bool,
    pub hover_delete: Option<usize>,
    pub confirm_delete: Option<usize>,
}

impl Default for ChatState {
    fn default() -> Self { Self::new() }
}

impl ChatState {
    pub fn new() -> Self {
        let threads = threads::load_all();
        // Always start at the "home" view — user picks a thread or
        // creates a new one explicitly.
        let active = None;

        let (api_key, key_error) = match keychain::fetch_api_key() {
            Ok(Some(k)) => (Some(k), None),
            Ok(None) => (None, Some(
                "No item with name=\"Claude API\" in the login keyring.".into(),
            )),
            Err(e) => (None, Some(format!("keychain: {e}"))),
        };

        Self {
            threads,
            active,
            draft: crate::search::input::Input::new(),
            streaming: false,
            pending: String::new(),
            stream_rx: None,
            api_key,
            key_error,
            last_error: None,
            messages_scroll: 0.0,
            messages_scroll_max: 0.0,
            sidebar_scroll: 0.0,
            sidebar_scroll_max: 0.0,
            hover_thread: None,
            hover_new_thread: false,
            hover_send: false,
            hover_delete: None,
            confirm_delete: None,
        }
    }

    pub fn active_thread(&self) -> Option<&Thread> {
        self.active.and_then(|i| self.threads.get(i))
    }

    pub fn active_thread_mut(&mut self) -> Option<&mut Thread> {
        self.active.and_then(|i| self.threads.get_mut(i))
    }

    /// Drain pending stream events. Returns true if anything changed (so the
    /// caller can request a redraw).
    pub fn poll_stream(&mut self) -> bool {
        let Some(rx) = self.stream_rx.as_ref() else { return false; };
        let mut changed = false;
        loop {
            match rx.try_recv() {
                Ok(StreamEvent::Delta(s)) => {
                    self.pending.push_str(&s);
                    changed = true;
                }
                Ok(StreamEvent::Done) => {
                    self.commit_assistant();
                    self.streaming = false;
                    self.stream_rx = None;
                    changed = true;
                    break;
                }
                Ok(StreamEvent::Error(e)) => {
                    if !self.pending.is_empty() {
                        // partial reply still gets stored
                        self.commit_assistant();
                    }
                    self.streaming = false;
                    self.stream_rx = None;
                    self.last_error = Some(e);
                    changed = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.pending.is_empty() {
                        self.commit_assistant();
                    }
                    self.streaming = false;
                    self.stream_rx = None;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    /// Move accumulated `pending` text into the active thread as an assistant
    /// message, then save the thread to disk.
    fn commit_assistant(&mut self) {
        let text = std::mem::take(&mut self.pending);
        if text.is_empty() { return; }
        let Some(idx) = self.active else { return; };
        let Some(thread) = self.threads.get_mut(idx) else { return; };
        thread.messages.push(Message {
            role: Role::Assistant,
            content: text,
        });
        thread.modified = threads::unix_now();
        threads::save(thread);
    }

    pub fn submit_draft(&mut self) {
        if self.streaming { return; }
        let Some(api_key) = self.api_key.clone() else {
            self.last_error = Some("API key not available — set it in lntrn-keychain.".into());
            return;
        };
        let text = self.draft.query().trim().to_string();
        if text.is_empty() { return; }
        self.draft.clear();

        if self.active.is_none() {
            self.threads.insert(0, Thread::new());
            self.active = Some(0);
        }

        let history_owned: Vec<OwnedMessage>;
        {
            let Some(idx) = self.active else { return; };
            let Some(thread) = self.threads.get_mut(idx) else { return; };
            thread.messages.push(Message {
                role: Role::User,
                content: text,
            });
            thread.modified = threads::unix_now();
            thread.auto_title();
            threads::save(thread);

            history_owned = thread.messages.iter().map(|m| OwnedMessage {
                role: m.role.as_api_str().to_string(),
                content: m.content.clone(),
            }).collect();
        }

        self.pending.clear();
        self.last_error = None;
        self.streaming = true;
        self.stream_rx = Some(api::spawn_stream(api_key, history_owned));
        // pin scroll to bottom on send
        self.messages_scroll = f32::MAX;
    }

    pub fn new_thread(&mut self) {
        self.threads.insert(0, Thread::new());
        self.active = Some(0);
        self.draft.clear();
        self.pending.clear();
        self.last_error = None;
        self.messages_scroll = 0.0;
    }

    pub fn select_thread(&mut self, idx: usize) {
        if idx < self.threads.len() {
            self.active = Some(idx);
            self.draft.clear();
            self.pending.clear();
            self.last_error = None;
            self.messages_scroll = f32::MAX;
        }
    }

    pub fn delete_thread(&mut self, idx: usize) {
        if idx >= self.threads.len() { return; }
        let id = self.threads[idx].id.clone();
        threads::delete(&id);
        self.threads.remove(idx);
        self.confirm_delete = None;
        self.active = if self.threads.is_empty() { None }
            else { Some(idx.min(self.threads.len() - 1)) };
        self.pending.clear();
        self.last_error = None;
    }

    pub fn scroll_messages(&mut self, dy: f32) {
        let new_scroll = (self.messages_scroll + dy).clamp(0.0, self.messages_scroll_max);
        self.messages_scroll = new_scroll;
    }

    pub fn scroll_sidebar(&mut self, dy: f32) {
        let new_scroll = (self.sidebar_scroll + dy).clamp(0.0, self.sidebar_scroll_max);
        self.sidebar_scroll = new_scroll;
    }
}

pub use render::draw;
