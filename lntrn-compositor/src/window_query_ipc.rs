//! Unix-socket IPC: external tools (lntrn-screenshot) ↔ compositor window
//! geometry query.
//!
//! Mirrors `gaming_ipc.rs` for setup (SO_PEERCRED uid-gated socket at
//! `/run/user/{uid}/lntrn-window-query.sock`), but is a *one-shot*
//! request/response — the connection is closed right after the reply.
//!
//!   # Client → compositor (one line):
//!   `query:<output_name>`            (output_name may be empty → primary)
//!
//!   # Compositor → client (one line per visible window, bottom→top z-order):
//!   `win:<x>\t<y>\t<w>\t<h>\t<app_id>\t<title>`   (physical px, output-local)
//!   `done`
//!
//! Coordinates are in *physical* pixels relative to the output's top-left —
//! exactly the space the screenshot tool's captured image and selection
//! overlay live in, so the client needs no conversion.
//!
//! Unlike the other IPC sockets (which are drained from the render loop),
//! this one is registered as a calloop event source via [`install_source`]
//! so requests are serviced the instant they arrive — the render loop goes
//! idle when nothing is animating, and the screenshot tool blocks waiting
//! for our reply, so a render-loop poll would deadlock against that idle.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use smithay::reexports::calloop::{generic::Generic, Interest, Mode, PostAction};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut ucred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (ret == 0).then_some(ucred.uid)
}

fn our_uid() -> u32 {
    unsafe { libc::getuid() }
}

pub fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-window-query.sock", uid))
}

/// A parsed request plus the stream to answer on.
pub struct WindowQueryRequest {
    pub output: String,
    pub writer: UnixStream,
}

pub struct WindowQueryIpc {
    listener: Option<UnixListener>,
}

impl WindowQueryIpc {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(?e, "failed to chmod 0600 on window-query IPC socket");
                }
                tracing::info!(?path, "window-query IPC socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind window-query IPC socket");
                None
            }
        };
        Self { listener }
    }

    fn try_clone_listener(&self) -> Option<UnixListener> {
        self.listener.as_ref().and_then(|l| l.try_clone().ok())
    }

    /// Accept every pending connection and read its request line. The
    /// request is tiny and sent immediately on connect, so we read it with
    /// a short blocking timeout right here rather than juggling per-client
    /// calloop sources.
    fn accept_and_collect(&mut self) -> Vec<WindowQueryRequest> {
        let mut requests = Vec::new();
        let Some(listener) = self.listener.as_ref() else {
            return requests;
        };
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    match peer_uid(&stream) {
                        Some(uid) if uid == our_uid() => {}
                        other => {
                            tracing::warn!(
                                ?other,
                                "rejecting window-query IPC connection from foreign uid"
                            );
                            continue;
                        }
                    }
                    stream.set_nonblocking(false).ok();
                    stream
                        .set_read_timeout(Some(Duration::from_millis(250)))
                        .ok();
                    let reader_stream = match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut reader = BufReader::new(reader_stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_ok() {
                        if let Some(output) = line.trim().strip_prefix("query:") {
                            requests.push(WindowQueryRequest {
                                output: output.to_string(),
                                writer: stream,
                            });
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    tracing::warn!(?e, "window-query IPC accept error");
                    break;
                }
            }
        }
        requests
    }
}

/// Register the socket with the compositor event loop. Call once at startup.
pub fn install_source(state: &mut Lantern) {
    let Some(listener) = state.window_query_ipc.try_clone_listener() else {
        return;
    };
    let res = state.loop_handle.insert_source(
        Generic::new(listener, Interest::READ, Mode::Level),
        move |_readiness, _listener, state: &mut Lantern| {
            state.poll_window_query_ipc();
            Ok(PostAction::Continue)
        },
    );
    if let Err(e) = res {
        tracing::warn!(?e, "failed to register window-query IPC source");
    }
}

impl Lantern {
    /// Answer every pending window-geometry request. Driven by the calloop
    /// source installed in [`install_source`].
    pub fn poll_window_query_ipc(&mut self) {
        let requests = self.window_query_ipc.accept_and_collect();
        for req in requests {
            let body = self.window_rects_response(&req.output);
            let mut writer = req.writer;
            let _ = writer.write_all(body.as_bytes());
            let _ = writer.flush();
            // Dropping `writer` closes the connection, signalling EOF to the
            // client which is reading until the socket closes.
        }
    }

    /// Build the reply body listing every window on `output_name`'s active
    /// workspace, bottom→top, in physical pixels relative to that output's
    /// top-left.
    fn window_rects_response(&self, output_name: &str) -> String {
        let mut body = String::new();

        let name = if output_name.is_empty() {
            self.workspaces
                .outputs()
                .next()
                .cloned()
                .unwrap_or_default()
        } else {
            output_name.to_string()
        };

        let geom_scale = self
            .workspaces
            .output_by_name(&name)
            .and_then(|output| {
                self.workspaces
                    .output_geometry(output)
                    .map(|geo| (geo, output.current_scale().fractional_scale()))
            });

        if let (Some((out_geo, scale)), Some(space)) =
            (geom_scale, self.active_space_on(&name))
        {
            for window in space.elements() {
                let Some(geo) = space.element_geometry(window) else {
                    continue;
                };
                // Global logical → output-local logical → physical.
                let px = ((geo.loc.x - out_geo.loc.x) as f64 * scale).round() as i32;
                let py = ((geo.loc.y - out_geo.loc.y) as f64 * scale).round() as i32;
                let pw = (geo.size.w as f64 * scale).round() as i32;
                let ph = (geo.size.h as f64 * scale).round() as i32;
                if pw <= 0 || ph <= 0 {
                    continue;
                }
                let app_id = sanitize(&window.get_app_id());
                let title = sanitize(&window.get_title());
                body.push_str(&format!(
                    "win:{px}\t{py}\t{pw}\t{ph}\t{app_id}\t{title}\n"
                ));
            }
        }

        body.push_str("done\n");
        body
    }
}

/// Strip the field/line delimiters so a stray tab or newline in a window
/// title can't corrupt the wire format.
fn sanitize(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}
