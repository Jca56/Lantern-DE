//! Drive and phone detection on a thread of its own.
//!
//! Detecting devices means running `lsblk`, reading /proc/mounts, a statvfs
//! per mounted drive and a walk through /sys/bus/usb. That used to happen on
//! the render thread every two seconds while frames were drawn: a dropped
//! frame each time during a scroll or drag, and no hot-plug detection at all
//! in an idle window.
//!
//! The watcher sleeps in poll() until something happens:
//!  - the kernel announces a block or USB device coming or going (netlink
//!    uevents; the udev group as well, which fires again once the new
//!    device has been probed and `lsblk` can tell its filesystem);
//!  - the mount table changes (/proc/self/mounts signals that with POLLPRI);
//!  - the UI asks (`kick`): after a mount, eject or format of its own, and
//!    every couple of seconds while frames are being drawn, which keeps the
//!    free-space bars current the way the old poll did.
//!
//! Each pass sends the lists to the main loop only when they changed, and
//! wakes it (`bg::wake`). The same thread looks after the phone mounts:
//! mounts whose phone is gone are detached, and this process's "I am using
//! this phone" markers follow the folders its tabs show (phone_mounts.rs).

use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::fs::{self, Drive, Phone};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Devices {
    pub drives: Vec<Drive>,
    pub phones: Vec<Phone>,
}

struct Shared {
    kick_fd: RawFd,
    /// The folders this process's tabs and panes show.
    shown: Mutex<Vec<PathBuf>>,
    /// The window is closing: the main thread is settling the phone mounts
    /// itself, and this thread keeps its hands off them.
    closing: AtomicBool,
}

pub struct DeviceWatcher {
    rx: Receiver<Devices>,
    shared: Arc<Shared>,
}

impl DeviceWatcher {
    /// Start the thread. The first lists arrive a few milliseconds later.
    pub fn start() -> Self {
        let kick_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        let shared = Arc::new(Shared {
            kick_fd,
            shown: Mutex::new(Vec::new()),
            closing: AtomicBool::new(false),
        });
        let (tx, rx) = mpsc::channel();
        let thread_shared = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("fox-devices".into())
            .spawn(move || watch(thread_shared, tx));
        Self { rx, shared }
    }

    /// Detect again now.
    pub fn kick(&self) {
        if self.shared.kick_fd >= 0 {
            let one: u64 = 1;
            unsafe {
                libc::write(
                    self.shared.kick_fd,
                    &one as *const u64 as *const libc::c_void,
                    8,
                );
            }
        }
    }

    /// Tell the watcher which folders are on screen. Kicks it when they
    /// changed, so the phone markers follow at once.
    pub fn set_shown(&self, shown: &[PathBuf]) {
        let mut held = self.shared.shown.lock().unwrap_or_else(|e| e.into_inner());
        if held.as_slice() != shown {
            *held = shown.to_vec();
            drop(held);
            self.kick();
        }
    }

    /// Called when the window is about to close, before the phone mounts
    /// are released (`phone_mounts::release_on_exit`): from here on the
    /// watcher no longer writes markers or records that the exit is in the
    /// middle of removing.
    pub fn stand_down(&self) {
        self.shared.closing.store(true, Ordering::SeqCst);
    }

    /// The newest lists, if any arrived since the last call.
    pub fn poll(&self) -> Option<Devices> {
        self.rx.try_iter().last()
    }
}

/// A uevent that can change the sidebar: a block device or a USB device
/// came, went or changed. Everything else on that socket (battery, network,
/// input…) is ignored; some of it arrives every few seconds.
fn uevent_matters(message: &[u8]) -> bool {
    message
        .split(|&b| b == 0)
        .any(|field| field == b"SUBSYSTEM=block" || field == b"SUBSYSTEM=usb")
}

