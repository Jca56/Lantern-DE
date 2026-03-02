use std::sync::mpsc;

use crate::cloud::config::FoxDenConfig;
use crate::cloud::storage::CloudEntry;

// ── Messages between background threads and the UI ───────────────────────────

pub enum CloudOp {
    /// List all files in the Fox Den
    ListFiles,
    /// Upload a local file (absolute path)
    Upload(String),
    /// Download a cloud file to local cache
    Download(CloudEntry),
    /// Delete a cloud file
    Delete(String),
    /// Sign in with email + password
    SignIn(String, String),
    /// Refresh auth token
    RefreshToken,
}

pub enum CloudResult {
    /// File listing completed
    FileList(Result<Vec<CloudEntry>, String>),
    /// Upload completed (original local path, result)
    Uploaded(String, Result<CloudEntry, String>),
    /// Download completed (cloud name, local path result)
    Downloaded(String, Result<std::path::PathBuf, String>),
    /// Delete completed (cloud full_path, result)
    Deleted(String, Result<(), String>),
    /// Sign-in completed
    SignedIn(Result<crate::cloud::config::AuthTokens, String>),
    /// Token refreshed
    TokenRefreshed(Result<String, String>),
}

// ── Background worker ────────────────────────────────────────────────────────

pub fn spawn_cloud_worker(
    config: FoxDenConfig,
    op_receiver: mpsc::Receiver<CloudOp>,
    result_sender: mpsc::Sender<CloudResult>,
) {
    std::thread::spawn(move || {
        let mut cfg = config;

        while let Ok(op) = op_receiver.recv() {
            match op {
                CloudOp::ListFiles => {
                    let result = try_with_token(&mut cfg, |token, bucket| {
                        crate::cloud::storage::list_files(bucket, token)
                    });
                    result_sender.send(CloudResult::FileList(result)).ok();
                }

                CloudOp::Upload(local_path) => {
                    let result = try_with_token(&mut cfg, |token, bucket| {
                        crate::cloud::storage::upload_file(bucket, token, &local_path)
                    });
                    result_sender
                        .send(CloudResult::Uploaded(local_path, result))
                        .ok();
                }

                CloudOp::Download(entry) => {
                    let name = entry.name.clone();
                    let result = try_with_token(&mut cfg, |token, bucket| {
                        crate::cloud::storage::download_file(bucket, token, &entry)
                    });
                    result_sender
                        .send(CloudResult::Downloaded(name, result))
                        .ok();
                }

                CloudOp::Delete(full_path) => {
                    let fp = full_path.clone();
                    let result = try_with_token(&mut cfg, |token, bucket| {
                        crate::cloud::storage::delete_file(bucket, token, &fp)
                    });
                    result_sender
                        .send(CloudResult::Deleted(full_path, result))
                        .ok();
                }

                CloudOp::SignIn(email, password) => {
                    let result =
                        crate::cloud::auth::sign_in(&cfg.api_key, &email, &password);
                    if let Ok(ref tokens) = result {
                        cfg.auth = Some(tokens.clone());
                        crate::cloud::config::save_config(&cfg).ok();
                    }
                    result_sender.send(CloudResult::SignedIn(result)).ok();
                }

                CloudOp::RefreshToken => {
                    let result = crate::cloud::auth::ensure_valid_token(&mut cfg);
                    result_sender.send(CloudResult::TokenRefreshed(result)).ok();
                }
            }
        }
    });
}

// ── Token helper: ensures valid token before calling the operation ────────────

fn try_with_token<T, F>(config: &mut FoxDenConfig, op: F) -> Result<T, String>
where
    F: FnOnce(&str, &str) -> Result<T, String>,
{
    let token = crate::cloud::auth::ensure_valid_token(config)?;
    let bucket = config.storage_bucket.clone();
    op(&token, &bucket)
}
