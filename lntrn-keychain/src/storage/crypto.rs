//! KDF + AEAD primitives.
//!
//! - Argon2id derives a 32-byte master key from a passphrase.
//! - AES-256-GCM encrypts the JSON-serialized collection body.
//! - 12-byte random nonce per encryption (never reused with same key).

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng as AeadOsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const SALT_LEN: usize = 16;
pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

#[derive(Debug)]
pub enum Error {
    Argon2,
    Aead,
}

/// Argon2id tuning. Conservative defaults — ~64MB / 3 passes / 1 thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 64 * 1024, // 64 MiB
            t_cost: 3,
            p_cost: 1,
        }
    }
}

/// Master key + the parameters needed to re-derive it.
/// Wiped from memory on drop.
#[derive(ZeroizeOnDrop)]
pub struct MasterKey {
    pub key: [u8; KEY_LEN],
    #[zeroize(skip)]
    pub salt: [u8; SALT_LEN],
    #[zeroize(skip)]
    pub params: KdfParams,
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MasterKey")
            .field("key", &"<redacted>")
            .field("salt", &self.salt)
            .field("params", &self.params)
            .finish()
    }
}

pub fn random_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

pub fn derive_key(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    params: &KdfParams,
) -> Result<MasterKey, Error> {
    let p = Params::new(
        params.m_cost_kib,
        params.t_cost,
        params.p_cost,
        Some(KEY_LEN),
    )
    .map_err(|_| Error::Argon2)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|_| Error::Argon2)?;
    Ok(MasterKey {
        key,
        salt: *salt,
        params: params.clone(),
    })
}

/// Encrypt plaintext under the master key. Returns (nonce, ciphertext_with_tag).
pub fn encrypt(key: &MasterKey, plaintext: &[u8]) -> Result<([u8; NONCE_LEN], Vec<u8>), Error> {
    let k = Key::<Aes256Gcm>::from_slice(&key.key);
    let cipher = Aes256Gcm::new(k);
    let nonce_arr = Aes256Gcm::generate_nonce(&mut AeadOsRng);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_arr.as_slice());
    let ct = cipher
        .encrypt(&nonce_arr, plaintext)
        .map_err(|_| Error::Aead)?;
    Ok((nonce, ct))
}

/// Decrypt + verify ciphertext under the master key. Returns plaintext.
pub fn decrypt(
    key: &MasterKey,
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, Error> {
    let k = Key::<Aes256Gcm>::from_slice(&key.key);
    let cipher = Aes256Gcm::new(k);
    let nonce_obj = Nonce::from_slice(nonce);
    let mut pt = cipher
        .decrypt(nonce_obj, ciphertext)
        .map_err(|_| Error::Aead)?;
    let out = pt.clone();
    pt.zeroize();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aead_roundtrip() {
        let salt = random_salt();
        let key = derive_key("hunter2", &salt, &KdfParams::default()).unwrap();
        let pt = b"hello world, this is a secret".to_vec();
        let (nonce, ct) = encrypt(&key, &pt).unwrap();
        let back = decrypt(&key, &nonce, &ct).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn aead_rejects_tamper() {
        let salt = random_salt();
        let key = derive_key("hunter2", &salt, &KdfParams::default()).unwrap();
        let (nonce, mut ct) = encrypt(&key, b"data").unwrap();
        ct[0] ^= 0x01;
        assert!(decrypt(&key, &nonce, &ct).is_err());
    }

    #[test]
    fn aead_rejects_wrong_key() {
        let salt = random_salt();
        let k1 = derive_key("right", &salt, &KdfParams::default()).unwrap();
        let k2 = derive_key("wrong", &salt, &KdfParams::default()).unwrap();
        let (nonce, ct) = encrypt(&k1, b"data").unwrap();
        assert!(decrypt(&k2, &nonce, &ct).is_err());
    }
}
