//! Shared calloop plumbing for the newline-delimited Unix-socket IPC modules
//! (workspaces, HDR, gaming, clipboard, CC thumbnails, hover preview).
//!
//! Each module still owns its listener + client streams and drains them with
//! its existing non-blocking `poll()`. What this adds is the *wake-up*: a dup
//! of every socket fd is registered with the event loop, so a module's poll
//! runs exactly when one of its sockets is readable. Before, the render path
//! polled all six sockets on every output frame — accept + read syscalls at
//! the combined refresh rate of every monitor, and a request that arrived
//! while the desktop was idle waited for the next render to be serviced.
//!
//! `window_query_ipc` already did this for itself; this is the same pattern
//! generalised so each module only has to expose three tiny hooks.

use std::os::fd::{AsFd, OwnedFd};

use smithay::reexports::calloop::{generic::Generic, Interest, Mode, PostAction};

use crate::Lantern;

/// Runs when one of a module's sockets is readable. Must drain without
/// blocking and register any clients accepted during the drain.
pub type OnReady = fn(&mut Lantern);

/// Whether the client with this id is still tracked by its module. A peer
/// that hung up leaves its fd level-readable (EOF) forever, so the moment
/// the module drops the client its source must go too.
pub type ClientAlive = fn(&Lantern, u64) -> bool;

pub fn dup_fd<F: AsFd>(fd: &F) -> Option<OwnedFd> {
    fd.as_fd().try_clone_to_owned().ok()
}

pub fn register_listener(
    state: &mut Lantern,
    fd: OwnedFd,
    name: &'static str,
    on_ready: OnReady,
) {
    let res = state.loop_handle.insert_source(
        Generic::new(fd, Interest::READ, Mode::Level),
        move |_, _, state: &mut Lantern| {
            on_ready(state);
            Ok(PostAction::Continue)
        },
    );
    if let Err(e) = res {
        tracing::warn!(?e, ipc = name, "failed to register IPC listener with the event loop");
    }
}

pub fn register_client(
    state: &mut Lantern,
    fd: OwnedFd,
    name: &'static str,
    id: u64,
    on_ready: OnReady,
    alive: ClientAlive,
) {
    let res = state.loop_handle.insert_source(
        Generic::new(fd, Interest::READ, Mode::Level),
        move |_, _, state: &mut Lantern| {
            on_ready(state);
            if alive(state, id) {
                Ok(PostAction::Continue)
            } else {
                Ok(PostAction::Remove)
            }
        },
    );
    if let Err(e) = res {
        tracing::warn!(?e, ipc = name, id, "failed to register IPC client with the event loop");
    }
}

/// Register every IPC listener with the event loop. Call once at startup,
/// after `Lantern::new` has bound the sockets.
pub fn install_all(state: &mut Lantern) {
    if let Some(fd) = state.workspace_ipc.listener_fd() {
        register_listener(state, fd, "workspaces", Lantern::poll_workspace_ipc);
    }
    if let Some(fd) = state.hdr_ipc.listener_fd() {
        register_listener(state, fd, "hdr", Lantern::poll_hdr_ipc);
    }
    if let Some(fd) = state.gaming_ipc.listener_fd() {
        register_listener(state, fd, "gaming", Lantern::poll_gaming_ipc);
    }
    if let Some(fd) = state.clipboard_ipc.listener_fd() {
        register_listener(state, fd, "clipboard", crate::clipboard_ipc::poll);
    }
    if let Some(fd) = state.cc_thumbs.listener_fd() {
        register_listener(state, fd, "cc-thumbs", Lantern::poll_cc_thumbs_ipc);
    }
    if let Some(fd) = state.hover_preview.listener_fd() {
        register_listener(state, fd, "hover", Lantern::poll_hover_preview_ipc);
    }
}
