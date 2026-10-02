//! Phone (MTP) mounts: making one, telling a dead one from a live one, and
//! deciding who may take it down.
//!
//! Every Fox window and every file picker is a process of its own, and they
//! all share the mounts under `~/.lantern/mounts`. So each mount Fox makes
//! gets a small record in the runtime directory:
//!  - which USB address the phone had when it was mounted. A re-plugged
//!    phone comes back under the same name and mount point but at a new
//!    address; the jmtpfs behind the old mount is bound to a device that no
//!    longer exists, and the mount has to be replaced, not reused;
//!  - which process mounted it (the owner);
//!  - one marker per process that currently has a tab or pane inside it.
//!
//! On exit a process unmounts only what it owns, only when no other Fox is
//! inside, and never when it is a picker whose answer lies on the phone
//! (the caller is about to read that file). A mount whose owner is gone is
//! adopted by the next process that browses it, so it is still cleaned up
//! eventually. A mount whose phone is gone is detached by whichever Fox
//! notices first.
//!
//! Everything here touches the device or runs a child process: worker and
//! watcher threads only, except `release_on_exit`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::fs::{self, Phone};

/// What is written next to a mount when Fox makes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Record {
    usb_address: Option<(u32, u32)>,
    owner: Option<u32>,
}

fn format_record(record: &Record) -> String {
    let mut text = String::new();
    if let Some((bus, dev)) = record.usb_address {
        text.push_str(&format!("device={bus},{dev}\n"));
    }
    if let Some(pid) = record.owner {
        text.push_str(&format!("owner={pid}\n"));
    }
    text
}

fn parse_record(text: &str) -> Record {
    let mut record = Record::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("device", value)) => {
                record.usb_address = value.split_once(',').and_then(|(bus, dev)| {
                    Some((bus.trim().parse().ok()?, dev.trim().parse().ok()?))
                });
            }
            Some(("owner", value)) => record.owner = value.trim().parse().ok(),
            _ => {}
        }
    }
    record
}

#[derive(Debug, PartialEq, Eq)]
enum MountState {
    /// Made for the device that is attached now.
    Fresh,
    /// Made for a device that has since been unplugged (or re-plugged).
    Stale,
    /// No record, or no address to compare: only a probe can tell.
    Unknown,
}

fn mount_state(record: Option<Record>, attached: Option<(u32, u32)>) -> MountState {
    match (record.and_then(|r| r.usb_address), attached) {
        (Some(then), Some(now)) if then == now => MountState::Fresh,
        (Some(_), Some(_)) => MountState::Stale,
        _ => MountState::Unknown,
    }
}

/// The exit rule (see the module comment).
fn exit_unmounts(owner: Option<u32>, me: u32, others_inside: bool, answer_inside: bool) -> bool {
    owner == Some(me) && !others_inside && !answer_inside
}

// ── Registry files ──────────────────────────────────────────────────────────

fn registry_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        // tmpfs, gone at logout: the lifetime of the mounts themselves.
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("lantern-fox/phones"),
        _ => fs::mounts_root().join(".registry"),
    }
}

fn slug(mount_point: &Path) -> Option<String> {
    Some(mount_point.file_name()?.to_string_lossy().into_owned())
}

fn record_path(mount_point: &Path) -> Option<PathBuf> {
    Some(registry_dir().join(format!("{}.mount", slug(mount_point)?)))
}

fn marker_path(mount_point: &Path, pid: u32) -> Option<PathBuf> {
    Some(registry_dir().join(format!("{}.user.{pid}", slug(mount_point)?)))
}

fn read_record(mount_point: &Path) -> Option<Record> {
    let text = std::fs::read_to_string(record_path(mount_point)?).ok()?;
    Some(parse_record(&text))
}

fn write_record(mount_point: &Path, record: &Record) {
    let Some(path) = record_path(mount_point) else {
        return;
    };
    let _ = std::fs::create_dir_all(registry_dir());
    // Whole or not at all: another Fox may be reading it.
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, format_record(record)).is_ok() && std::fs::rename(&tmp, &path).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn remove_record(mount_point: &Path) {
    if let Some(path) = record_path(mount_point) {
        let _ = std::fs::remove_file(path);
    }
}

/// Mount points that have a record, i.e. that some Fox mounted.
fn recorded_mounts() -> Vec<PathBuf> {
    let Ok(read_dir) = std::fs::read_dir(registry_dir()) else {
        return Vec::new();
    };
    let root = fs::mounts_root();
    read_dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            Some(root.join(name.strip_suffix(".mount")?))
        })
        .collect()
}

fn comm_of(pid: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
}

/// `pid` is a running Fox (window or picker). The name check keeps a
/// recycled pid of some other program from counting.
fn is_fox(pid: u32) -> bool {
    match (comm_of(&pid.to_string()), comm_of("self")) {
        (Some(theirs), Some(mine)) => theirs == mine,
        _ => false,
    }
}

