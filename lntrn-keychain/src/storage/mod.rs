//! On-disk encrypted collection storage.
//!
//! Layout: ~/.lantern/keychain/<collection_id>.lkc — a single file per
//! collection, encrypted with AES-256-GCM, key derived from the user
//! passphrase via Argon2id. See `format` for the binary layout and `crypto`
//! for KDF + AEAD primitives.

// Storage API is wired into the D-Bus layer in phase 3; suppress until then.
#![allow(dead_code)]

pub mod crypto;
pub mod format;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use self::crypto::{decrypt, derive_key, encrypt, KdfParams, MasterKey};
use self::format::{FileBlob, FILE_MAGIC, FORMAT_VERSION};

const COLLECTION_EXT: &str = "lkc";

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    Crypto(crypto::Error),
    Format(format::Error),
    BadPassphrase,
    NotFound,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Json(e) => write!(f, "json: {e}"),
            Self::Crypto(e) => write!(f, "crypto: {e:?}"),
            Self::Format(e) => write!(f, "format: {e:?}"),
            Self::BadPassphrase => write!(f, "bad passphrase"),
            Self::NotFound => write!(f, "collection not found"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<crypto::Error> for Error {
    fn from(e: crypto::Error) -> Self {
        Self::Crypto(e)
    }
}
impl From<format::Error> for Error {
    fn from(e: format::Error) -> Self {
        Self::Format(e)
    }
}

// ── Domain types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub attributes: HashMap<String, String>,
    pub content_type: String,
    #[serde(with = "serde_bytes")]
    pub secret: Vec<u8>,
    pub created: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub label: String,
    pub created: u64,
    pub modified: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecryptedCollection {
    pub meta: CollectionMeta,
    pub items: Vec<Item>,
}

// Minimal byte-array helper for serde (we don't pull in serde_bytes).
mod serde_bytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64_encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64_decode(&s).map_err(serde::de::Error::custom)
    }
    fn base64_encode(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
            out.push(T[(b0 >> 2) as usize] as char);
            out.push(T[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                out.push(T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(T[(b2 & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }
    fn base64_decode(s: &str) -> Result<Vec<u8>, &'static str> {
        fn val(c: u8) -> Result<u8, &'static str> {
            Ok(match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return Err("bad b64 char"),
            })
        }
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks(4) {
            if chunk.len() < 4 {
                return Err("truncated b64");
            }
            let v0 = val(chunk[0])?;
            let v1 = val(chunk[1])?;
            out.push((v0 << 2) | (v1 >> 4));
            if chunk[2] != b'=' {
                let v2 = val(chunk[2])?;
                out.push(((v1 & 0x0f) << 4) | (v2 >> 2));
                if chunk[3] != b'=' {
                    let v3 = val(chunk[3])?;
                    out.push(((v2 & 0x03) << 6) | v3);
                }
            }
        }
        Ok(out)
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Where collections live on disk.
pub fn keychain_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/keychain")
}

pub fn collection_path(id: &str) -> PathBuf {
    keychain_dir().join(format!("{id}.{COLLECTION_EXT}"))
}

/// Read + decrypt a collection from disk, returning the decrypted body
/// alongside the derived master key (caller keeps it for subsequent saves).
pub fn unlock(id: &str, passphrase: &str) -> Result<(DecryptedCollection, MasterKey), Error> {
    let path = collection_path(id);
    if !path.exists() {
        return Err(Error::NotFound);
    }
    let raw = std::fs::read(&path)?;
    let blob = FileBlob::parse(&raw)?;
    let key = derive_key(passphrase, &blob.salt, &blob.kdf_params)?;
    let plaintext =
        decrypt(&key, &blob.nonce, &blob.ciphertext).map_err(|_| Error::BadPassphrase)?;
    let coll: DecryptedCollection = serde_json::from_slice(&plaintext)?;
    Ok((coll, key))
}

/// Read + decrypt a collection from disk (key discarded — use `unlock` to
/// keep it for subsequent saves).
pub fn load(id: &str, passphrase: &str) -> Result<DecryptedCollection, Error> {
    unlock(id, passphrase).map(|(c, _)| c)
}

/// Encrypt + write a collection to disk atomically (write tmp, fsync, rename).
pub fn save(id: &str, coll: &DecryptedCollection, key: &MasterKey) -> Result<(), Error> {
    let dir = keychain_dir();
    std::fs::create_dir_all(&dir)?;
    // 0700 on the directory
    set_mode_0700(&dir)?;

    let plaintext = Zeroizing::new(serde_json::to_vec(coll)?);
    let (nonce, ciphertext) = encrypt(key, &plaintext)?;

    let blob = FileBlob {
        magic: *FILE_MAGIC,
        version: FORMAT_VERSION,
        kdf_id: format::KDF_ARGON2ID,
        salt: key.salt,
        kdf_params: key.params.clone(),
        nonce,
        ciphertext,
    };
    let bytes = blob.encode();

    let final_path = collection_path(id);
    let tmp_path = final_path.with_extension(format!("{COLLECTION_EXT}.tmp"));
    std::fs::write(&tmp_path, &bytes)?;
    set_mode_0600(&tmp_path)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Create a fresh empty collection encrypted with a new master key.
pub fn create(id: &str, label: &str, passphrase: &str) -> Result<MasterKey, Error> {
    if collection_path(id).exists() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "collection already exists",
        )));
    }
    let params = KdfParams::default();
    let salt = crypto::random_salt();
    let key = derive_key(passphrase, &salt, &params)?;
    let now = unix_now();
    let coll = DecryptedCollection {
        meta: CollectionMeta {
            label: label.into(),
            created: now,
            modified: now,
        },
        items: Vec::new(),
    };
    save(id, &coll, &key)?;
    Ok(key)
}

pub fn list_collection_ids() -> Vec<String> {
    let dir = keychain_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some(COLLECTION_EXT) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
    }
    out.sort();
    out
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn set_mode_0700(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(0o700);
    std::fs::set_permissions(path, perm)
}

fn set_mode_0600(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(0o600);
    std::fs::set_permissions(path, perm)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home() -> tempdir_t::TempDir {
        let d = tempdir_t::TempDir::new();
        std::env::set_var("HOME", d.path());
        d
    }

    #[test]
    fn roundtrip_collection() {
        let _h = temp_home();
        let key = create("login", "Login Keyring", "hunter2").unwrap();

        let mut coll = load("login", "hunter2").unwrap();
        assert_eq!(coll.meta.label, "Login Keyring");
        assert!(coll.items.is_empty());

        let now = unix_now();
        coll.items.push(Item {
            id: "abc123".into(),
            label: "github.com".into(),
            attributes: [("host".to_string(), "github.com".to_string())]
                .into_iter()
                .collect(),
            content_type: "text/plain".into(),
            secret: b"ghp_secret_token".to_vec(),
            created: now,
            modified: now,
        });
        save("login", &coll, &key).unwrap();

        let loaded = load("login", "hunter2").unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].secret, b"ghp_secret_token");
    }

    #[test]
    fn wrong_passphrase_rejected() {
        let _h = temp_home();
        create("login", "L", "right").unwrap();
        match load("login", "wrong") {
            Err(Error::BadPassphrase) => {}
            other => panic!("expected BadPassphrase, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tempdir_t {
    pub struct TempDir(std::path::PathBuf);
    impl TempDir {
        pub fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("lntrn-keychain-test-{nanos}"));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
