// Thin ureq wrapper that injects the bearer token from the shared Session and
// auto-refreshes once on 401.

use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::{auth, CloudConfig, Session};

/// The one agent every cloud request goes through. ureq's default has no
/// read or write timeout: a connection that goes silent (suspend/resume,
/// Wi-Fi roam) would park the sync thread forever. These are per read and
/// per write, not for the whole request, so big blobs still transfer; the
/// read value is generous because the reply to a large upload only comes
/// once the server has stored it.
///
/// Connection pooling is OFF on purpose: ureq 2.x clears the socket timeouts
/// when it returns a connection to the pool and does not restore them when
/// it hands that connection out again, so a reused connection would have no
/// timeout at all. One TLS handshake per request is what the code did before
/// it had a shared agent.
pub(super) fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(120))
            .timeout_write(Duration::from_secs(120))
            .max_idle_connections(0)
            .build()
    })
}

/// The server answered, with an error status. Travels inside the
/// `anyhow::Error` (`status_of` finds it again) so a caller can tell a
/// refused precondition or a missing object from a broken connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStatus {
    pub method: &'static str,
    pub url: String,
    pub code: u16,
    /// Google's status word from the error body ("FAILED_PRECONDITION",
    /// "ALREADY_EXISTS"), or empty when the body named none.
    pub status: String,
    /// The server's own explanation, shortened.
    pub message: String,
}

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // "status code N" is ureq's wording, and what the quota check
        // (reconcile::is_quota_error) looks for.
        write!(f, "{} {}: status code {}", self.method, self.url, self.code)?;
        match (self.status.is_empty(), self.message.is_empty()) {
            (true, true) => Ok(()),
            (false, true) => write!(f, " ({})", self.status),
            (true, false) => write!(f, " ({})", self.message),
            (false, false) => write!(f, " ({}: {})", self.status, self.message),
        }
    }
}

impl std::error::Error for HttpStatus {}

/// The error status behind an error, if a server's answer is what it is.
pub fn status_of(e: &anyhow::Error) -> Option<&HttpStatus> {
    e.chain().find_map(|c| c.downcast_ref::<HttpStatus>())
}

/// `{"error": {"status": ..., "message": ...}}`, as Firestore and Storage
/// both send it (Storage without the status word).
fn error_details(body: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    // A streaming call (runQuery) wraps its error in a one-element array.
    let v = v.get(0).unwrap_or(&v);
    let field = |k: &str| {
        v.get("error")
            .and_then(|e| e.get(k))
            .and_then(|s| s.as_str())
            .unwrap_or("")
    };
    let mut message: String = field("message").chars().take(200).collect();
    // A message is one line in the log and in the "cannot sync" list.
    message = message.replace(['\n', '\r'], " ");
    (field("status").to_string(), message)
}

#[derive(Clone)]
pub struct Authed {
    pub cfg: Arc<CloudConfig>,
    pub session: Arc<Mutex<Session>>,
}

impl Authed {
    fn token(&self) -> anyhow::Result<String> {
        let mut s = self.session.lock().unwrap();
        auth::ensure_fresh(&self.cfg, &mut s)?;
        Ok(s.id_token.clone())
    }

    fn uid(&self) -> String {
        self.session.lock().unwrap().uid.clone()
    }

    pub fn user_id(&self) -> String {
        self.uid()
    }

    /// One request with the bearer token. `send` attaches the body and
    /// fires it; it runs a second time if the first try comes back 401 and
    /// the token could be refreshed, so it must be able to produce the body
    /// again.
    fn call(
        &self,
        method: &'static str,
        url: &str,
        send: &mut dyn FnMut(ureq::Request) -> Result<ureq::Response, ureq::Error>,
    ) -> anyhow::Result<ureq::Response> {
        let mut refreshed = false;
        loop {
            let tok = self.token()?;
            let req = agent()
                .request(method, url)
                .set("Authorization", &format!("Bearer {tok}"));
            match send(req) {
                Ok(resp) => return Ok(resp),
                Err(ureq::Error::Status(401, _)) if !refreshed => {
                    self.force_refresh()?;
                    refreshed = true;
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let (status, message) =
                        error_details(&resp.into_string().unwrap_or_default());
                    return Err(anyhow::Error::new(HttpStatus {
                        method,
                        url: url.to_string(),
                        code,
                        status,
                        message,
                    }));
                }
                Err(e) => return Err(anyhow::anyhow!("{method} {url}: {e}")),
            }
        }
    }

    pub fn get(&self, url: &str) -> anyhow::Result<ureq::Response> {
        self.call("GET", url, &mut |req| req.call())
    }

    pub fn delete(&self, url: &str) -> anyhow::Result<ureq::Response> {
        self.call("DELETE", url, &mut |req| req.call())
    }

    pub fn post_json(&self, url: &str, body: serde_json::Value) -> anyhow::Result<ureq::Response> {
        self.call("POST", url, &mut |req| req.send_json(&body))
    }

    /// POST a body of exactly `len` bytes read from what `open` returns,
    /// without holding it in memory (Storage uploads). `open` is called once
    /// per attempt. A reader that fails aborts the request with the body
    /// incomplete, so the server stores nothing.
    pub fn post_stream<'a>(
        &self,
        url: &str,
        content_type: &str,
        len: u64,
        open: &mut dyn FnMut() -> std::io::Result<Box<dyn Read + 'a>>,
    ) -> anyhow::Result<ureq::Response> {
        let mut open_error = None;
        let result = self.call("POST", url, &mut |req| match open() {
            Ok(body) => req
                .set("Content-Type", content_type)
                // With the length given ureq sends the body as is; without
                // it, chunked.
                .set("Content-Length", &len.to_string())
                .send(body),
            Err(e) => {
                let failed = std::io::Error::new(e.kind(), e.to_string());
                open_error = Some(e);
                Err(failed.into())
            }
        });
        match open_error {
            // The file, not the network: say so without the URL around it.
            Some(e) => Err(e.into()),
            None => result,
        }
    }

    fn force_refresh(&self) -> anyhow::Result<()> {
        let mut s = self.session.lock().unwrap();
        auth::refresh(&self.cfg, &mut s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: u16, body: &str) -> HttpStatus {
        let (status, message) = error_details(body);
        HttpStatus {
            method: "POST",
            url: "https://x/y".to_string(),
            code,
            status,
            message,
        }
    }

    #[test]
    fn the_status_survives_inside_an_error() {
        let st = status(
            400,
            r#"{"error":{"code":400,"message":"the stored version\ndoes not match","status":"FAILED_PRECONDITION"}}"#,
        );
        assert_eq!(st.status, "FAILED_PRECONDITION");
        assert_eq!(st.message, "the stored version does not match");
        let e = anyhow::Error::new(st.clone()).context("writing a.txt");
        assert_eq!(status_of(&e), Some(&st));
        assert_eq!(status_of(&anyhow::anyhow!("connection refused")), None);
        let streamed = status(400, r#"[{"error":{"status":"INVALID_ARGUMENT","message":"bad"}}]"#);
        assert_eq!(streamed.status, "INVALID_ARGUMENT");
    }

    #[test]
    fn the_wording_the_quota_check_looks_for_is_kept() {
        let quota = status(429, r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"Quota exceeded."}}"#);
        assert!(quota.to_string().contains("status code 429"));
        // Storage sends no status word, and some proxies no JSON at all.
        let plain = status(402, "<html>Payment Required</html>");
        assert_eq!(plain.to_string(), "POST https://x/y: status code 402");
        assert_eq!((plain.status.as_str(), plain.message.as_str()), ("", ""));
    }
}
