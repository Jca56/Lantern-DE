//! Scratch directories for tests, so none of them goes near `~/.lantern`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct TestDir(PathBuf);

impl TestDir {
    /// A new empty directory, removed again when this is dropped. Straight
    /// under `/tmp` whatever `$TMPDIR` says: a Unix socket's path has to fit
    /// in 108 bytes.
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = PathBuf::from(format!("/tmp/lntrn-session-test-{}-{n}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TestDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
