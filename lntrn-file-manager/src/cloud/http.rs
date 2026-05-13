// Thin ureq wrapper that injects the bearer token from the shared Session and
// auto-refreshes once on 401.

use std::sync::{Arc, Mutex};

use super::{auth, CloudConfig, Session};

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
        match ureq::get(url).set("Authorization", &format!("Bearer {tok}")).call() {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                // Force a refresh and retry once.
                self.force_refresh()?;
                let tok = self.token()?;
                ureq::get(url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .call()
                    .map_err(|e| anyhow::anyhow!("GET {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("GET {url}: {e}")),
        }
    }

    pub fn post_json(&self, url: &str, body: serde_json::Value) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match ureq::post(url)
            .set("Authorization", &format!("Bearer {tok}"))
            .send_json(body.clone())
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                ureq::post(url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .send_json(body)
                    .map_err(|e| anyhow::anyhow!("POST {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("POST {url}: {e}")),
        }
    }

    pub fn patch_json(&self, url: &str, body: serde_json::Value) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match ureq::request("PATCH", url)
            .set("Authorization", &format!("Bearer {tok}"))
            .send_json(body.clone())
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                ureq::request("PATCH", url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .send_json(body)
                    .map_err(|e| anyhow::anyhow!("PATCH {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("PATCH {url}: {e}")),
        }
    }

    pub fn delete(&self, url: &str) -> anyhow::Result<ureq::Response> {
        let tok = self.token()?;
        match ureq::request("DELETE", url)
            .set("Authorization", &format!("Bearer {tok}"))
            .call()
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                ureq::request("DELETE", url)
                    .set("Authorization", &format!("Bearer {tok}"))
                    .call()
                    .map_err(|e| anyhow::anyhow!("DELETE {url}: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("DELETE {url}: {e}")),
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
        match ureq::request("POST", url)
            .set("Authorization", &format!("Bearer {tok}"))
            .set("Content-Type", content_type)
            .send_bytes(bytes)
        {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(401, _)) => {
                self.force_refresh()?;
                let tok = self.token()?;
                ureq::request("POST", url)
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
