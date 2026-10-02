// Thin ureq wrapper that injects the bearer token from the shared Session and
// auto-refreshes once on 401.

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

    pub fn get(&self, url: &str) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match agent().get(url)
            .set("Authorization", &format!("Bearer {tok}"))
            .call()
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                // Force a refresh and retry once.
                self.force_refresh()?;
                let tok = self.token()?;
                agent().get(url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .call()
                    .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("GET {url}: {e}")),
        }
    }

    pub fn post_json(&self, url: &str, body: serde_json::Value) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match agent().post(url)
            .set("Authorization", &format!("Bearer {tok}"))
            .send_json(body.clone())
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                agent().post(url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .send_json(body)
                    .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("POST {url}: {e}")),
        }
    }

    pub fn patch_json(&self, url: &str, body: serde_json::Value) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match agent().request("PATCH", url)
            .set("Authorization", &format!("Bearer {tok}"))
            .send_json(body.clone())
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                agent().request("PATCH", url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .send_json(body)
                    .map_err(|e| anyhow::anyhow!("PATCH {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("PATCH {url}: {e}")),
        }
    }


    /// PUT raw bytes (used for Storage uploads).
    pub fn put_bytes(
        &self,
        url: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match agent().request("POST", url)
            .set("Authorization", &format!("Bearer {tok}"))
            .set("Content-Type", content_type)
            .send_bytes(bytes)
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                agent().request("POST", url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .set("Content-Type", content_type)
                    .send_bytes(bytes)
                    .map_err(|e| anyhow::anyhow!("PUT {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("PUT {url}: {e}")),
        }
    }

    fn force_refresh(&self) -> anyhow::Result<()> {
        let mut s = self.session.lock().unwrap();
        auth::refresh(&self.cfg, &mut s)
    }
}
