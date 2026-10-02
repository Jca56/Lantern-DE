// Firebase Auth REST. Two endpoints:
//
//   POST https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword?key={api_key}
//        body: { "email", "password", "returnSecureToken": true }
//        resp: { "idToken", "refreshToken", "expiresIn", "localId" (=uid), "email" }
//
//   POST https://securetoken.googleapis.com/v1/token?key={api_key}
//        body: { "grant_type": "refresh_token", "refresh_token": "..." }  (form-encoded)
//        resp: { "id_token", "refresh_token", "expires_in", "user_id" }
//
// Both return expires_in as a string of seconds. We convert to an absolute
// Unix-seconds expiry so Session.is_fresh() works.

use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CloudConfig, Session};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
struct SignInResponse {
    #[serde(rename = "idToken")]
    id_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
    #[serde(rename = "expiresIn")]
    expires_in: String,
    #[serde(rename = "localId")]
    local_id: String,
    email: String,
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    id_token: String,
    refresh_token: String,
    expires_in: String,
    user_id: String,
}

#[derive(Debug, Deserialize)]
struct FirebaseError {
    error: FirebaseErrorDetail,
}

#[derive(Debug, Deserialize)]
struct FirebaseErrorDetail {
    message: String,
}

fn parse_expires_in(s: &str) -> u64 {
    s.parse::<u64>().unwrap_or(3600)
}

/// ureq prints the request URL in a transport error, and ours carries the API
/// key as a query parameter. Keep the reason, drop the URL, so the key never
/// reaches the log or the login dialog.
fn transport_msg(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Transport(t) => {
            let mut out = match t.message() {
                Some(m) => format!("{}: {m}", t.kind()),
                None => t.kind().to_string(),
            };
            // The real cause ("timed out", "Connection refused") lives in
            // the source; it carries no URL.
            if let Some(source) = std::error::Error::source(t) {
                out.push_str(&format!(": {source}"));
            }
            out
        }
        ureq::Error::Status(code, _) => format!("status code {code}"),
    }
}

/// Firebase's answers that mean the refresh token itself is no good. Anything
/// else (5xx, 429, a proxy hiccup) is transient and must not sign the user out.
const REVOKED: &[&str] = &[
    "TOKEN_EXPIRED",
    "INVALID_REFRESH_TOKEN",
    "USER_DISABLED",
    "USER_NOT_FOUND",
    "INVALID_GRANT",
];

/// Why sync has to stop until the user signs in again. Travels inside the
/// `anyhow::Error` of whatever request needed the token (`stop_reason` finds
/// it again), so the sync loop can stop instead of retrying a dead session
/// every 30 seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStop {
    /// Firebase rejected the refresh token for good (password changed,
    /// account disabled or deleted).
    Revoked,
    /// The session file was removed, or another account signed in, while
    /// this process still held the old session.
    SignedOut,
}

impl std::fmt::Display for AuthStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AuthStop::Revoked => "the cloud session is no longer valid, sign in again",
            AuthStop::SignedOut => "signed out of the cloud",
        })
    }
}

impl std::error::Error for AuthStop {}

/// The `AuthStop` behind an error, if that is what it is.
pub fn stop_reason(e: &anyhow::Error) -> Option<AuthStop> {
    e.chain().find_map(|c| c.downcast_ref::<AuthStop>()).copied()
}

fn is_revocation(code: u16, msg: &str) -> bool {
    matches!(code, 400 | 401 | 403) && REVOKED.iter().any(|m| msg.contains(m))
}

fn map_err_body(body: &str) -> String {
    match serde_json::from_str::<FirebaseError>(body) {
        Ok(e) => e.error.message,
        Err(_) => body.to_string(),
    }
}

