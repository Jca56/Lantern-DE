//! Running an app so it outlives what started it: in a session of its
//! own (no terminal to hang up on it, no process group to be killed with),
//! with nothing of ours open on its standard streams, from the home
//! folder. A thread waits for it, so it never lingers as a zombie of ours.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

unsafe extern "C" {
    fn setsid() -> i32;
}

/// Run `command` (a `.desktop` file's, quotes and all) through the shell,
/// as the desktop runs one.
pub fn run(command: &str) -> io::Result<()> {
    if command.trim().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "nothing to run"));
    }
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c").arg(command);
    detached(cmd)
}

/// Run `program` with `args`, no shell between.
pub fn run_args(program: &str, args: &[&str]) -> io::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    detached(cmd)
}

fn detached(mut cmd: Command) -> io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(home) = lntrn_sys::dirs::home() {
        cmd.current_dir(home);
    }
    // SAFETY: `setsid` is async-signal-safe, as a `pre_exec` hook must be.
    // The child of a fork leads no process group, so it can't fail.
    unsafe {
        cmd.pre_exec(|| {
            setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    std::thread::Builder::new().name("reap".into()).spawn(move || {
        let _ = child.wait();
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_runs_in_a_session_of_its_own() {
        let dir = std::env::temp_dir().join(format!("lntrn-apps-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("sid");
        // A session leader's session id (the stat line's sixth field) is
        // its own pid.
        run(&format!("[ \"$(cut -d' ' -f6 /proc/$$/stat)\" = \"$$\" ] && echo yes > '{}'", out.display())).unwrap();
        let said = (0..100).find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::read_to_string(&out).ok().filter(|s| !s.is_empty())
        });
        assert_eq!(said.as_deref(), Some("yes\n"));
        assert!(run("   ").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
