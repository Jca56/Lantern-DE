use serde::Deserialize;
use crate::cloud::config::{AuthTokens, FoxDenConfig, save_config};

// ── Firebase REST Auth endpoints ─────────────────────────────────────────────

const SIGN_IN_URL: &str =
    "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword";
const REFRESH_URL: &str =
    "https://securetoken.googleapis.com/v1/token";

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignInResponse {
    id_token: String,
    refresh_token: String,
    email: String,
    local_id: String,
    expires_in: String,
}

#[derive(Deserialize)]
struct RefreshResponse {
    id_token: String,
    refresh_token: String,
    expires_in: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

// ── Sign in with email & password ────────────────────────────────────────────

pub fn sign_in(
    api_key: &str,
    email: &str,
    password: &str,
) -> Result<AuthTokens, String> {
    let url = format!("{}?key={}", SIGN_IN_URL, urlencoding::encode(api_key));

    let body = serde_json::json!({
        "email": email,
        "password": password,
        "returnSecureToken": true,
    });

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|e| format!("Network error: {}", e))?;

    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read error: {}", e))?;

    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<ErrorResponse>(&text) {
            return Err(friendly_error(&err.error.message));
        }
        return Err(format!("Auth failed ({})", status));
    }

    let data: SignInResponse =
        serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;

    let expires_in: u64 = data.expires_in.parse().unwrap_or(3600);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(AuthTokens {
        id_token: data.id_token,
        refresh_token: data.refresh_token,
        email: data.email,
        local_id: data.local_id,
        expires_at: now + expires_in,
    })
}

// ── Refresh an expired token ─────────────────────────────────────────────────

pub fn refresh_token(api_key: &str, refresh_tok: &str) -> Result<AuthTokens, String> {
    let url = format!("{}?key={}", REFRESH_URL, urlencoding::encode(api_key));

    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_tok,
    });

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .map_err(|e| format!("Network error: {}", e))?;

    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read error: {}", e))?;

    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<ErrorResponse>(&text) {
            return Err(friendly_error(&err.error.message));
        }
        return Err(format!("Refresh failed ({})", status));
    }

    let data: RefreshResponse =
        serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;

    let expires_in: u64 = data.expires_in.parse().unwrap_or(3600);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // We don't get email/local_id from refresh, caller must preserve them
    Ok(AuthTokens {
        id_token: data.id_token,
        refresh_token: data.refresh_token,
        email: String::new(),
        local_id: String::new(),
        expires_at: now + expires_in,
    })
}

// ── Ensure we have a valid token, refreshing if needed ───────────────────────

pub fn ensure_valid_token(config: &mut FoxDenConfig) -> Result<String, String> {
    let auth = config.auth.as_ref().ok_or("Not signed in")?;

    if !auth.is_expired() {
        return Ok(auth.id_token.clone());
    }

    let refresh_tok = auth.refresh_token.clone();
    let email = auth.email.clone();
    let local_id = auth.local_id.clone();

    let mut new_auth = refresh_token(&config.api_key, &refresh_tok)?;
    // Preserve email and local_id from original auth
    new_auth.email = email;
    new_auth.local_id = local_id;

    config.auth = Some(new_auth);
    save_config(config).ok();

    Ok(config.auth.as_ref().unwrap().id_token.clone())
}

// ── User-friendly error messages ─────────────────────────────────────────────

fn friendly_error(code: &str) -> String {
    match code {
        "EMAIL_NOT_FOUND" => "No account found with that email".into(),
        "INVALID_PASSWORD" => "Incorrect password".into(),
        "USER_DISABLED" => "This account has been disabled".into(),
        "INVALID_LOGIN_CREDENTIALS" => "Invalid email or password".into(),
        "TOO_MANY_ATTEMPTS_TRY_LATER" => "Too many attempts — try again later".into(),
        "TOKEN_EXPIRED" => "Session expired — please sign in again".into(),
        other => format!("Auth error: {}", other),
    }
}
