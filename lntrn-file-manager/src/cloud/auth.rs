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

    let resp = match ureq::post(&url).send_json(body) {
        Ok(r) => r,
        Err(ureq::Error::Status(_code, r)) => {
            let body = r.into_string().unwrap_or_default();
            anyhow::bail!("sign-in failed: {}", map_err_body(&body));
        }
        Err(e) => anyhow::bail!("sign-in transport error: {e}"),
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
/// session is mutated and re-saved to disk. On failure (e.g. refresh token
/// revoked) the cached session is wiped.
pub fn refresh(cfg: &CloudConfig, session: &mut Session) -> anyhow::Result<()> {
    let url = format!(
        "https://securetoken.googleapis.com/v1/token?key={}",
        cfg.api_key
    );

    let resp = match ureq::post(&url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&format!(
            "grant_type=refresh_token&refresh_token={}",
            urlencoding::encode(&session.refresh_token)
        )) {
        Ok(r) => r,
        Err(ureq::Error::Status(_code, r)) => {
            let body = r.into_string().unwrap_or_default();
            // Refresh token revoked / invalid — drop the cached session.
            Session::forget();
            anyhow::bail!("token refresh failed: {}", map_err_body(&body));
        }
        Err(e) => anyhow::bail!("token refresh transport error: {e}"),
    };

    let parsed: RefreshResponse = resp.into_json()?;
    session.id_token = parsed.id_token;
    session.refresh_token = parsed.refresh_token;
    session.uid = parsed.user_id;
    session.expires_at = now_secs() + parse_expires_in(&parsed.expires_in);
    session.save()?;
    Ok(())
}

/// Ensure the session's id_token is fresh; refresh if not.
pub fn ensure_fresh(cfg: &CloudConfig, session: &mut Session) -> anyhow::Result<()> {
    if session.is_fresh() {
        return Ok(());
    }
    refresh(cfg, session)
}