fn open_uevent_socket() -> Option<RawFd> {
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            libc::NETLINK_KOBJECT_UEVENT,
        );
        if fd < 0 {
            return None;
        }
        let mut addr: libc::sockaddr_nl = std::mem::zeroed();
        addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        // Group 1: the kernel's own events. Group 2: udev's, sent once it
        // has finished with the device.
        addr.nl_groups = 1 | 2;
        let bound = libc::bind(
            fd,
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        );
        if bound != 0 {
            libc::close(fd);
            return None;
        }
        Some(fd)
    }
}

/// Read everything queued on the uevent socket. True when any of it matters.
fn drain_uevents(fd: RawFd) -> bool {
    let mut relevant = false;
    let mut buf = [0u8; 8192];
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOBUFS) {
            // The socket buffer overflowed and events were dropped; one of
            // them may have been ours.
            relevant = true;
            continue;
        }
        if n <= 0 {
            return relevant;
        }
        relevant |= uevent_matters(&buf[..n as usize]);
    }
}

fn drain_eventfd(fd: RawFd) {
    let mut buf = 0u64;
    unsafe {
        libc::read(fd, &mut buf as *mut u64 as *mut libc::c_void, 8);
    }
}

/// One detection pass, phone-mount housekeeping included.
fn detect(shared: &Shared, suspects: &mut Vec<PathBuf>) -> Devices {
    let drives = fs::detect_drives();
    // Mounts before phones: see `phone_mounts::clean_stale`.
    let mounts = crate::phone_mounts::phone_mounts();
    let phones = match fs::scan_phones() {
        Some(phones) if crate::phone_mounts::clean_stale(&mounts, &phones, suspects) => {
            // `mounted` was read before the dead mounts went.
            fs::scan_phones().unwrap_or(phones)
        }
        Some(phones) => phones,
        // sysfs unreadable: "could not look" is not "all phones are gone".
        None => Vec::new(),
    };
    if !shared.closing.load(Ordering::SeqCst) {
        let shown = shared
            .shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        crate::phone_mounts::sync_usage(&shown);
    }
    Devices { drives, phones }
}

/// After a hot-plug the device node exists before udev has probed it, and
/// `lsblk` only reports a filesystem once it has. Look again a little later.
const FOLLOW_UPS: [Duration; 2] = [Duration::from_millis(1200), Duration::from_millis(4000)];

/// Uevents come in bursts (disk, then each partition): let one settle.
const SETTLE: Duration = Duration::from_millis(150);

/// Never two passes closer together than this, whatever the events do: a
/// pass forks `lsblk`, and a storm of events must not turn into a storm of
/// those.
const MIN_GAP: Duration = Duration::from_millis(250);

fn watch(shared: Arc<Shared>, tx: Sender<Devices>) {
    let uevents = open_uevent_socket();
    // Kept open: poll() reports POLLPRI on it when the mount table changes.
    let mounts = std::fs::File::open("/proc/self/mounts").ok();
    let mounts_fd = mounts.as_ref().map(|f| {
        use std::os::fd::AsRawFd;
        f.as_raw_fd()
    });

    let mut last_sent: Option<Devices> = None;
    let mut follow_ups: Vec<Instant> = Vec::new();
    // Phone mounts found dead by the last pass; see `clean_stale`.
    let mut suspects: Vec<PathBuf> = Vec::new();
    loop {
        let pass_started = Instant::now();
        let found = detect(&shared, &mut suspects);
        // A mount that looked dead is looked at once more before it is
        // detached, also when no event would bring another pass.
        if !suspects.is_empty() && follow_ups.is_empty() {
            follow_ups.push(Instant::now() + FOLLOW_UPS[0]);
        }
        if last_sent.as_ref() != Some(&found) {
            if tx.send(found.clone()).is_err() {
                return; // the window is gone
            }
            last_sent = Some(found);
            crate::bg::wake();
        }
        std::thread::sleep(MIN_GAP.saturating_sub(pass_started.elapsed()));

        if wait_for_change(
            &shared,
            uevents,
            mounts.as_ref(),
            mounts_fd,
            &mut follow_ups,
        ) {
            let now = Instant::now();
            follow_ups = FOLLOW_UPS.iter().map(|d| now + *d).collect();
        }
    }
}

