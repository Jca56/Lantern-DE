use pam_client::conv_mock::Conversation;
use pam_client::{Context, Flag};

/// PAM service name. Ships as /etc/pam.d/lntrn-lockscreen.
const SERVICE: &str = "lntrn-lockscreen";

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

/// Verify the given password for the current user via PAM.
///
/// Returns `true` only when both authentication and account management
/// succeed. Empty passwords are rejected up front.
pub fn verify(username: &str, password: &str) -> bool {
    if password.is_empty() {
        return false;
    }

    let conv = Conversation::with_credentials(username.to_string(), password.to_string());
    let mut ctx = match Context::new(SERVICE, Some(username), conv) {
        Ok(c) => c,
        Err(_) => return false,
    };

    if ctx.authenticate(Flag::DISALLOW_NULL_AUTHTOK).is_err() {
        return false;
    }
    // Account validity (expiry, locked, etc.).
    ctx.acct_mgmt(Flag::NONE).is_ok()
}
