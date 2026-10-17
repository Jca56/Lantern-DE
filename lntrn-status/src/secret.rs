//! Secrets held only as long as they are needed: what we hold of one is
//! overwritten with zeros when it goes. (Copies made on the way out, by
//! the bus library or the kernel, are theirs to drop.)

use std::sync::atomic::{Ordering, compiler_fence};

/// A password on its way to the daemon that asks for it.
pub struct Secret(String);

impl Secret {
    pub fn new(text: String) -> Secret {
        Secret(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Overwrite `text` with zeros, and leave it empty.
pub fn wipe(text: &mut String) {
    wipe_bytes(&mut std::mem::take(text).into_bytes());
}

pub(crate) fn wipe_bytes(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        // SAFETY: `b` is a byte we hold; written in place, and volatile so
        // the write isn't dropped as one nothing reads.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wiped_password_is_gone_and_a_secret_never_prints() {
        let mut typed = "hunter22".to_owned();
        wipe(&mut typed);
        assert!(typed.is_empty());
        let mut bytes = *b"hunter22";
        wipe_bytes(&mut bytes);
        assert_eq!(bytes, [0; 8]);
        assert_eq!(format!("{:?}", Secret::new("hunter22".into())), "Secret(…)");
    }
}
