//! Background SHA-256 computation for the Properties dialog.
//!
//! Spawned lazily when the user opens the Checksum section (never on
//! dialog open — hashing a 50GB ISO nobody asked about would be rude).
//! The dialog polls `get()` each frame and shows "Computing…" until the
//! result lands.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

pub struct ChecksumJob {
    result: Arc<Mutex<Option<String>>>,
    /// Set when the dialog goes away; the worker stops at its next chunk
    /// instead of hashing the rest of a 50GB file nobody is waiting for.
    cancel: Arc<AtomicBool>,
}

impl ChecksumJob {
    pub fn spawn(path: PathBuf) -> Self {
        let result = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let slot = Arc::clone(&result);
        let stop = Arc::clone(&cancel);
        std::thread::spawn(move || {
            let hex = compute_sha256(&path, &stop).unwrap_or_else(|| "Unreadable".to_string());
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(hex);
        });
        Self { result, cancel }
    }

    /// `None` while still computing.
    pub fn get(&self) -> Option<String> {
        self.result.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Still hashing — the dialog needs frames to notice the result.
    pub fn is_running(&self) -> bool {
        self.get().is_none()
    }
}

impl Drop for ChecksumJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn compute_sha256(path: &std::path::Path, cancel: &AtomicBool) -> Option<String> {
    use std::io::Read;
    // Regular files only: a FIFO blocks forever on open, a device or
    // /dev/zero never reaches end of file.
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
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
