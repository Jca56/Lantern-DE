use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// sha256 of a file, lowercase hex. Streams the file so big blobs don't blow up RAM.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish())
}

/// sha256 of a byte string, lowercase hex.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// sha256 of bytes as they go by: what a transfer hashes while it streams.
#[derive(Clone, Default)]
pub struct Hasher(Sha256);

impl Hasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    /// Lowercase hex of everything fed in.
    pub fn finish(self) -> String {
        hex(&self.0.finalize())
    }
}

/// Is `s` what a sha256 looks like here: 64 lowercase hex digits?
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_and_one_shot_agree() {
        let mut h = Hasher::new();
        h.update(b"hello ");
        h.update(b"");
        h.update(b"world");
        let sha = h.finish();
        assert_eq!(sha, sha256_bytes(b"hello world"));
        assert_eq!(
            sha,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert!(is_sha256_hex(&sha));
        assert!(!is_sha256_hex(&sha[1..]));
        assert!(!is_sha256_hex(&sha.to_uppercase()));
        assert!(!is_sha256_hex(""));
    }
}
