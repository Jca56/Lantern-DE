//! Mounting, ejecting and formatting drives through UDisks.
//!
//! Every function here waits on a child process (`udisksctl`, `busctl`):
//! an unmount returns when the kernel has flushed the stick, a format when
//! mkfs has finished. None of them may be called from the render thread;
//! `app/device_ops.rs` runs them on a worker and shows a "working" state.

use std::path::PathBuf;

use crate::fs::{install_hint, invalidate_mount_table, parent_disk_of, unescape_mount, Drive};

/// Mount a removable drive via `udisksctl mount -b <device>`. Polkit handles
/// the auth prompt (no sudo). On success returns the new mount point — udisks2
/// mounts to `/run/media/$USER/<LABEL>/`.
pub fn mount_drive(drive: &Drive) -> Result<PathBuf, String> {
    if drive.mounted {
        return Ok(drive.mount_point.clone());
    }
    let output = std::process::Command::new("udisksctl")
        .args(["mount", "-b", &drive.device])
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "udisks2 is not installed \u{2014} {}",
                    install_hint("sudo emerge sys-fs/udisks", "sudo pacman -S udisks2")
                )
            } else {
                format!("spawn udisksctl: {e}")
            }
        })?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("udisksctl: {}", msg.trim()));
    }
    // Output looks like: "Mounted /dev/sda1 at /run/media/alva/ARCH_202604."
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(at) = stdout.find(" at ") {
        let tail = &stdout[at + 4..];
        let mount = tail.trim().trim_end_matches('.').trim();
        if !mount.is_empty() {
            invalidate_mount_table();
            return Ok(PathBuf::from(mount));
        }
    }
    // Fallback: re-scan /proc/mounts for the device.
    if let Ok(contents) = std::fs::read_to_string("/proc/mounts") {
        for line in contents.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(drive.device.as_str()) {
                if let Some(mp) = parts.next() {
                    invalidate_mount_table();
                    return Ok(unescape_mount(mp));
                }
            }
        }
    }
    Err("mounted, but couldn't determine mount point".to_string())
}

/// The drive a Format dialog was opened for is still the disk behind its
/// device name. Runs `lsblk` (through `detect_drives`).
pub fn same_drive_present(drive: &Drive, disk_size: u64) -> bool {
    crate::fs::detect_drives().into_iter().any(|d| {
        d.device == drive.device
            && d.parent_disk == drive.parent_disk
            && d.removable
            && d.name == drive.name
            && crate::fs::drive_disk_size(&d) == disk_size
    })
}

/// Format a removable drive as ext4 via UDisks2 D-Bus (`busctl call`).
/// Active-seat users are allowed by the `modify-device` polkit rule
/// (`<allow_active>yes</allow_active>`), so no auth prompt is needed.
///
/// For removable drives, this targets the *whole disk* (e.g. `/dev/sda`,
/// not `/dev/sda1`) and writes ext4 directly to the raw block device with
/// no partition table — recovers the full disk capacity even if a leftover
/// partition layout was present.
pub fn format_drive_ext4(drive: &Drive, label: &str) -> Result<(), String> {
    let target = if drive.removable && !drive.parent_disk.is_empty() {
        drive.parent_disk.clone()
    } else {
        drive.device.clone()
    };

    // Unmount everything on the target disk (disk itself + all partitions).
    unmount_all_on_disk(&target);

    let basename = target.trim_start_matches("/dev/");
    if basename.is_empty() || basename.contains('/') {
        return Err(format!("invalid device: {target}"));
    }
    let obj_path = format!("/org/freedesktop/UDisks2/block_devices/{}", basename);

    // Format(in s type, in a{sv} options).
    // `take-ownership` makes UDisks2 chown the new filesystem root to the
    // calling user — without it, ext4 belongs to root and the user can't
    // write to their own USB.
    // busctl gives a call 25 seconds unless told otherwise, and Format only
    // answers when mkfs has finished: on a large or slow stick that ran
    // out, and a format that was still going was reported as failed.
    let mut args: Vec<String> = vec![
        "call".into(),
        format!("--timeout={FORMAT_TIMEOUT_SECS}"),
        "--system".into(),
        "org.freedesktop.UDisks2".into(),
        obj_path,
        "org.freedesktop.UDisks2.Block".into(),
        "Format".into(),
        "sa{sv}".into(),
        "ext4".into(),
    ];
    let entries = if label.is_empty() { 1 } else { 2 };
    args.push(entries.to_string());
    args.push("take-ownership".into());
    args.push("b".into());
    args.push("true".into());
    if !label.is_empty() {
        args.push("label".into());
        args.push("s".into());
        args.push(label.into());
    }

    let output = std::process::Command::new("busctl")
        .args(&args)
        .output()
        .map_err(|e| format!("spawn busctl: {e}"))?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format_failure(msg.trim()));
    }
    invalidate_mount_table();
    Ok(())
}

