//! The windows on a monitor, from the compositor's window-query socket,
//! for picking one to record.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use lntrn_core::log_warn;
use lntrn_math::Rect;

/// A window where it shows: physical pixels from its monitor's top left,
/// the picker's own space.
pub struct Win {
    pub rect: Rect,
    /// Its app, short (`firefox`), else its title.
    pub label: String,
}

fn path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid can't fail.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    });
    dir.join("lntrn-window-query.sock")
}

/// The windows showing on the monitor called `output`, bottom to top:
/// the last one under a point is the one seen there. None when the
/// compositor can't say.
pub fn on(output: &str) -> Vec<Win> {
    ask(output).unwrap_or_else(|e| {
        log_warn!("screen-capture: the windows on {output}: {e}");
        Vec::new()
    })
}

fn ask(output: &str) -> std::io::Result<Vec<Win>> {
    let mut stream = UnixStream::connect(path())?;
    stream.set_read_timeout(Some(Duration::from_millis(800)))?;
    stream.set_write_timeout(Some(Duration::from_millis(800)))?;
    stream.write_all(format!("query:{output}\n").as_bytes())?;
    // The whole reply, then the compositor hangs up.
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply.lines().take_while(|l| *l != "done").filter_map(parse).collect())
}

/// `win:x\ty\tw\th\tapp_id\ttitle`.
fn parse(line: &str) -> Option<Win> {
    let mut it = line.strip_prefix("win:")?.splitn(6, '\t');
    let mut num = || it.next()?.parse::<f64>().ok();
    let (x, y, w, h) = (num()?, num()?, num()?, num()?);
    let app = it.next().unwrap_or("").trim();
    let title = it.next().unwrap_or("").trim();
    let label = if app.is_empty() { title } else { app.rsplit('.').next().unwrap_or(app) };
    Some(Win { rect: Rect::from_xywh(x, y, w, h), label: label.to_owned() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reads_as_a_window_named_by_its_app() {
        let w = parse("win:10\t20\t300\t200\torg.mozilla.firefox\tSome page").unwrap();
        assert_eq!((w.rect, w.label.as_str()), (Rect::from_xywh(10.0, 20.0, 300.0, 200.0), "firefox"));
        assert_eq!(parse("win:0\t0\t5\t5\t\tJust a title").unwrap().label, "Just a title");
        assert!(parse("done").is_none());
    }
}