/// Sleep until there is a reason to detect again: a kick, a mount-table
/// change, a device uevent, or a follow-up that has come due. True for a
/// device uevent (a hot-plug), which schedules follow-ups.
fn wait_for_change(
    shared: &Shared,
    uevents: Option<RawFd>,
    mounts: Option<&std::fs::File>,
    mounts_fd: Option<RawFd>,
    follow_ups: &mut Vec<Instant>,
) -> bool {
    loop {
        let now = Instant::now();
        if follow_ups.iter().any(|t| *t <= now) {
            follow_ups.retain(|t| *t > now);
            return false;
        }
        let timeout_ms: i32 = follow_ups
            .iter()
            .min()
            .map_or(-1, |t| t.duration_since(now).as_millis() as i32 + 1);
        let mut pfds = [
            libc::pollfd {
                fd: shared.kick_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: uevents.unwrap_or(-1),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: mounts_fd.unwrap_or(-1),
                events: libc::POLLPRI,
                revents: 0,
            },
        ];
        let ready = unsafe { libc::poll(pfds.as_mut_ptr(), 3, timeout_ms) };
        if ready < 0 {
            // Interrupted: look again. Anything worse would spin, so take
            // a breath first.
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        if ready == 0 {
            continue; // the due follow-up is picked up at the top
        }
        let mut asked = false;
        if pfds[0].revents != 0 {
            drain_eventfd(shared.kick_fd);
            asked = true;
        }
        if pfds[2].revents != 0 {
            if let Some(file) = mounts {
                reread_mounts(file);
            }
            asked = true;
        }
        if pfds[1].revents != 0 {
            if let Some(fd) = uevents {
                if drain_uevents(fd) {
                    std::thread::sleep(SETTLE);
                    drain_uevents(fd);
                    return true;
                }
            }
        }
        if asked {
            return false;
        }
        // Only uevents nobody here cares about: keep sleeping.
    }
}

/// Read the mount table again from the start after poll() flagged a change,
/// so the next poll() waits for the next change.
fn reread_mounts(file: &std::fs::File) {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    let _ = file.seek(SeekFrom::Start(0));
    let mut sink = [0u8; 4096];
    while matches!(file.read(&mut sink), Ok(n) if n > 0) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_block_and_usb_uevents_wake_the_watcher() {
        let kernel = b"add@/devices/pci/usb1/1-2/host6/target/block/sdb\0ACTION=add\0\
            DEVPATH=/devices/x/block/sdb\0SUBSYSTEM=block\0DEVNAME=sdb\0SEQNUM=4411\0";
        assert!(uevent_matters(kernel));
        let usb =
            b"remove@/devices/pci/usb1/1-2\0ACTION=remove\0SUBSYSTEM=usb\0DEVTYPE=usb_device\0";
        assert!(uevent_matters(usb));
        // udev's own messages carry a binary header, then the same fields.
        let udev =
            b"libudev\0\xfe\xed\xca\xfe\x28\0\0\0ACTION=change\0SUBSYSTEM=block\0ID_FS_TYPE=ext4\0";
        assert!(uevent_matters(udev));

        let battery =
            b"change@/devices/x/power_supply/BAT0\0ACTION=change\0SUBSYSTEM=power_supply\0";
        assert!(!uevent_matters(battery));
        // A substring is not a field.
        let lookalike = b"change@/x\0ACTION=change\0SUBSYSTEM=usbmisc\0NOTE=SUBSYSTEM=block-ish\0";
        assert!(!uevent_matters(lookalike));
        assert!(!uevent_matters(b""));
    }
}
