//! Versioned on-disk file format for encrypted collections.
//!
//! ```text
//! [magic: "LNKC" 4 bytes]
//! [version: u16 LE]
//! [kdf_id: u8]   // 0x01 = Argon2id
//! [reserved: u8]
//! [salt: 16 bytes]
//! [m_cost_kib: u32 LE]
//! [t_cost:     u32 LE]
//! [p_cost:     u32 LE]
//! [nonce: 12 bytes]            // AES-GCM nonce
//! [ciphertext_len: u32 LE]     // ciphertext bytes (incl. 16-byte tag)
//! [ciphertext: variable]
//! ```

use super::crypto::{KdfParams, NONCE_LEN, SALT_LEN};

pub const FILE_MAGIC: &[u8; 4] = b"LNKC";
pub const FORMAT_VERSION: u16 = 1;
pub const KDF_ARGON2ID: u8 = 1;

const HEADER_LEN: usize =
    4 + 2 + 1 + 1 + SALT_LEN + 4 + 4 + 4 + NONCE_LEN + 4;

#[derive(Debug)]
pub enum Error {
    BadMagic,
    UnsupportedVersion(u16),
    UnsupportedKdf(u8),
    Truncated,
    OversizedCiphertext,
}

pub struct FileBlob {
    pub magic: [u8; 4],
    pub version: u16,
    pub kdf_id: u8,
    pub salt: [u8; SALT_LEN],
    pub kdf_params: KdfParams,
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

impl FileBlob {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.ciphertext.len());
        out.extend_from_slice(&self.magic);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.push(self.kdf_id);
        out.push(0); // reserved
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&self.kdf_params.m_cost_kib.to_le_bytes());
        out.extend_from_slice(&self.kdf_params.t_cost.to_le_bytes());
        out.extend_from_slice(&self.kdf_params.p_cost.to_le_bytes());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&(self.ciphertext.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.ciphertext);
        out
    }

    pub fn parse(buf: &[u8]) -> Result<Self, Error> {
        if buf.len() < HEADER_LEN { return Err(Error::Truncated); }
        let mut cur = 0;

        let mut magic = [0u8; 4];
        magic.copy_from_slice(&buf[cur..cur+4]); cur += 4;
        if &magic != FILE_MAGIC { return Err(Error::BadMagic); }

        let version = u16::from_le_bytes([buf[cur], buf[cur+1]]); cur += 2;
        if version != FORMAT_VERSION { return Err(Error::UnsupportedVersion(version)); }

        let kdf_id = buf[cur]; cur += 1;
        if kdf_id != KDF_ARGON2ID { return Err(Error::UnsupportedKdf(kdf_id)); }
        cur += 1; // reserved

        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&buf[cur..cur+SALT_LEN]); cur += SALT_LEN;

        let m_cost_kib = u32::from_le_bytes(buf[cur..cur+4].try_into().unwrap()); cur += 4;
        let t_cost     = u32::from_le_bytes(buf[cur..cur+4].try_into().unwrap()); cur += 4;
        let p_cost     = u32::from_le_bytes(buf[cur..cur+4].try_into().unwrap()); cur += 4;

        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&buf[cur..cur+NONCE_LEN]); cur += NONCE_LEN;

        let ct_len = u32::from_le_bytes(buf[cur..cur+4].try_into().unwrap()) as usize;
        cur += 4;

        // Sanity cap: refuse > 64 MiB ciphertext.
        if ct_len > 64 * 1024 * 1024 { return Err(Error::OversizedCiphertext); }
        if buf.len() < cur + ct_len { return Err(Error::Truncated); }

        let ciphertext = buf[cur..cur+ct_len].to_vec();

        Ok(Self {
            magic, version, kdf_id, salt,
            kdf_params: KdfParams { m_cost_kib, t_cost, p_cost },
            nonce, ciphertext,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_parse_roundtrip() {
        let blob = FileBlob {
            magic: *FILE_MAGIC,
            version: FORMAT_VERSION,
            kdf_id: KDF_ARGON2ID,
            salt: [7u8; SALT_LEN],
            kdf_params: KdfParams { m_cost_kib: 4096, t_cost: 2, p_cost: 1 },
            nonce: [9u8; NONCE_LEN],
            ciphertext: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let bytes = blob.encode();
        let back = FileBlob::parse(&bytes).unwrap();
        assert_eq!(back.salt, blob.salt);
        assert_eq!(back.nonce, blob.nonce);
        assert_eq!(back.ciphertext, blob.ciphertext);
        assert_eq!(back.kdf_params.m_cost_kib, 4096);
    }

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = FileBlob {
            magic: *FILE_MAGIC,
            version: FORMAT_VERSION,
            kdf_id: KDF_ARGON2ID,
            salt: [0u8; SALT_LEN],
            kdf_params: KdfParams::default(),
            nonce: [0u8; NONCE_LEN],
            ciphertext: vec![],
        }.encode();
        bytes[0] = b'X';
        assert!(matches!(FileBlob::parse(&bytes), Err(Error::BadMagic)));
    }

    #[test]
    fn truncated_rejected() {
        let bytes = vec![0u8; 8];
        assert!(matches!(FileBlob::parse(&bytes), Err(Error::Truncated)));
    }
}
