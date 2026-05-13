// Firebase Storage REST. Blobs are content-addressed (sha256) under
// users/{uid}/blobs/{sha256}. Storing by hash gives us free dedup and lets
// us treat "remote blob exists" as "no upload needed".
//
// Endpoints:
//   Upload:   POST https://firebasestorage.googleapis.com/v0/b/{bucket}/o?name={url_encoded_object}
//   Download: GET  https://firebasestorage.googleapis.com/v0/b/{bucket}/o/{url_encoded_object}?alt=media
//   Stat:     GET  https://firebasestorage.googleapis.com/v0/b/{bucket}/o/{url_encoded_object}
//
// All require Authorization: Bearer {id_token}.

use std::io::Read;

use super::http::Authed;

fn object_path(uid: &str, sha256: &str) -> String {
    format!("users/{uid}/blobs/{sha256}")
}

pub fn blob_exists(authed: &Authed, sha256: &str) -> anyhow::Result<bool> {
    let bucket = authed.cfg.storage_bucket.clone();
    let obj = urlencoding::encode(&object_path(&authed.user_id(), sha256)).into_owned();
    let url = format!("https://firebasestorage.googleapis.com/v0/b/{bucket}/o/{obj}");
    match authed.get(&url) {
        Ok(_) => Ok(true),
        Err(e) => {
            // 404 isn't an error to us — it just means "not uploaded yet".
            let msg = format!("{e}");
            if msg.contains("404") {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

pub fn upload_blob(authed: &Authed, sha256: &str, bytes: &[u8], mime: &str) -> anyhow::Result<()> {
    let bucket = authed.cfg.storage_bucket.clone();
    let obj = urlencoding::encode(&object_path(&authed.user_id(), sha256)).into_owned();
    let url = format!("https://firebasestorage.googleapis.com/v0/b/{bucket}/o?name={obj}");
    authed.put_bytes(&url, mime, bytes)?;
    Ok(())
}

pub fn download_blob(authed: &Authed, sha256: &str) -> anyhow::Result<Vec<u8>> {
    let bucket = authed.cfg.storage_bucket.clone();
    let obj = urlencoding::encode(&object_path(&authed.user_id(), sha256)).into_owned();
    let url = format!("https://firebasestorage.googleapis.com/v0/b/{bucket}/o/{obj}?alt=media");
    let resp = authed.get(&url)?;
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf)?;
    Ok(buf)
}