/// Sign in with email + password. Returns a fresh `Session` (saved to disk).
pub fn sign_in(cfg: &CloudConfig, email: &str, password: &str) -> anyhow::Result<Session> {
    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword?key={}",
        cfg.api_key
    );
    let body = serde_json::json!({
        "email": email,
        "password": password,
        "returnSecureToken": true,
    });

    let resp = match super::http::agent().post(&url).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(_code, r)) => {
            let body = r.into_string().unwrap_or_default();
            anyhow::bail!("sign-in failed: {}", map_err_body(&body));
        }
        Err(e) => anyhow::bail!("sign-in transport error: {}", transport_msg(&e)),
    };

    let parsed: SignInResponse = resp.into_json()?;
    let session = Session {
        uid: parsed.local_id,
        email: parsed.email,
        id_token: parsed.id_token,
        refresh_token: parsed.refresh_token,
        expires_at: now_secs() + parse_expires_in(&parsed.expires_in),
    };
    session.save()?;
    Ok(session)
}

/// Refresh `session.id_token` in place using its refresh_token. On success the
/// session is mutated and written back, unless the user signed out meanwhile
/// (then nothing is written and the error is `AuthStop::SignedOut`). When
/// Firebase says the refresh token itself is dead, the cached session is
/// removed and the error is `AuthStop::Revoked`. Any other failure is
/// transient and leaves everything as it was.
pub fn refresh(cfg: &CloudConfig, session: &mut Session) -> anyhow::Result<()> {
    // Signed out already: do not even send the refresh token.
    if !session.is_current() {
        return Err(anyhow::Error::new(AuthStop::SignedOut));
    }
    let url = format!(
        "https://securetoken.googleapis.com/v1/token?key={}",
        cfg.api_key
    );

    let resp = match super::http::agent()
        .post(&url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&format!(
            "grant_type=refresh_token&refresh_token={}",
            urlencoding::encode(&session.refresh_token)
        )) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            let msg = map_err_body(&body);
            if is_revocation(code, &msg) {
                // Drop the cached session so the next launch asks for a
                // sign-in, but only if it is still this dead one.
                Session::forget_revoked(&session.refresh_token);
                super::log_line(&format!("token refresh refused for good: {msg}"));
                return Err(anyhow::Error::new(AuthStop::Revoked));
            }
            anyhow::bail!("token refresh failed: status code {code}: {msg}");
        }
        Err(e) => anyhow::bail!("token refresh transport error: {}", transport_msg(&e)),
    };

    let parsed: RefreshResponse = resp.into_json()?;
    session.id_token = parsed.id_token;
    session.refresh_token = parsed.refresh_token;
    session.uid = parsed.user_id;
    session.expires_at = now_secs() + parse_expires_in(&parsed.expires_in);
    // Never re-create a session file that was removed on purpose: a sign-out
    // during the request above stays a sign-out.
    if !session.save_refreshed()? {
        return Err(anyhow::Error::new(AuthStop::SignedOut));
    }
    Ok(())
}

/// Ensure the session's id_token is fresh; refresh if not.
pub fn ensure_fresh(cfg: &CloudConfig, session: &mut Session) -> anyhow::Result<()> {
    if session.is_fresh() {
        return Ok(());
    }
    refresh(cfg, session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_dead_refresh_token_counts_as_revoked() {
        assert!(is_revocation(400, "TOKEN_EXPIRED"));
        assert!(is_revocation(400, "INVALID_REFRESH_TOKEN"));
        assert!(is_revocation(403, "USER_DISABLED"));
        // Quota, server trouble, a proxy page: transient, keep the session.
        assert!(!is_revocation(429, "QUOTA_EXCEEDED"));
        assert!(!is_revocation(500, "INTERNAL"));
        assert!(!is_revocation(503, "TOKEN_EXPIRED"));
        assert!(!is_revocation(400, "API key not valid"));
    }

    #[test]
    fn the_stop_reason_survives_added_context() {
        let e = anyhow::Error::new(AuthStop::Revoked).context("uploading a.txt");
        assert_eq!(stop_reason(&e), Some(AuthStop::Revoked));
        let plain = anyhow::anyhow!("GET https://x: status code 500");
        assert_eq!(stop_reason(&plain), None);
    }
}
