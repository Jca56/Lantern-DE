//! Finding the sockets the compositor opens: its Wayland socket, and the X11
//! display of the XWayland it starts.
//!
//! Both are picked by who owns them, never by name. A session that crashed
//! can leave its sockets behind, and another compositor or X server may be
//! running on a different VT; "any wayland-*" or "the highest X number" would
//! hand the whole shell to the wrong one.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::time::{Duration, Instant};

use nix::libc;

use crate::paths;

const POLL: Duration = Duration::from_millis(50);

/// Wait for `compositor` to listen on a Wayland socket in `dir` and return
/// its name ("wayland-1"). `None` once `timeout` has passed or `keep_waiting`
/// says to stop, whichever comes first.
pub fn wait_for_wayland(
    dir: &Path,
    compositor: i32,
    timeout: Duration,
    keep_waiting: &mut dyn FnMut() -> bool,
) -> Option<String> {
    wait(timeout, keep_waiting, || wayland_socket_of(dir, compositor))
}

/// Wait for the XWayland that `compositor` starts and return its display
/// (":0"). `None` as for [`wait_for_wayland`].
pub fn wait_for_x11(
    compositor: i32,
    timeout: Duration,
    keep_waiting: &mut dyn FnMut() -> bool,
) -> Option<String> {
    wait(timeout, keep_waiting, || x11_display_of(Path::new("/tmp"), compositor))
}

fn wait(
    timeout: Duration,
    keep_waiting: &mut dyn FnMut() -> bool,
    mut look: impl FnMut() -> Option<String>,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(found) = look() {
            return Some(found);
        }
        if Instant::now() >= deadline || !keep_waiting() {
            return None;
        }
        std::thread::sleep(POLL);
    }
}

/// Whether the compositor has been told to run without XWayland. These are
/// its own two switches (the `Backend::Udev` arm of its `main`); with either
/// set there is no display to wait for.
pub fn xwayland_disabled() -> bool {
    std::env::var_os("LNTRN_NO_XWAYLAND").is_some()
        || paths::log_dir().join("no-xwayland").exists()
}

/// Every `wayland-*` socket in `dir`, in name order.
pub fn wayland_sockets(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_socket()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("wayland-"))
        .collect();
    names.sort();
    names
}

fn wayland_socket_of(dir: &Path, compositor: i32) -> Option<String> {
    wayland_sockets(dir)
        .into_iter()
        .find(|name| listener_pid(&dir.join(name)) == Some(compositor))
}

/// The process listening on the Unix socket at `path`. The kernel notes who
/// called listen() and tells whoever connects; a socket nobody listens on any
/// more refuses the connection.
fn listener_pid(path: &Path) -> Option<i32> {
    let path = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un and ucred are plain C structs, valid all zero.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    // One byte short of full, to keep the NUL at the end.
    if path.len() >= addr.sun_path.len() {
        return None;
    }
    for (to, &from) in addr.sun_path.iter_mut().zip(path) {
        *to = from as libc::c_char;
    }

    // Non-blocking: a listener that has stopped accepting and whose queue is
    // full must not hang the login.
    let kind = libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC;
    // SAFETY: socket() takes no pointers; a descriptor it returns is ours
    // alone, and OwnedFd closes it on every way out of here.
    let sock = unsafe {
        let fd = libc::socket(libc::AF_UNIX, kind, 0);
        if fd < 0 {
            return None;
        }
        OwnedFd::from_raw_fd(fd)
    };
    let mut cred_len = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: both calls are given a pointer to a live local of the type
    // they expect, with that type's size.
    let connected_to = unsafe {
        let addr_len = size_of::<libc::sockaddr_un>() as libc::socklen_t;
        if libc::connect(sock.as_raw_fd(), (&raw const addr).cast(), addr_len) != 0 {
            return None;
        }
        libc::getsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &mut cred_len,
        )
    };
    (connected_to == 0).then_some(cred.pid)
}