/// How long busctl waits for UDisks to finish a format. Far longer than any
/// mkfs on removable media takes; it only bounds a daemon that hangs.
const FORMAT_TIMEOUT_SECS: u32 = 2 * 60 * 60;

/// What to tell the user when the Format call did not succeed. Running out
/// of time is not a failed format: UDisks is still at it, and saying
/// "failed" invites pulling the stick in the middle of mkfs.
fn format_failure(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") || lower.contains("noreply") {
        "The drive is still being formatted. Keep it plugged in: it shows up \
         in the sidebar again when the format is done."
            .to_string()
    } else {
        format!("format: {stderr}")
    }
}

/// Unmount the disk itself and all partitions on it via udisksctl. Errors
/// are ignored — Format will surface a meaningful message if anything is
/// still busy.
fn unmount_all_on_disk(disk_device: &str) {
    let Ok(contents) = std::fs::read_to_string("/proc/mounts") else {
        return;
    };
    for line in contents.lines() {
        let mut parts = line.split_whitespace();
        let Some(device) = parts.next() else {
            continue;
        };
        if !device.starts_with("/dev/") {
            continue;
        }
        let parent = parent_disk_of(device);
        if device == disk_device || parent == disk_device {
            let _ = std::process::Command::new("udisksctl")
                .args(["unmount", "-b", device])
                .output();
        }
    }
}

/// Set a drive's filesystem label via UDisks2 D-Bus.
#[allow(dead_code)]
pub fn relabel_drive(drive: &Drive, new_label: &str) -> Result<(), String> {
    let basename = drive.device.trim_start_matches("/dev/");
    if basename.is_empty() || basename.contains('/') {
        return Err(format!("invalid device: {}", drive.device));
    }
    let obj_path = format!("/org/freedesktop/UDisks2/block_devices/{}", basename);

    let output = std::process::Command::new("busctl")
        .args([
            "call",
            "--system",
            "org.freedesktop.UDisks2",
            &obj_path,
            "org.freedesktop.UDisks2.Filesystem",
            "SetLabel",
            "sa{sv}",
            new_label,
            "0",
        ])
        .output()
        .map_err(|e| format!("spawn busctl: {e}"))?;
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("relabel: {}", msg.trim()));
    }
    Ok(())
}

/// Unmount a removable drive via `udisksctl unmount -b <device>`. The sidebar
/// shows one row per physical disk, so ejecting a removable disk unmounts
/// every mounted partition on it, not just the one the row represents.
pub fn unmount_drive(drive: &Drive) -> Result<(), String> {
    let mut devices = vec![drive.device.clone()];
    if drive.removable && !drive.parent_disk.is_empty() {
        if let Ok(contents) = std::fs::read_to_string("/proc/mounts") {
            for line in contents.lines() {
                let Some(device) = line.split_whitespace().next() else {
                    continue;
                };
                if device.starts_with("/dev/")
                    && parent_disk_of(device) == drive.parent_disk
                    && !devices.iter().any(|d| d == device)
                {
                    devices.push(device.to_string());
                }
            }
        }
    }
    let mut first_err = None;
    for device in &devices {
        let result = std::process::Command::new("udisksctl")
            .args(["unmount", "-b", device])
            .output()
            .map_err(|e| format!("spawn udisksctl: {e}"))
            .and_then(|output| {
                if output.status.success() {
                    Ok(())
                } else {
                    let msg = String::from_utf8_lossy(&output.stderr);
                    Err(format!("udisksctl: {}", msg.trim()))
                }
            });
        if let Err(e) = result {
            first_err.get_or_insert(e);
        }
    }
    invalidate_mount_table();
    first_err.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_that_ran_out_of_time_is_not_called_failed() {
        let still = format_failure("Call failed: Connection timed out");
        assert!(still.contains("still being formatted"), "{still}");
        let still = format_failure("Call failed: org.freedesktop.DBus.Error.NoReply");
        assert!(still.contains("still being formatted"), "{still}");
        assert_eq!(
            format_failure("Call failed: Device /dev/sdb is busy"),
            "format: Call failed: Device /dev/sdb is busy"
        );
    }
}
