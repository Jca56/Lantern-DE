use std::ffi::c_void;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use pam_client::conv_mock::{Conversation, LogEntry};
use pam_client::{Context, Flag};

/// PAM service name. Ships as /etc/pam.d/lntrn-lockscreen.
const SERVICE: &str = "lntrn-lockscreen";

/// Longest PAM message shown under the field; anything more is cut.
const MAX_MESSAGE_CHARS: usize = 120;

/// How a password check came out.
pub enum Verdict {
    Granted,
    /// Refused, with the line to show the user.
    Denied(String),
}

/// Resolve the current user's login name from the real UID.
pub fn current_username() -> Option<String> {
    unsafe {
        let uid = libc::getuid();
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return std::env::var("USER").ok();
        }
        let name = std::ffi::CStr::from_ptr((*pw).pw_name);
        name.to_str().ok().map(|s| s.to_string())
    }
}

/// Overwrite a string's whole buffer with zeros and empty it. Volatile
/// writes, so the optimizer can't drop them as dead stores.
pub fn wipe(s: &mut String) {
    // SAFETY: the bytes are cleared first, so the string stays valid UTF-8.
    let bytes = unsafe { s.as_mut_vec() };
    bytes.clear();
    for byte in bytes.spare_capacity_mut() {
        unsafe { std::ptr::write_volatile(byte.as_mut_ptr(), 0) };
    }
}

/// Verify the password for the current user via PAM: both authentication and
/// account management have to pass. Consumes the password and wipes it.
fn verify(username: &str, password: String) -> Verdict {
    if password.is_empty() {
        return Verdict::Denied("Incorrect password".into());
    }

    // Moved, not copied: the conversation's copy is the only one, wiped below.
    let conv = Conversation::with_credentials(username.to_string(), password);
    let mut ctx = match Context::new(SERVICE, Some(username), conv) {
        Ok(c) => c,
        Err(_) => return Verdict::Denied("Can't reach the login service".into()),
    };

    let mut result = ctx.authenticate(Flag::DISALLOW_NULL_AUTHTOK);
    if result.is_ok() {
        // Account validity (expiry, locked, etc.).
        result = ctx.acct_mgmt(Flag::NONE);
    }
    let verdict = match result {
        Ok(()) => Verdict::Granted,
        Err(_) => Verdict::Denied(denial_message(ctx.conversation())),
    };
    wipe(&mut ctx.conversation_mut().password);
    verdict
}

/// What to tell the user after a refusal. A plain wrong password makes PAM
/// say nothing; when a module does speak up (faillock: "The account is locked
/// due to 3 failed logins"), that is the real reason and worth showing.
fn denial_message(conv: &Conversation) -> String {
    let said: Vec<String> = conv
        .log
        .iter()
        .map(|entry| match entry {
            LogEntry::Info(msg) | LogEntry::Error(msg) => msg.to_string_lossy().into_owned(),
        })
        .collect();
    tidy_message(&said.join(" "))
}

/// One short line out of whatever PAM said, or the stock message for nothing.
fn tidy_message(said: &str) -> String {
    let line = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        "Incorrect password".into()
    } else {
        line.chars().take(MAX_MESSAGE_CHARS).collect()
    }
}

/// Runs password checks off the main thread and wakes it when one finishes.
///
/// PAM blocks (a wrong password alone costs its ~2 s fail delay), so each check
/// gets a worker. The verdict comes back over a channel, and the worker pokes
/// an eventfd the main loop has in its `poll` set, so an answer is acted on
/// the moment it exists instead of at the next timer tick.
pub struct Authenticator {
    username: String,
    tx: Sender<Verdict>,
    rx: Receiver<Verdict>,
    wake: Arc<OwnedFd>,
}

impl Authenticator {
    pub fn new() -> std::io::Result<Self> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        Ok(Self {
            username: current_username().unwrap_or_default(),
            tx,
            rx,
            wake: Arc::new(unsafe { OwnedFd::from_raw_fd(fd) }),
        })
    }

    /// Readable whenever a verdict is waiting. Put it in the `poll` set.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    /// Start checking `password` in the background.
    pub fn submit(&self, password: String) {
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let user = self.username.clone();
        std::thread::spawn(move || {
            let verdict = verify(&user, password);
            // Verdict first, then the wake-up: whoever it wakes finds it there.
            let _ = tx.send(verdict);
            let one: u64 = 1;
            unsafe { libc::write(wake.as_raw_fd(), (&one as *const u64).cast::<c_void>(), 8) };
        });
    }

    /// The verdict of a finished check, if one is waiting.
    pub fn poll(&self) -> Option<Verdict> {
        let mut count: u64 = 0;
        unsafe { libc::read(self.wake.as_raw_fd(), (&mut count as *mut u64).cast::<c_void>(), 8) };
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiping_zeroes_the_whole_buffer() {
        let mut secret = String::with_capacity(32);
        secret.push_str("hunter2");
        let (ptr, cap) = (secret.as_ptr(), secret.capacity());

        wipe(&mut secret);

        assert!(secret.is_empty());
        // The allocation is still the string's own; only its contents changed.
        let raw = unsafe { std::slice::from_raw_parts(ptr, cap) };
        assert!(raw.iter().all(|&b| b == 0));
    }

    #[test]
    fn silence_from_pam_means_a_wrong_password() {
        assert_eq!(tidy_message(""), "Incorrect password");
        assert_eq!(tidy_message("  \n "), "Incorrect password");
    }

    #[test]
    fn what_pam_says_becomes_one_line() {
        let said = "The account is locked due to 3 failed logins.\n (10 minutes left to unlock)";
        assert_eq!(
            tidy_message(said),
            "The account is locked due to 3 failed logins. (10 minutes left to unlock)"
        );
        assert_eq!(tidy_message(&"x".repeat(500)).chars().count(), MAX_MESSAGE_CHARS);
    }

    #[test]
    fn a_verdict_wakes_the_poll_set() {
        let auth = Authenticator::new().unwrap();
        let mut pfd = libc::pollfd {
            fd: auth.fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(unsafe { libc::poll(&mut pfd, 1, 0) }, 0, "nothing pending yet");

        // An empty password is refused before PAM is ever involved.
        auth.submit(String::new());
        assert_eq!(unsafe { libc::poll(&mut pfd, 1, 5000) }, 1, "worker never woke us");
        assert!(matches!(auth.poll(), Some(Verdict::Denied(_))));

        // Taking the verdict drains the wake-up with it.
        assert_eq!(unsafe { libc::poll(&mut pfd, 1, 0) }, 0);
        assert!(auth.poll().is_none());
    }
}
