pub mod auth;
pub mod config;
pub mod storage;
pub mod sync;

pub use config::FoxDenConfig;
pub use storage::CloudEntry;
pub use sync::{CloudOp, CloudResult};

use std::collections::HashSet;
use std::sync::mpsc;

// ── Fox Den state (lives inside FoxFlareApp) ─────────────────────────────────

pub struct FoxDenState {
    // Panel visibility
    pub panel_open: bool,
    pub panel_anim: f32, // 0.0 = closed, 1.0 = fully open (for slide animation)

    // Triple-click detection
    pub logo_click_times: Vec<f64>,

    // Auth UI
    pub email_input: String,
    pub password_input: String,
    pub auth_error: Option<String>,
    pub auth_loading: bool,

    // Config
    pub config: FoxDenConfig,
    pub signed_in: bool,

    // Cloud files
    pub files: Vec<CloudEntry>,
    pub files_loading: bool,
    pub files_error: Option<String>,
    pub last_refresh: f64,

    // Selected cloud files
    pub selected: HashSet<String>,

    // Upload state
    pub uploading: HashSet<String>,
    pub upload_error: Option<(String, String)>,

    // Download state
    pub downloading: HashSet<String>,
    pub download_error: Option<(String, String)>,

    // Delete confirmation
    pub delete_confirm: Option<String>,

    // Background thread channels
    pub op_sender: mpsc::Sender<CloudOp>,
    pub result_receiver: mpsc::Receiver<CloudResult>,

    // Large file warning
    pub large_file_warning: Option<(String, u64)>,
}

const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100 MB

