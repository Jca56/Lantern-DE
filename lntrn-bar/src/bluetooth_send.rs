//! Dispatch picked paths (files and folders) for OBEX sending.
//!
//! Folders are zipped to /tmp via `bsdtar` on a background thread before the
//! actual SendFile is queued. Caller tracks expected temp paths so the main
//! thread can delete them once the transfer completes.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use crate::bluetooth_transfer::TransferCmd;

pub fn dispatch_paths(
    mac: &str,
    paths: Vec<String>,
    cmd_tx: &mpsc::Sender<TransferCmd>,
    expected_temp_zips: &mut Vec<String>,
    next_temp_id: &mut u32,
) {
    for path in paths {
        let pb = PathBuf::from(&path);
        if pb.is_dir() {
            spawn_zip_then_send(mac.to_string(), pb,
                cmd_tx.clone(), expected_temp_zips, next_temp_id);
        } else if pb.is_file() {
            let _ = cmd_tx.send(TransferCmd::SendFile {
                mac: mac.to_string(), file_path: path,
            });
        }
    }
}

fn spawn_zip_then_send(
    mac: String,
    folder: PathBuf,
    cmd_tx: mpsc::Sender<TransferCmd>,
    expected_temp_zips: &mut Vec<String>,
    next_temp_id: &mut u32,
) {
    let folder_name = folder.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("folder")
        .to_string();
    let parent = folder.parent().unwrap_or(Path::new("/")).to_path_buf();
    let temp_zip = format!("/tmp/lntrn-bar-{}-{}-{}.zip",
        std::process::id(), *next_temp_id, sanitize(&folder_name));
    *next_temp_id += 1;
    expected_temp_zips.push(temp_zip.clone());

    let result = std::thread::Builder::new().name("bt-zip".into()).spawn(move || {
        let status = Command::new("bsdtar")
            .arg("--format=zip")
            .arg("-c").arg("-f").arg(&temp_zip)
            .arg("-C").arg(&parent)
            .arg(&folder_name)
            .status();
        match status {
            Ok(s) if s.success() => {
                let _ = cmd_tx.send(TransferCmd::SendFile {
                    mac, file_path: temp_zip,
                });
            }
            _ => {
                tracing::error!("bsdtar zip failed for {}", folder_name);
                let _ = std::fs::remove_file(&temp_zip);
            }
        }
    });
    if let Err(e) = result {
        tracing::error!("failed to spawn zip thread: {e}");
    }
}

/// Strip path separators and replace odd characters so the temp filename is
/// safe and the receiver sees a sensible .zip name.
fn sanitize(name: &str) -> String {
    name.chars().map(|c| match c {
        '/' | '\\' | '\0' => '_',
        c if c.is_control() => '_',
        c => c,
    }).collect()
}
