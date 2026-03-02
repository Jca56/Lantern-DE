use serde::Deserialize;

use crate::cloud::config::cache_dir;

// ── Cloud file entry ─────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CloudEntry {
    pub name: String,
    pub full_path: String,
    pub size: u64,
    pub updated: String,
    pub content_type: String,
    pub download_token: Option<String>,
}

// ── Firebase Storage REST responses ──────────────────────────────────────────

#[derive(Deserialize)]
struct ListResponse {
    #[serde(default)]
    items: Vec<StorageObject>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageObject {
    name: String,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    metadata: Option<std::collections::HashMap<String, String>>,
}

// Firebase Storage uses this base URL for the REST API
fn storage_api_base(bucket: &str) -> String {
    format!(
        "https://firebasestorage.googleapis.com/v0/b/{}",
        urlencoding::encode(bucket),
    )
}

// ── List files in Fox Den ────────────────────────────────────────────────────

pub fn list_files(
    bucket: &str,
    id_token: &str,
) -> Result<Vec<CloudEntry>, String> {
    let base = storage_api_base(bucket);
    let url = format!("{}/o?prefix=fox_den/&delimiter=/", base);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .bearer_auth(id_token)
        .send()
        .map_err(|e| format!("Network error: {}", e))?;

    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read error: {}", e))?;

    if !status.is_success() {
        return Err(format!("List failed ({}): {}", status, &text[..text.len().min(200)]));
    }

    let data: ListResponse =
        serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;

    let entries = data
        .items
        .into_iter()
        .filter(|obj| {
            // Skip the folder marker itself
            obj.name != "fox_den/" && obj.name.starts_with("fox_den/")
        })
        .map(|obj| {
            let name = obj
                .name
                .strip_prefix("fox_den/")
                .unwrap_or(&obj.name)
                .to_string();
            let size: u64 = obj
                .size
                .as_deref()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            let download_token = obj
                .metadata
                .as_ref()
                .and_then(|m| m.get("firebaseStorageDownloadTokens").cloned());

            CloudEntry {
                name,
                full_path: obj.name,
                size,
                updated: obj.updated.unwrap_or_default(),
                content_type: obj.content_type.unwrap_or_else(|| "application/octet-stream".into()),
                download_token,
            }
        })
        .collect();

    Ok(entries)
}

// ── Upload a file ────────────────────────────────────────────────────────────

pub fn upload_file(
    bucket: &str,
    id_token: &str,
    local_path: &str,
) -> Result<CloudEntry, String> {
    let file_name = std::path::Path::new(local_path)
        .file_name()
        .ok_or("Invalid file path")?
        .to_string_lossy();

    let file_size = std::fs::metadata(local_path)
        .map_err(|e| format!("Cannot read file: {}", e))?
        .len();

    // Warn for files > 100 MB
    const WARN_SIZE: u64 = 100 * 1024 * 1024;
    if file_size > WARN_SIZE {
        // Caller should check size before calling, but we note it
        eprintln!(
            "Warning: uploading large file ({:.1} MB)",
            file_size as f64 / (1024.0 * 1024.0)
        );
    }

    let storage_path = format!("fox_den/{}", file_name);
    let encoded_path = urlencoding::encode(&storage_path);
    let base = storage_api_base(bucket);
    let url = format!(
        "{}/o?uploadType=media&name={}",
        base, encoded_path,
    );

    let data = std::fs::read(local_path).map_err(|e| format!("Read error: {}", e))?;

    let content_type = guess_content_type(&file_name);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(id_token)
        .header("Content-Type", &content_type)
        .body(data)
        .send()
        .map_err(|e| format!("Upload error: {}", e))?;

    let status = resp.status();
    let text = resp.text().map_err(|e| format!("Read error: {}", e))?;

    if !status.is_success() {
        return Err(format!("Upload failed ({}): {}", status, &text[..text.len().min(200)]));
    }

    let obj: StorageObject =
        serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;

    let download_token = obj
        .metadata
        .as_ref()
        .and_then(|m| m.get("firebaseStorageDownloadTokens").cloned());

    Ok(CloudEntry {
        name: file_name.to_string(),
        full_path: obj.name,
        size: file_size,
        updated: obj.updated.unwrap_or_default(),
        content_type,
        download_token,
    })
}

// ── Download a file ──────────────────────────────────────────────────────────

pub fn download_file(
    bucket: &str,
    id_token: &str,
    cloud_entry: &CloudEntry,
) -> Result<std::path::PathBuf, String> {
    let encoded_path = urlencoding::encode(&cloud_entry.full_path);
    let base = storage_api_base(bucket);
    let url = format!("{}/o/{}?alt=media", base, encoded_path);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .bearer_auth(id_token)
        .send()
        .map_err(|e| format!("Download error: {}", e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(format!("Download failed ({}): {}", status, &text[..text.len().min(200)]));
    }

    let bytes = resp.bytes().map_err(|e| format!("Read error: {}", e))?;

    let cache = cache_dir();
    std::fs::create_dir_all(&cache).map_err(|e| format!("Cache dir error: {}", e))?;

    let dest = cache.join(&cloud_entry.name);
    std::fs::write(&dest, &bytes).map_err(|e| format!("Write error: {}", e))?;

    Ok(dest)
}

// ── Delete a file from cloud ─────────────────────────────────────────────────

pub fn delete_file(
    bucket: &str,
    id_token: &str,
    full_path: &str,
) -> Result<(), String> {
    let encoded_path = urlencoding::encode(full_path);
    let base = storage_api_base(bucket);
    let url = format!("{}/o/{}", base, encoded_path);

    let client = reqwest::blocking::Client::new();
    let resp = client
        .delete(&url)
        .bearer_auth(id_token)
        .send()
        .map_err(|e| format!("Delete error: {}", e))?;

    let status = resp.status();
    // 204 No Content = success, 404 = already deleted
    if status.is_success() || status.as_u16() == 404 {
        Ok(())
    } else {
        let text = resp.text().unwrap_or_default();
        Err(format!("Delete failed ({}): {}", status, &text[..text.len().min(200)]))
    }
}

// ── Content type guesser ─────────────────────────────────────────────────────

fn guess_content_type(name: &str) -> String {
    let ext = std::path::Path::new(name)
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "rs" => "text/x-rust",
        "toml" => "text/x-toml",
        "yaml" | "yml" => "text/x-yaml",
        "csv" => "text/csv",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        _ => "application/octet-stream",
    }
    .into()
}