impl FoxDenState {
    pub fn new() -> Self {
        let (op_sender, op_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();

        // Load config with baked-in defaults + any saved auth tokens
        let config = config::load_config();

        let signed_in = config.auth.as_ref().map_or(false, |a| !a.id_token.is_empty());

        // Spawn background worker if we have config
        if !config.api_key.is_empty() {
            sync::spawn_cloud_worker(config.clone(), op_receiver, result_sender);
        } else {
            // Still spawn a worker — it'll get config after sign-in
            sync::spawn_cloud_worker(config.clone(), op_receiver, result_sender);
        }

        Self {
            panel_open: false,
            panel_anim: 0.0,
            logo_click_times: Vec::new(),
            email_input: String::new(),
            password_input: String::new(),
            auth_error: None,
            auth_loading: false,
            config,
            signed_in,
            files: Vec::new(),
            files_loading: false,
            files_error: None,
            last_refresh: 0.0,
            selected: HashSet::new(),
            uploading: HashSet::new(),
            upload_error: None,
            downloading: HashSet::new(),
            download_error: None,
            delete_confirm: None,
            op_sender,
            result_receiver,
            large_file_warning: None,
        }
    }

    // ── Triple-click detection ───────────────────────────────────────────────

    pub fn register_logo_click(&mut self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        self.logo_click_times.push(now);
        // Keep only clicks within the last 1 second
        self.logo_click_times.retain(|&t| now - t < 1.0);

        if self.logo_click_times.len() >= 3 {
            self.logo_click_times.clear();
            true
        } else {
            false
        }
    }

    pub fn toggle_panel(&mut self) {
        self.panel_open = !self.panel_open;
        if self.panel_open && self.signed_in && self.files.is_empty() {
            self.refresh_files();
        }
        // Tab insertion/removal is handled by FoxFlareApp
    }

    // ── Cloud operations ─────────────────────────────────────────────────────

    pub fn refresh_files(&mut self) {
        if !self.signed_in {
            return;
        }
        self.files_loading = true;
        self.files_error = None;
        self.op_sender.send(CloudOp::ListFiles).ok();
    }

    pub fn upload_file(&mut self, local_path: &str) {
        let file_name = std::path::Path::new(local_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Check file size for warning
        if let Ok(meta) = std::fs::metadata(local_path) {
            if meta.len() > LARGE_FILE_THRESHOLD {
                self.large_file_warning = Some((local_path.to_string(), meta.len()));
                return;
            }
        }

        self.uploading.insert(file_name);
        self.op_sender.send(CloudOp::Upload(local_path.to_string())).ok();
    }

    pub fn upload_file_confirmed(&mut self, local_path: &str) {
        let file_name = std::path::Path::new(local_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.uploading.insert(file_name);
        self.large_file_warning = None;
        self.op_sender.send(CloudOp::Upload(local_path.to_string())).ok();
    }

    pub fn download_file(&mut self, entry: &CloudEntry) {
        self.downloading.insert(entry.name.clone());
        self.op_sender.send(CloudOp::Download(entry.clone())).ok();
    }

    pub fn delete_file(&mut self, full_path: &str) {
        self.op_sender
            .send(CloudOp::Delete(full_path.to_string()))
            .ok();
    }

    pub fn sign_in(&mut self, email: &str, password: &str) {
        self.auth_loading = true;
        self.auth_error = None;
        self.op_sender
            .send(CloudOp::SignIn(email.to_string(), password.to_string()))
            .ok();
    }

    // ── Restart worker with new config ───────────────────────────────────────

    pub fn restart_worker(&mut self) {
        let (op_sender, op_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        self.op_sender = op_sender;
        self.result_receiver = result_receiver;
        sync::spawn_cloud_worker(self.config.clone(), op_receiver, result_sender);
    }

    // ── Poll results from background worker ──────────────────────────────────

    pub fn poll_results(&mut self) {
        while let Ok(result) = self.result_receiver.try_recv() {
            match result {
                CloudResult::FileList(Ok(files)) => {
                    self.files = files;
                    self.files_loading = false;
                    self.files_error = None;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();
                    self.last_refresh = now;
                }
                CloudResult::FileList(Err(e)) => {
                    self.files_loading = false;
                    self.files_error = Some(e);
                }
                CloudResult::Uploaded(local_path, Ok(_entry)) => {
                    let name = std::path::Path::new(&local_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    self.uploading.remove(&name);
                    self.upload_error = None;
                    // Auto-refresh file list
                    self.refresh_files();
                }
                CloudResult::Uploaded(local_path, Err(e)) => {
                    let name = std::path::Path::new(&local_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    self.uploading.remove(&name);
                    self.upload_error = Some((name, e));
                }
                CloudResult::Downloaded(name, Ok(local_path)) => {
                    self.downloading.remove(&name);
                    self.download_error = None;
                    // Open the downloaded file
                    let _ = open::that(&local_path);
                }
                CloudResult::Downloaded(name, Err(e)) => {
                    self.downloading.remove(&name);
                    self.download_error = Some((name, e));
                }
                CloudResult::Deleted(full_path, Ok(())) => {
                    self.files.retain(|f| f.full_path != full_path);
                    self.delete_confirm = None;
                }
                CloudResult::Deleted(_full_path, Err(e)) => {
                    self.files_error = Some(format!("Delete failed: {}", e));
                    self.delete_confirm = None;
                }
                CloudResult::SignedIn(Ok(tokens)) => {
                    self.auth_loading = false;
                    self.auth_error = None;
                    self.signed_in = true;
                    self.config.auth = Some(tokens);
                    config::save_config(&self.config).ok();
                    // Restart worker with updated config and refresh
                    self.restart_worker();
                    self.refresh_files();
                }
                CloudResult::SignedIn(Err(e)) => {
                    self.auth_loading = false;
                    self.auth_error = Some(e);
                }
                CloudResult::TokenRefreshed(Ok(_token)) => {
                    // Token updated in worker, nothing to do in UI
                }
                CloudResult::TokenRefreshed(Err(e)) => {
                    self.auth_error = Some(e);
                    self.signed_in = false;
                }
            }
        }
    }
}
