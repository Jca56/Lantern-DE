//! Handing things to the desktop: a file, folder or web address to
//! whatever opens it, a picture to the app that edits or shows it, a
//! notification to the notification daemon. None is waited on; a child
//! that cannot start goes to the log.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(test)]
thread_local! {
    /// The command lines the tests would have started, in order.
    pub static LAUNCHED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Start `cmd` in a process group of its own, so what it opens outlives
/// the editor. The tests start nothing: they note the command line.
fn try_spawn(mut cmd: Command) -> io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    #[cfg(test)]
    return {
        let line: Vec<String> = std::iter::once(cmd.get_program()).chain(cmd.get_args()).map(|a| a.to_string_lossy().into_owned()).collect();
        LAUNCHED.with(|l| l.borrow_mut().push(line.join(" ")));
        Ok(())
    };
    #[cfg(not(test))]
    {
        let mut child = cmd.spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

fn spawn_detached(cmd: Command, what: &str) {
    if let Err(e) = try_spawn(cmd) {
        lntrn_core::log_warn!("{what}: {e}");
    }
}

/// A file, folder or web address to the desktop's default app for it.
pub fn open_external(target: &str) {
    let mut c = Command::new("xdg-open");
    c.arg(target);
    spawn_detached(c, "open");
}

/// `path` to one of the desktop's own apps: the one on the `PATH`, else
/// the one deployed in `~/.lantern/bin`, else whatever the desktop opens
/// such a file with.
fn open_with(app: &str, path: &Path) {
    let deployed = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".lantern/bin").join(app));
    for program in std::iter::once(PathBuf::from(app)).chain(deployed) {
        let mut c = Command::new(&program);
        c.arg(path);
        if try_spawn(c).is_ok() {
            return;
        }
    }
    open_external(&path.display().to_string());
}

/// A Studio document, or a picture to edit, to Lantern Studio.
pub fn open_studio(path: &Path) {
    open_with("lantern-studio", path);
}

/// A picture to the desktop's image viewer.
pub fn open_image_viewer(path: &Path) {
    open_with("lntrn-image-viewer", path);
}

/// A desktop notification: what happened and where.
pub fn notify(summary: &str, body: &str) {
    let mut c = Command::new("notify-send");
    c.args(["-a", "Lantern Code", "-u", "normal", summary, body]);
    spawn_detached(c, "notify");
}
