//! In-memory state for the running daemon.
//!
//! Single-threaded mutable state shared across the main dispatch loop.
//! All persistence is flushed to disk by `service::collection` on every
//! mutation so a crash can't lose more than one in-flight write.

use std::collections::HashMap;

use crate::storage::crypto::MasterKey;
use crate::storage::Item as StoredItem;

/// Live state — wraps everything the dispatcher needs to touch.
pub struct ServiceState {
    pub collections: HashMap<String, Collection>,
    pub sessions: HashMap<u64, Session>,
    pub prompts: HashMap<u64, Prompt>,
    pub aliases: HashMap<String, String>,
    pub next_session_id: u64,
    pub next_prompt_id: u64,
    /// Counter used to mint item ids inside collections.
    pub next_item_id: u64,
}

impl ServiceState {
    pub fn new() -> Self {
        Self {
            collections: HashMap::new(),
            sessions: HashMap::new(),
            prompts: HashMap::new(),
            aliases: HashMap::new(),
            next_session_id: 0,
            next_prompt_id: 0,
            next_item_id: 1,
        }
    }

    pub fn allocate_session_id(&mut self) -> u64 {
        self.next_session_id += 1;
        self.next_session_id
    }

    pub fn allocate_prompt_id(&mut self) -> u64 {
        self.next_prompt_id += 1;
        self.next_prompt_id
    }
}

/// One loaded collection. `master_key` is `Some` when unlocked, `None` when
/// locked. Items remain in memory even when locked because the on-disk file
/// must be decrypted to read them — once decrypted they stay until the user
/// explicitly relocks the collection.
pub struct Collection {
    pub id: String,
    pub label: String,
    pub created: u64,
    pub modified: u64,
    pub items: HashMap<String, Item>,
    pub master_key: Option<MasterKey>,
}

impl Collection {
    pub fn is_locked(&self) -> bool {
        self.master_key.is_none()
    }
}

#[derive(Clone)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub attributes: HashMap<String, String>,
    pub content_type: String,
    pub secret: Vec<u8>,
    pub created: u64,
    pub modified: u64,
}

impl From<StoredItem> for Item {
    fn from(s: StoredItem) -> Self {
        Self {
            id: s.id,
            label: s.label,
            attributes: s.attributes,
            content_type: s.content_type,
            secret: s.secret,
            created: s.created,
            modified: s.modified,
        }
    }
}

impl From<&Item> for StoredItem {
    fn from(i: &Item) -> Self {
        Self {
            id: i.id.clone(),
            label: i.label.clone(),
            attributes: i.attributes.clone(),
            content_type: i.content_type.clone(),
            secret: i.secret.clone(),
            created: i.created,
            modified: i.modified,
        }
    }
}

/// A live D-Bus session — established via `Service.OpenSession`. Determines
/// whether responses carrying secrets are sent in cleartext or encrypted
/// under a DH-derived AES-128-CBC key.
pub struct Session {
    pub algorithm: SessionAlgo,
}

pub enum SessionAlgo {
    Plain,
    /// 16-byte AES-128 key shared with the client.
    DhAesCbc {
        key: [u8; 16],
    },
}

/// A live D-Bus prompt object — created by Service.Unlock / Service.Lock /
/// CreateCollection / Delete and walked through Prompt.Prompt / Dismiss.
pub struct Prompt {
    pub kind: PromptKind,
    pub completed: bool,
}

pub enum PromptKind {
    /// Unlock one or more collections. Result variant: `ao` of unlocked paths.
    Unlock { collection_ids: Vec<String> },
    /// Create a new collection. Result variant: `o` of the new path.
    CreateCollection {
        id: String,
        label: String,
        alias: Option<String>,
    },
    /// Delete a collection. Result variant: `s` empty string (per spec).
    DeleteCollection { collection_id: String },
    /// Delete an item. Result variant: `s` empty string.
    DeleteItem {
        collection_id: String,
        item_id: String,
    },
}
