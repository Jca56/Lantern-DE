//! Drags that leave the window: the hand-off to the compositor.
//!
//! While the pointer is inside the window a drag is Fox's own affair. Once
//! it crosses the edge it becomes a `wl_data_device` drag, so other
//! programs (and Fox itself, see dnd_in.rs) can take the drop.
//!
//! The compositor only honours `start_drag` while the button press that
//! began the drag is still held, and it refuses without a word: no error,
//! no `cancelled`. A release that crosses the request on the wire is
//! enough. Believing the drag had started when it had not left `dnd_active`
//! set for good: every later drag-out was blocked and the loop redrew
//! forever. So the hand-off is checked: a `wl_display.sync` goes out right
//! behind `start_drag`, and its answer arrives after everything the
//! compositor did in response. A drag that started took the pointer away
//! from the window first (`wl_pointer.leave`) and swallowed the release; a
//! refused one did neither.

use wayland_client::protocol::{wl_callback, wl_data_device_manager, wl_data_source};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

use crate::wayland::State;

/// User data of the sync callback sent behind `start_drag`.
pub(crate) struct DragProbe;

/// What was seen between `start_drag` and the probe's answer.
#[derive(Default)]
pub(crate) struct ProbeSeen {
    /// The pointer left the window: what a starting drag does first.
    pub(crate) leave: bool,
    /// The button's release reached Fox: no drag took the button over.
    pub(crate) release: bool,
}

impl ProbeSeen {
    fn drag_started(&self) -> bool {
        self.leave && !self.release
    }
}

impl State {
    /// Hand the drag of `dnd_paths` to the compositor. False when that is
    /// not possible (no data device, or the button is already up).
    pub(crate) fn hand_off_drag(&mut self, conn: &Connection, qh: &QueueHandle<State>) -> bool {
        // The release is already here: the compositor has ended the press,
        // and would refuse. The drop is handled in-window instead.
        if self.left_released {
            return false;
        }
        let (Some(mgr), Some(device), Some(surface)) =
            (&self.data_device_manager, &self.data_device, &self.surface)
        else {
            return false;
        };
        let source = mgr.create_data_source(qh, ());
        source.offer(crate::dnd_in::URI_LIST.to_string());
        source.offer("text/plain".to_string());
        if source.version() >= 3 {
            source.set_actions(
                wl_data_device_manager::DndAction::Copy | wl_data_device_manager::DndAction::Move,
            );
        }
        device.start_drag(Some(&source), surface, None, self.dnd_serial);
        conn.display().sync(qh, DragProbe);
        if let Some(old) = self.dnd_source.replace(source) {
            old.destroy();
        }
        self.dnd_probe = Some(ProbeSeen::default());
        self.dnd_active = true;
        true
    }

    /// The drag handed to the compositor is over (dropped, cancelled, or
    /// never started).
    pub(crate) fn end_drag_out(&mut self) {
        self.dnd_active = false;
        self.dnd_dropped_at = None;
        self.dnd_probe = None;
        self.dnd_paths.clear();
        // One source per drag; without this each drag leaks one. Destroying
        // the source of a drag that is somehow still alive ends that drag.
        if let Some(source) = self.dnd_source.take() {
            source.destroy();
        }
        self.frame_done = true;
    }
}

/// How long a receiver that took a drop gets to finish with it.
const DROP_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

impl State {
    /// A receiver that accepted the drop and then crashed or hung never
    /// sends "finished", and the compositor sends nothing in its place.
    /// Left at that, the hand-off would count as still running for good:
    /// no drag could start again, and every attempt would end as a click
    /// that opens the item. After a while the old drag is given up on.
    pub(crate) fn expire_drag_out(&mut self) {
        if self.dnd_active
            && self
                .dnd_dropped_at
                .is_some_and(|at| at.elapsed() >= DROP_PATIENCE)
        {
            self.end_drag_out();
        }
    }
}

impl Dispatch<wl_callback::WlCallback, DragProbe> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &DragProbe,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            if let Some(seen) = state.dnd_probe.take() {
                if !seen.drag_started() {
                    state.end_drag_out();
                }
            }
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                let Some(payload) = payload(&state.dnd_paths, &mime_type) else {
                    return;
                };
                // Written on a thread of its own: the pipe holds 64 KiB,
                // and a receiver that reads slowly (or is Fox's own drop
                // reader, served by this loop) must not stall the window.
                let _ = std::thread::Builder::new()
                    .name("fox-drag-send".into())
                    .spawn(move || {
                        use std::io::Write;
                        let _ = std::fs::File::from(fd).write_all(&payload);
                    });
            }
            // The receiver has the drop and still has to say "finished".
            wl_data_source::Event::DndDropPerformed => {
                if state.dnd_source.as_ref() == Some(source) {
                    state.dnd_dropped_at = Some(std::time::Instant::now());
                }
            }
            wl_data_source::Event::DndFinished | wl_data_source::Event::Cancelled => {
                if state.dnd_source.as_ref() == Some(source) {
                    state.end_drag_out();
                } else {
                    // A source that was already given up on.
                    source.destroy();
                }
            }
            _ => {}
        }
    }
}

/// What a receiver asking for `mime` is sent for the dragged `paths`.
fn payload(paths: &[std::path::PathBuf], mime: &str) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    let mut out = Vec::new();
    match mime {
        "text/uri-list" => {
            // RFC 3986: a raw space, '#', '%' or non-ASCII byte makes the
            // receiver cut or misread the path.
            for path in paths {
                out.extend_from_slice(b"file://");
                out.extend_from_slice(crate::file_ops::percent_encode_path(path).as_bytes());
                out.extend_from_slice(b"\r\n");
            }
        }
        "text/plain" => {
            for (i, path) in paths.iter().enumerate() {
                if i > 0 {
                    out.push(b'\n');
                }
                out.extend_from_slice(path.as_os_str().as_bytes());
            }
        }
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_drag_started_only_if_the_pointer_left_and_the_release_never_came() {
        let seen = |leave, release| ProbeSeen { leave, release }.drag_started();
        assert!(seen(true, false));
        // Refused with the button still down: nothing happened at all.
        assert!(!seen(false, false));
        // The release beat the request: the pointer may well have left the
        // window afterwards, but no drag holds the button.
        assert!(!seen(true, true));
        assert!(!seen(false, true));
    }

    #[test]
    fn dragged_paths_are_sent_as_uris_and_as_plain_text() {
        let paths = [PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c")];
        assert_eq!(
            payload(&paths, "text/uri-list").unwrap(),
            b"file:///tmp/a%20b.txt\r\nfile:///tmp/c\r\n"
        );
        assert_eq!(
            payload(&paths, "text/plain").unwrap(),
            b"/tmp/a b.txt\n/tmp/c"
        );
        assert_eq!(payload(&paths, "image/png"), None);
        // What is sent reads back as what was dragged.
        assert_eq!(
            crate::dnd_in::parse_uri_list(&payload(&paths, "text/uri-list").unwrap()),
            paths
        );
    }
}
