//! Flush in-memory collection state back to disk.
//!
//! Every mutation through Collection/Item methods funnels through here so
//! a crash can lose at most the call that didn't return yet.

use crate::storage;
use crate::storage::crypto::MasterKey;
use crate::storage::{CollectionMeta, DecryptedCollection, Item as StoredItem};

use super::state::Collection;

/// Encrypt + atomically write the collection to its on-disk file.
pub fn persist_collection(coll: &Collection, key: &MasterKey) -> Result<(), storage::Error> {
    let mut items: Vec<StoredItem> = coll.items.values().map(StoredItem::from).collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    let dec = DecryptedCollection {
        meta: CollectionMeta {
            label: coll.label.clone(),
            created: coll.created,
            modified: coll.modified,
        },
        items,
    };
    storage::save(&coll.id, &dec, key)
}

/// Discover collections on disk + insert locked stubs into state.
pub fn discover_locked_collections(state: &mut super::state::ServiceState) {
    for id in storage::list_collection_ids() {
        if state.collections.contains_key(&id) {
            continue;
        }
        state.collections.insert(
            id.clone(),
            Collection {
                id,
                label: String::new(),
                created: 0,
                modified: 0,
                items: Default::default(),
                master_key: None,
            },
        );
    }
}