/// Another running Fox has a tab or pane inside this mount. Markers left by
/// processes that are gone are removed on the way.
fn others_inside(mount_point: &Path) -> bool {
    let Some(prefix) = slug(mount_point).map(|s| format!("{s}.user.")) else {
        return false;
    };
    let Ok(read_dir) = std::fs::read_dir(registry_dir()) else {
        return false;
    };
    let me = std::process::id();
    let mut found = false;
    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_prefix(&prefix)
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me {
            continue;
        }
        if is_fox(pid) {
            found = true;
        } else {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    found
}

// ── Mount table ─────────────────────────────────────────────────────────────

/// By the text of /proc/mounts only. (`fs::is_path_mounted` resolves the
/// path first, which fails on a dead FUSE mount, exactly the kind that has
/// to be found here.)
fn is_mounted(mount_point: &Path) -> bool {
    fs::mounts().iter().any(|(m, _)| m == mount_point)
}

/// jmtpfs mounts directly under the phone mount root, live or dead. Only
/// jmtpfs: anything else someone mounted there is none of our business.
pub fn phone_mounts() -> Vec<PathBuf> {
    let root = fs::mounts_root();
    fs::mounts()
        .into_iter()
        .filter(|(m, fstype)| m.parent() == Some(root.as_path()) && fstype == "fuse.jmtpfs")
        .map(|(m, _)| m)
        .collect()
}

fn run_bounded(mut cmd: std::process::Command, limit: Duration) -> bool {
    use std::process::Stdio;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            // Still running at the limit: it finishes on its own. Reaped
            // here if this process lives that long, by init if not.
            _ => {
                let _ = std::thread::Builder::new()
                    .name("fox-reap".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                return false;
            }
        }
    }
}

fn fusermount(args: &[&str], mount_point: &Path, limit: Duration) -> bool {
    // jmtpfs is a FUSE 2 filesystem; `fusermount3` unmounts it just as well
    // on a machine that only has that one.
    let unmounted = ["fusermount", "fusermount3"].iter().any(|tool| {
        let mut cmd = std::process::Command::new(tool);
        cmd.args(args).arg(mount_point);
        run_bounded(cmd, limit)
    });
    fs::invalidate_mount_table();
    unmounted
}

/// Detach a dead mount. Lazy: a dead FUSE mount cannot be unmounted the
/// normal way while anything still holds a path into it.
fn detach(mount_point: &Path) {
    fusermount(&["-u", "-z"], mount_point, Duration::from_secs(5));
    remove_record(mount_point);
}

/// How long a click on a phone waits for the probe before taking the
/// mount to be alive and busy.
const PROBE_PATIENCE: Duration = Duration::from_millis(1500);

/// Listing the root of a mount whose jmtpfs has lost its device (or died)
/// fails at once. On a live phone it succeeds, or it waits behind a
/// transfer: no answer in time means "busy", which is alive.
fn probe_is_dead(mount_point: &Path) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let dir = mount_point.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("fox-phone-probe".into())
        .spawn(move || {
            let _ = tx.send(std::fs::read_dir(&dir).map(|_| ()));
        });
    if spawned.is_err() {
        return false;
    }
    matches!(rx.recv_timeout(PROBE_PATIENCE), Ok(Err(_)))
}

// ── Operations ──────────────────────────────────────────────────────────────

/// Mount `phone` with jmtpfs, or find it already mounted and alive. Waits
/// on the device (a locked phone takes libmtp's timeouts): worker only.
pub fn mount(phone: &Phone) -> Result<(), String> {
    // `phone` is the sidebar's picture of the device, a moment old. Its USB
    // address decides whether an existing mount is torn down, so it is read
    // again here: after a re-plug the old one would condemn a healthy mount
    // another window has just made.
    let attached = fs::scan_phones().unwrap_or_default();
    let Some(phone) = attached.iter().find(|p| p.mount_point == phone.mount_point) else {
        return Err("The phone is no longer connected.".to_string());
    };
    let mount_point = &phone.mount_point;
    if is_mounted(mount_point) {
        let dead = match mount_state(read_record(mount_point), phone.usb_address) {
            MountState::Stale => true,
            MountState::Fresh | MountState::Unknown => probe_is_dead(mount_point),
        };
        if !dead {
            return Ok(());
        }
        detach(mount_point);
    }
    if let Err(e) = std::fs::create_dir_all(mount_point) {
        return Err(format!("create mount dir: {e}"));
    }
    let mut cmd = std::process::Command::new("jmtpfs");
    // Without this jmtpfs opens the first MTP/PTP device it finds, which
    // with a camera or a second phone attached is not always this one.
    if let Some((bus, dev)) = phone.usb_address {
        cmd.arg(format!("-device={bus},{dev}"));
    }
    let status = cmd.arg(mount_point).status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "jmtpfs is not installed \u{2014} {}",
                fs::install_hint("sudo emerge sys-fs/jmtpfs", "yay -S jmtpfs")
            )
        } else {
            format!("spawn jmtpfs: {e}")
        }
    })?;
    if !status.success() {
        // Almost always the phone is locked or still in charge-only mode.
        return Err(
            "jmtpfs couldn\u{2019}t open the phone. Unlock it and set USB mode to \
             \u{201C}File transfer\u{201D} (tap the USB notification on the phone), \
             then try again."
                .to_string(),
        );
    }
    // jmtpfs returns once mount is established, but give it a beat.
    for _ in 0..20 {
        if is_mounted(mount_point) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    write_record(
        mount_point,
        &Record {
            usb_address: phone.usb_address,
            owner: Some(std::process::id()),
        },
    );
    // The cached table predates this mount; without this the first listing
    // of the phone would be treated as a fast local path.
    fs::invalidate_mount_table();
    Ok(())
}

