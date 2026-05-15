//! Diffie-Hellman session crypto for `dh-ietf1024-sha256-aes128-cbc-pkcs7`.
//!
//! Wire details: RFC 2409 §6.2 (Oakley MODP-1024), HKDF-SHA256 (RFC 5869)
//! with **empty salt and empty info**, then AES-128-CBC with PKCS7 padding.
//!
//! gnome-keyring sends/receives:
//! - server pub key in `OpenSession` reply (variant of `ay`)
//! - per-message random 16-byte IV in the secret's `parameters` field

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hkdf::Hkdf;
use num_bigint::BigUint;
use num_traits::Num;
use rand::RngCore;
use sha2::Sha256;
use zeroize::Zeroize;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// RFC 2409 §6.2 — 1024-bit MODP group prime, hex.
const RFC2409_PRIME_HEX: &str =
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E08\
     8A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B\
     302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9\
     A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE6\
     49286651ECE65381FFFFFFFFFFFFFFFF";

/// Generator g = 2.
fn g() -> BigUint {
    BigUint::from(2u32)
}

fn p() -> BigUint {
    let trimmed: String = RFC2409_PRIME_HEX.chars().filter(|c| !c.is_whitespace()).collect();
    BigUint::from_str_radix(&trimmed, 16).expect("hardcoded prime parses")
}

#[derive(Debug)]
pub enum Error {
    BadInput,
    AeadFailure,
}

/// Server-side DH state. Generated fresh per `OpenSession` call.
pub struct ServerSecret {
    private: BigUint,
    pub public: Vec<u8>, // big-endian, 128 bytes
}

/// Generate a fresh server private key + corresponding public key.
pub fn generate_server_secret() -> ServerSecret {
    // Private exponent: a 1024-bit value < p. Per the Secret Service spec
    // gnome-keyring uses 1024-bit randoms — we match.
    let mut bytes = [0u8; 128];
    rand::thread_rng().fill_bytes(&mut bytes);
    let private = BigUint::from_bytes_be(&bytes);
    bytes.zeroize();

    let public_big = g().modpow(&private, &p());
    let public = pad_to_128(public_big.to_bytes_be());

    ServerSecret { private, public }
}

/// Compute shared secret from peer's public key + derive a 16-byte AES key.
///
/// Returns the 16-byte symmetric key (HKDF-SHA256 with empty salt/info).
pub fn derive_shared_key(server: &ServerSecret, peer_pub: &[u8]) -> Result<[u8; 16], Error> {
    if peer_pub.is_empty() || peer_pub.len() > 256 {
        return Err(Error::BadInput);
    }
    let peer = BigUint::from_bytes_be(peer_pub);
    let p_val = p();
    if peer <= BigUint::from(1u32) || peer >= p_val.clone() - BigUint::from(1u32) {
        return Err(Error::BadInput);
    }

    let shared = peer.modpow(&server.private, &p_val);
    let mut ikm = pad_to_128(shared.to_bytes_be());

    let hkdf = Hkdf::<Sha256>::new(None, &ikm); // empty salt
    let mut okm = [0u8; 16];
    hkdf.expand(&[], &mut okm).map_err(|_| Error::AeadFailure)?;
    ikm.zeroize();
    Ok(okm)
}

/// Left-pad a big-endian byte array with zeros to exactly 128 bytes (1024 bits).
fn pad_to_128(mut v: Vec<u8>) -> Vec<u8> {
    if v.len() >= 128 {
        // Take the rightmost 128 bytes if somehow longer (shouldn't happen
        // mod p where p is 1024 bits, but be defensive).
        v.drain(..v.len() - 128);
        return v;
    }
    let mut out = vec![0u8; 128 - v.len()];
    out.append(&mut v);
    out
}

/// Encrypt + PKCS7-pad `plaintext` under `key` with `iv`. Returns ciphertext.
pub fn encrypt(key: &[u8; 16], iv: &[u8; 16], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes128CbcEnc::new(key.into(), iv.into());
    cipher.encrypt_padded_vec_mut::<Pkcs7>(plaintext)
}

/// Decrypt + strip PKCS7 padding. Returns plaintext or AeadFailure on bad pad.
pub fn decrypt(key: &[u8; 16], iv: &[u8; 16], ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
    let cipher = Aes128CbcDec::new(key.into(), iv.into());
    cipher
        .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| Error::AeadFailure)
}

/// Generate a random 16-byte IV.
pub fn random_iv() -> [u8; 16] {
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut iv);
    iv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dh_roundtrip_matches_both_sides() {
        // Simulate a client doing its own DH and verify we land on the same key.
        let client = generate_server_secret();
        let server = generate_server_secret();
        let k_server = derive_shared_key(&server, &client.public).unwrap();
        let k_client = derive_shared_key(&client, &server.public).unwrap();
        assert_eq!(k_server, k_client);
    }

    #[test]
    fn aes_cbc_roundtrip() {
        let key = [42u8; 16];
        let iv = random_iv();
        let pt = b"hunter2 with some padding-not-aligned content".to_vec();
        let ct = encrypt(&key, &iv, &pt);
        let back = decrypt(&key, &iv, &ct).unwrap();
        assert_eq!(back, pt);
    }

    #[test]
    fn rejects_trivial_peer_pub() {
        let server = generate_server_secret();
        assert!(matches!(derive_shared_key(&server, &[]), Err(Error::BadInput)));
        assert!(matches!(derive_shared_key(&server, &[0]), Err(Error::BadInput)));
        assert!(matches!(derive_shared_key(&server, &[1]), Err(Error::BadInput)));
    }
}