/// The display whose lock `compositor` holds, once its socket is there too.
///
/// The compositor (Smithay's `X11Lock`) takes `<tmp>/.X<n>-lock` and writes
/// its own pid into it before it binds `<tmp>/.X11-unix/X<n>` and hands both
/// to XWayland.
fn x11_display_of(tmp: &Path, compositor: i32) -> Option<String> {
    std::fs::read_dir(tmp)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let number: u32 = name.to_str()?.strip_prefix(".X")?.strip_suffix("-lock")?.parse().ok()?;
            let holder: i32 = std::fs::read_to_string(entry.path()).ok()?.trim().parse().ok()?;
            (holder == compositor).then_some(number)
        })
        .find(|number| tmp.join(format!(".X11-unix/X{number}")).exists())
        .map(|number| format!(":{number}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;
    use std::os::unix::net::{UnixDatagram, UnixListener};

    #[test]
    fn the_wayland_socket_is_the_one_its_owner_listens_on() {
        let dir = TestDir::new("wayland");
        let me = std::process::id() as i32;

        // Left behind by a session that crashed: the file is there, nobody
        // is listening. Not a listener that is then dropped: a child forked
        // by another test at that moment holds a copy of it until it execs,
        // and for that long the socket still answers.
        drop(UnixDatagram::bind(dir.path().join("wayland-1")).unwrap());
        std::fs::write(dir.path().join("wayland-1.lock"), "").unwrap();
        assert_eq!(wayland_sockets(dir.path()), ["wayland-1"]);
        assert_eq!(wayland_socket_of(dir.path(), me), None);

        let _live = UnixListener::bind(dir.path().join("wayland-2")).unwrap();
        assert_eq!(wayland_socket_of(dir.path(), me).as_deref(), Some("wayland-2"));
        // Listening, but it belongs to someone else.
        assert_eq!(wayland_socket_of(dir.path(), 1), None);
    }

    /// Against the session this is run in rather than a stand-in: the real
    /// compositor's sockets have to be found the same way. It only looks, and
    /// connects to the Wayland socket once.
    /// `cargo test -p lntrn-session-manager -- --ignored`
    #[test]
    #[ignore = "needs a running Lantern session"]
    fn the_live_compositors_sockets_are_found() {
        let compositors = crate::procfs::pids_named("lntrn-compositor");
        let [compositor] = compositors[..] else {
            panic!("expected one lntrn-compositor, found {compositors:?}");
        };
        let wayland = wayland_socket_of(&paths::runtime_dir(), compositor);
        let x11 = x11_display_of(Path::new("/tmp"), compositor);
        println!("lntrn-compositor is pid {compositor}: {wayland:?}, {x11:?}");
        assert_eq!(wayland, std::env::var("WAYLAND_DISPLAY").ok());
        assert_eq!(x11, std::env::var("DISPLAY").ok());

        // What the portal restart goes by (argv[0]) against what each
        // process really is (its executable).
        for pid in crate::procfs::pids_named("xdg-desktop-portal") {
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
            println!("xdg-desktop-portal is pid {pid}: {}", exe.display());
            assert_eq!(exe.file_name().unwrap(), "xdg-desktop-portal");
        }
    }

    #[test]
    fn waiting_stops_when_told_to() {
        let dir = TestDir::new("wait");
        let mut asked = 0;
        let found = wait_for_wayland(dir.path(), 1, Duration::from_secs(30), &mut || {
            asked += 1;
            asked < 3
        });
        assert_eq!(found, None);
        assert_eq!(asked, 3);
    }

    #[test]
    fn the_x11_display_is_the_one_whose_lock_the_compositor_holds() {
        let dir = TestDir::new("x11");
        let tmp = dir.path();
        std::fs::create_dir(tmp.join(".X11-unix")).unwrap();
        let add = |number: u32, pid: i32| {
            std::fs::write(tmp.join(format!(".X{number}-lock")), format!("{pid:>10}\n")).unwrap();
            std::fs::write(tmp.join(format!(".X11-unix/X{number}")), "").unwrap();
        };

        // A stale display with a higher number, and someone else's X server.
        add(0, 4242);
        add(7, 999);
        assert_eq!(x11_display_of(tmp, 386_486), None);

        add(1, 386_486);
        assert_eq!(x11_display_of(tmp, 386_486).as_deref(), Some(":1"));

        // Locked but not bound yet: keep waiting.
        std::fs::remove_file(tmp.join(".X11-unix/X1")).unwrap();
        assert_eq!(x11_display_of(tmp, 386_486), None);
    }
}
