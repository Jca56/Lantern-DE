//! Background SHA-256 computation for the Properties dialog.
//!
//! Spawned lazily when the user opens the Checksum section (never on
//! dialog open — hashing a 50GB ISO nobody asked about would be rude).
//! The dialog polls `get()` each frame and shows "Computing…" until the
//! result lands.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

pub struct ChecksumJob {
    result: Arc<Mutex<Option<String>>>,
}

impl ChecksumJob {
    pub fn spawn(path: PathBuf) -> Self {
        let result = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&result);
        std::thread::spawn(move || {
            let hex = compute_sha256(&path).unwrap_or_else(|| "Unreadable".to_string());
            *slot.lock().unwrap() = Some(hex);
        });
        Self { result }
    }

    /// `None` while still computing.
    pub fn get(&self) -> Option<String> {
        self.result.lock().unwrap().clone()
    }
}

fn compute_sha256(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Some(hex)
}