/// Which of `mounts` are dead: their phone is unplugged, or it was
/// re-plugged since (same name, new address).
fn stale_mounts(
    mounts: &[PathBuf],
    phones: &[Phone],
    record_of: &dyn Fn(&Path) -> Option<Record>,
) -> Vec<PathBuf> {
    mounts
        .iter()
        .filter(
            |mount_point| match phones.iter().find(|p| &p.mount_point == *mount_point) {
                None => true,
                Some(phone) => {
                    mount_state(record_of(mount_point), phone.usb_address) == MountState::Stale
                }
            },
        )
        .cloned()
        .collect()
}

/// Detach the mounts whose phone is gone. `mounts` must have been read
/// BEFORE `phones` was scanned: a phone that is plugged in and mounted
/// between the two reads then shows up in neither list, instead of as a
/// mount without a phone.
///
/// A mount is only detached when it was already found dead by the pass
/// before (`suspects` carries them over): one odd reading of sysfs while a
/// device re-enumerates must not take a mount away. True when something
/// was detached.
pub fn clean_stale(mounts: &[PathBuf], phones: &[Phone], suspects: &mut Vec<PathBuf>) -> bool {
    let dead = stale_mounts(mounts, phones, &read_record);
    let confirmed = confirm_dead(dead, suspects);
    for mount_point in &confirmed {
        detach(mount_point);
    }
    !confirmed.is_empty()
}

/// Of the mounts found dead now, those that were already suspected are
/// returned for detaching; the others become the new suspects.
fn confirm_dead(dead: Vec<PathBuf>, suspects: &mut Vec<PathBuf>) -> Vec<PathBuf> {
    let (confirmed, fresh): (Vec<PathBuf>, Vec<PathBuf>) =
        dead.into_iter().partition(|m| suspects.contains(m));
    *suspects = fresh;
    confirmed
}

/// Keep this process's "I am inside" markers in step with the folders its
/// tabs and panes show, and take over mounts whose owner is gone.
pub fn sync_usage(shown: &[PathBuf]) {
    let me = std::process::id();
    for mount_point in recorded_mounts() {
        let Some(marker) = marker_path(&mount_point, me) else {
            continue;
        };
        if !shown.iter().any(|p| p.starts_with(&mount_point)) {
            let _ = std::fs::remove_file(marker);
            continue;
        }
        if !marker.exists() {
            let _ = std::fs::write(&marker, b"");
        }
        if let Some(record) = read_record(&mount_point) {
            if !record
                .owner
                .is_some_and(|owner| owner == me || is_fox(owner))
            {
                write_record(
                    &mount_point,
                    &Record {
                        owner: Some(me),
                        ..record
                    },
                );
            }
        }
    }
}

/// Last thing before the process ends. `answers`: the paths a picker is
/// about to hand to its caller.
pub fn release_on_exit(answers: &[PathBuf]) {
    let me = std::process::id();
    for mount_point in recorded_mounts() {
        if let Some(marker) = marker_path(&mount_point, me) {
            let _ = std::fs::remove_file(marker);
        }
        let owner = read_record(&mount_point).and_then(|r| r.owner);
        let answer_inside = answers.iter().any(|p| p.starts_with(&mount_point));
        // The markers are only looked at for a mount this process owns.
        let others = owner == Some(me) && others_inside(&mount_point);
        if !exit_unmounts(owner, me, others, answer_inside) {
            continue;
        }
        if !is_mounted(&mount_point) {
            remove_record(&mount_point);
            continue;
        }
        // Not lazy: a mount something is still reading from refuses, and
        // that is the right answer. Bounded: the window is closing.
        if fusermount(&["-u"], &mount_point, Duration::from_secs(2)) {
            remove_record(&mount_point);
        }
    }
}

#[cfg(test)]
mod tests;
