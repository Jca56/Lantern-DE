//! Compositor-side clipboard persistence + history.
//!
//! Wayland's clipboard is source-based: when a client copies, the compositor
//! just remembers "client X owns the selection" and asks them for the bytes
//! on paste. If the source client disconnects, the selection becomes void.
//!
//! This module intercepts new selections, eagerly reads the bytes for a
//! set of useful mime types into an in-memory buffer, and re-publishes
//! the latest entry as a compositor-provided selection whenever the source
//! dies. That way pasting works after the source window closes, and we
//! get a clipboard history "for free" (see [`history`]).
//!
//! Caps memory at [`MAX_HISTORY`] entries x [`MAX_BYTES_PER_MIME`] bytes.
//! Selections that advertise password-manager hint mime types are not cached.

use std::{
    collections::{HashMap, VecDeque},
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime},
};

use smithay::{
    input::Seat,
    reexports::calloop::{
        generic::Generic,
        ping::{make_ping, Ping},
        timer::{TimeoutAction, Timer},
        Interest, Mode, PostAction,
    },
    wayland::selection::{
        data_device::{
            request_data_device_client_selection, set_data_device_selection,
            SelectionRequestError,
        },
        SelectionSource, SelectionTarget,
    },
};

use crate::Lantern;

/// Sentinel mime type used to probe the seat's current selection state via
/// [`request_data_device_client_selection`] without actually transferring
/// any data. No real client should advertise this.
const PROBE_MIME: &str = "application/x-lantern-clipboard-probe";

/// Process-wide ping that `ClientData::disconnected` uses to wake the main
/// loop so we can re-check whether the dead client owned the active selection.
pub static RECHECK_PING: OnceLock<Ping> = OnceLock::new();

const MAX_HISTORY: usize = 50;
const MAX_BYTES_PER_MIME: usize = 16 * 1024 * 1024;

/// Mime hints that mark password-manager / sensitive content; skip caching.
const SENSITIVE_HINTS: &[&str] = &[
    "x-kde-passwordManagerHint",
    "x-kde-onlyReplaceEmpty",
    "x/secret",
];

/// Mime types we eagerly capture. Listed in preference order — the first
/// one a source offers is treated as the canonical text for the history view.
const ACCEPTED_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
    "text/uri-list",
    "application/json",
    "image/png",
    "image/jpeg",
];

/// One captured clipboard payload — all the mime variants the source offered
/// us, with their raw bytes.
#[derive(Clone)]
pub struct ClipboardEntry {
    pub id: u64,
    pub mime_data: HashMap<String, Vec<u8>>,
    pub timestamp: SystemTime,
    /// Best-effort text representation for the history UI (None for image-only).
    pub preview: Option<String>,
    /// User-pinned entries skip the [`MAX_HISTORY`] LRU eviction.
    pub pinned: bool,
}

impl ClipboardEntry {
    pub fn mime_types(&self) -> Vec<String> {
        self.mime_data.keys().cloned().collect()
    }

    pub fn has_image(&self) -> bool {
        self.mime_data.keys().any(|m| m.starts_with("image/"))
    }
}

struct PendingEntry {
    entry: ClipboardEntry,
    mimes_remaining: usize,
}

pub struct ClipboardManager {
    /// History, newest first.
    history: VecDeque<Arc<ClipboardEntry>>,
    /// Entries currently being filled by async pipe reads.
    pending: HashMap<u64, PendingEntry>,
    next_id: u64,
    /// Bytes for the currently-published compositor selection (if any).
    /// Used by [`Self::on_send_request`] to fulfill paste requests.
    published: Option<Arc<ClipboardEntry>>,
    /// Re-entry guard for `set_data_device_selection`, which itself triggers
    /// a new_selection callback we want to ignore.
    publishing: bool,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self {
            history: VecDeque::with_capacity(MAX_HISTORY),
            pending: HashMap::new(),
            next_id: 1,
            published: None,
            publishing: false,
        }
    }

    /// True if the given entry id is what we currently have published
    /// (so a recheck doesn't bother re-publishing the same thing).
    fn already_publishing(&self, id: u64) -> bool {
        self.published.as_ref().map(|e| e.id) == Some(id)
    }

    /// Snapshot of the current history (newest first), cheap clone.
    pub fn history(&self) -> Vec<Arc<ClipboardEntry>> {
        self.history.iter().cloned().collect()
    }

    /// Total entries currently in history.
    pub fn len(&self) -> usize {
        self.history.len()
    }

    pub fn is_publishing(&self) -> bool {
        self.publishing
    }

    /// Look up cached bytes for the currently-published selection, for use
    /// from `SelectionHandler::send_selection`.
    pub fn bytes_for(&self, mime: &str) -> Option<Vec<u8>> {
        self.published
            .as_ref()
            .and_then(|e| e.mime_data.get(mime).cloned())
    }

    /// Remove an entry from history by id. Returns true if it was found.
    pub fn forget(&mut self, id: u64) -> bool {
        if let Some(pos) = self.history.iter().position(|e| e.id == id) {
            let removed = self.history.remove(pos);
            // Clean up the on-disk image cache for this entry if any.
            if let Some(e) = removed {
                let _ = std::fs::remove_file(image_cache_path(e.id));
            }
            true
        } else {
            false
        }
    }

    /// Toggle the pinned flag on `id`. Returns the new state, or `None`
    /// if the entry doesn't exist.
    pub fn set_pinned(&mut self, id: u64, value: bool) -> Option<bool> {
        let pos = self.history.iter().position(|e| e.id == id)?;
        let entry = &self.history[pos];
        // Arc<Entry> needs to be cloned-modified-replaced since we don't
        // hold &mut into the Arc itself.
        let mut new_entry = (**entry).clone();
        new_entry.pinned = value;
        self.history[pos] = Arc::new(new_entry);
        Some(value)
    }

    /// Find an entry in history and return it for republishing.
    pub fn entry_by_id(&self, id: u64) -> Option<Arc<ClipboardEntry>> {
        self.history.iter().find(|e| e.id == id).cloned()
    }

    /// Drop the whole history. Image cache files are cleaned up too.
    pub fn clear(&mut self) {
        for e in self.history.drain(..) {
            let _ = std::fs::remove_file(image_cache_path(e.id));
        }
    }

    /// Push a completed entry to the head of history, dropping the oldest
    /// non-pinned once we exceed [`MAX_HISTORY`]. Duplicates of the
    /// most-recent text entry are coalesced. Image bytes are mirrored to
    /// `/run/user/{uid}/lntrn-clipboard/{id}.png` so the CC's image cache
    /// can render thumbs without us shipping bytes over the IPC channel.
    fn push_history(&mut self, entry: ClipboardEntry) {
        if let (Some(latest), Some(new_text)) = (self.history.front(), entry.preview.as_ref()) {
            if latest.preview.as_deref() == Some(new_text.as_str()) {
                return;
            }
        }
        // Mirror image bytes to disk before we wrap in Arc.
        if let Some(bytes) = entry
            .mime_data
            .get("image/png")
            .or_else(|| entry.mime_data.get("image/jpeg"))
        {
            let path = image_cache_path(entry.id);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, bytes);
        }
        self.history.push_front(Arc::new(entry));
        // Evict from the back, skipping pinned entries.
        while self.history.len() > MAX_HISTORY {
            // Find the oldest non-pinned entry.
            let mut victim: Option<usize> = None;
            for (i, e) in self.history.iter().enumerate().rev() {
                if !e.pinned {
                    victim = Some(i);
                    break;
                }
            }
            match victim {
                Some(i) => {
                    let removed = self.history.remove(i);
                    if let Some(e) = removed {
                        let _ = std::fs::remove_file(image_cache_path(e.id));
                    }
                }
                None => break, // everything pinned — leave history slightly over cap
            }
        }
    }
}

/// Where we cache image bytes for the CC's thumbnail loader.
pub fn image_cache_path(id: u64) -> std::path::PathBuf {
    let uid = unsafe { libc::getuid() };
    std::path::PathBuf::from(format!("/run/user/{}/lntrn-clipboard/{}.png", uid, id))
}

/// Install the calloop ping source that drives [`recheck_clipboard`] from
/// `ClientData::disconnected`. Call once at compositor startup.
pub fn install_recheck_source(state: &mut Lantern) -> io::Result<()> {
    let (ping, source) = make_ping().map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    state
        .loop_handle
        .insert_source(source, |_, _, state: &mut Lantern| {
            recheck_clipboard(state);
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let _ = RECHECK_PING.set(ping);
    Ok(())
}

/// Probe the seat's current clipboard selection via a sentinel mime type
/// (no wayland traffic is generated) and re-publish from history if the
/// seat has been left empty by smithay's internal liveness purge.
pub fn recheck_clipboard(state: &mut Lantern) {
    let seat = state.seat.clone();

    // We have to hand `request_data_device_client_selection` an OwnedFd, but
    // for `InvalidMimetype`/`NoSelection` no actual write happens — `/dev/null`
    // is the cheapest fd we can produce.
    let probe_fd = match std::fs::OpenOptions::new().write(true).open("/dev/null") {
        Ok(f) => OwnedFd::from(f),
        Err(_) => return,
    };

    match request_data_device_client_selection(&seat, PROBE_MIME.to_string(), probe_fd) {
        Err(SelectionRequestError::NoSelection) => {
            eprintln!("[clipboard] recheck: seat empty -> republishing");
            republish_latest(state);
        }
        Err(SelectionRequestError::InvalidMimetype) => {
            // A live client owns the selection (just doesn't offer our probe). Leave alone.
        }
        Err(SelectionRequestError::ServerSideSelection) => {
            // We already published; nothing to do.
        }
        Ok(()) => {
            // Should never happen — no client offers the probe mime — but treat as live.
        }
    }
}

/// Begin capturing the contents of a new client selection. Called from
/// `SelectionHandler::new_selection`. Schedules an immediate timer so the
/// actual pipe reads happen after smithay has finished installing the
/// selection on the seat — we can then use the public
/// [`request_data_device_client_selection`] to issue SEND events to the
/// owning client without poking at smithay's private fields.
pub fn capture_selection(state: &mut Lantern, source: &SelectionSource) {
    if state.clipboard_manager.publishing {
        return;
    }

    let mimes = source.mime_types();
    eprintln!("[clipboard] capture_selection mimes={:?}", mimes);
    if mimes.iter().any(|m| is_sensitive(m)) {
        eprintln!("[clipboard] skipping (sensitive)");
        return;
    }

    let wanted: Vec<String> = mimes
        .into_iter()
        .filter(|m| ACCEPTED_MIMES.iter().any(|a| m.eq_ignore_ascii_case(a)))
        .collect();
    eprintln!("[clipboard] wanted={:?}", wanted);
    if wanted.is_empty() {
        return;
    }

    let id = state.clipboard_manager.next_id;
    state.clipboard_manager.next_id += 1;

    let entry = ClipboardEntry {
        id,
        mime_data: HashMap::new(),
        timestamp: SystemTime::now(),
        preview: None,
        pinned: false,
    };
    state.clipboard_manager.pending.insert(
        id,
        PendingEntry { entry, mimes_remaining: wanted.len() },
    );

    let timer = Timer::from_duration(Duration::ZERO);
    let _ = state.loop_handle.insert_source(timer, move |_, _, state: &mut Lantern| {
        start_pending_reads(state, id, &wanted);
        TimeoutAction::Drop
    });
}

fn start_pending_reads(state: &mut Lantern, id: u64, mimes: &[String]) {
    let seat = state.seat.clone();
    eprintln!("[clipboard] start_pending_reads id={} mimes={:?}", id, mimes);

    let mut started = 0usize;
    for mime in mimes {
        let Some((read_fd, write_fd)) = make_pipe() else { continue };
        set_nonblocking(&read_fd);

        match request_data_device_client_selection(&seat, mime.clone(), write_fd) {
            Ok(()) => eprintln!("[clipboard] requested mime={}", mime),
            Err(e) => {
                eprintln!("[clipboard] request failed mime={}: {:?}", mime, e);
                continue;
            }
        }

        let mime_for_cb = mime.clone();
        let mut buf: Vec<u8> = Vec::new();
        let res = state.loop_handle.insert_source(
            Generic::new(read_fd, Interest::READ, Mode::Level),
            move |_readiness, fd, state: &mut Lantern| {
                let raw = fd.as_raw_fd();
                let mut chunk = [0u8; 8192];
                loop {
                    let n = unsafe {
                        libc::read(raw, chunk.as_mut_ptr() as *mut _, chunk.len())
                    };
                    if n > 0 {
                        if buf.len() + (n as usize) > MAX_BYTES_PER_MIME {
                            complete_mime(state, id, &mime_for_cb, Vec::new());
                            return Ok(PostAction::Remove);
                        }
                        buf.extend_from_slice(&chunk[..n as usize]);
                        continue;
                    }
                    if n == 0 {
                        complete_mime(state, id, &mime_for_cb, std::mem::take(&mut buf));
                        return Ok(PostAction::Remove);
                    }
                    let err = std::io::Error::last_os_error();
                    match err.kind() {
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted => {
                            return Ok(PostAction::Continue);
                        }
                        _ => {
                            complete_mime(state, id, &mime_for_cb, std::mem::take(&mut buf));
                            return Ok(PostAction::Remove);
                        }
                    }
                }
            },
        );
        if res.is_ok() {
            started += 1;
        }
    }

    // If no reads got off the ground, drop the pending entry.
    if started == 0 {
        state.clipboard_manager.pending.remove(&id);
        return;
    }
    // Adjust the expected count to what actually started.
    if let Some(p) = state.clipboard_manager.pending.get_mut(&id) {
        p.mimes_remaining = started;
    }
}

fn complete_mime(state: &mut Lantern, id: u64, mime: &str, bytes: Vec<u8>) {
    eprintln!("[clipboard] complete_mime id={} mime={} bytes={}", id, mime, bytes.len());
    if let Some(p) = state.clipboard_manager.pending.get_mut(&id) {
        if !bytes.is_empty() {
            p.entry.mime_data.insert(mime.to_string(), bytes);
        }
        p.mimes_remaining = p.mimes_remaining.saturating_sub(1);
    }
    finalize_if_ready(state, id);
}

fn finalize_if_ready(state: &mut Lantern, id: u64) {
    let ready = state
        .clipboard_manager
        .pending
        .get(&id)
        .map(|p| p.mimes_remaining == 0)
        .unwrap_or(false);
    if !ready {
        return;
    }
    let Some(p) = state.clipboard_manager.pending.remove(&id) else { return };
    let mut entry = p.entry;
    if entry.mime_data.is_empty() {
        eprintln!("[clipboard] entry id={} discarded (no data)", entry.id);
        return;
    }
    entry.preview = best_text_preview(&entry.mime_data);
    let preview_summary = entry
        .preview
        .as_deref()
        .map(|s| s.chars().take(40).collect::<String>())
        .unwrap_or_else(|| "<binary>".to_string());
    eprintln!(
        "[clipboard] history push id={} mimes={:?} preview={:?}",
        entry.id,
        entry.mime_data.keys().collect::<Vec<_>>(),
        preview_summary
    );
    state.clipboard_manager.push_history(entry);
}

/// Called when smithay reports the clipboard selection became `None` —
/// i.e. the source client cleared it or disconnected. We re-publish the
/// most recent history entry so paste still works.
pub fn on_selection_cleared(state: &mut Lantern) {
    eprintln!(
        "[clipboard] on_selection_cleared (publishing={}, history_len={})",
        state.clipboard_manager.publishing,
        state.clipboard_manager.history.len()
    );
    if state.clipboard_manager.publishing {
        return;
    }
    republish_latest(state);
}

/// Publish a specific history entry as the active selection. Used by IPC
/// when the user picks an item from the clipboard view.
pub fn republish(state: &mut Lantern, id: u64) -> bool {
    let Some(entry) = state
        .clipboard_manager
        .history
        .iter()
        .find(|e| e.id == id)
        .cloned()
    else {
        return false;
    };
    publish_entry(state, entry);
    true
}

fn republish_latest(state: &mut Lantern) {
    let Some(entry) = state.clipboard_manager.history.front().cloned() else {
        state.clipboard_manager.published = None;
        return;
    };
    publish_entry(state, entry);
}

fn publish_entry(state: &mut Lantern, entry: Arc<ClipboardEntry>) {
    let mimes = entry.mime_types();
    eprintln!(
        "[clipboard] publish_entry id={} mimes={:?}",
        entry.id, mimes
    );
    if mimes.is_empty() {
        return;
    }
    state.clipboard_manager.publishing = true;
    state.clipboard_manager.published = Some(entry);
    let seat: Seat<Lantern> = state.seat.clone();
    let dh = state.display_handle.clone();
    set_data_device_selection::<Lantern>(&dh, &seat, mimes, ());
    state.clipboard_manager.publishing = false;
}

/// Called from `SelectionHandler::send_selection` when a client is pasting
/// the compositor-provided (re-published) selection. Writes the cached
/// bytes for `mime` to `fd`.
pub fn on_send_request(state: &mut Lantern, ty: SelectionTarget, mime: String, fd: OwnedFd) {
    eprintln!("[clipboard] on_send_request ty={:?} mime={}", ty, mime);
    if !matches!(ty, SelectionTarget::Clipboard) {
        return;
    }
    let bytes = match state.clipboard_manager.bytes_for(&mime) {
        Some(b) => {
            eprintln!("[clipboard]   serving {} bytes", b.len());
            b
        }
        None => {
            eprintln!("[clipboard]   no cached bytes for {}", mime);
            return;
        }
    };
    // Off-thread so a slow paster can't block the compositor.
    std::thread::spawn(move || {
        let mut file = std::fs::File::from(fd);
        let _ = std::io::Write::write_all(&mut file, &bytes);
    });
}

fn make_pipe() -> Option<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe2 writes two valid file descriptors into the array on success.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        let _ = io::Error::last_os_error();
        return None;
    }
    // SAFETY: pipe2 succeeded, so both fds are valid and owned.
    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Some((read_fd, write_fd))
}

fn is_sensitive(m: &str) -> bool {
    SENSITIVE_HINTS.iter().any(|h| m.contains(h))
}

fn set_nonblocking(fd: &OwnedFd) {
    let raw = fd.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL, 0);
        if flags >= 0 {
            libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

fn best_text_preview(mime_data: &HashMap<String, Vec<u8>>) -> Option<String> {
    const TEXT_MIMES: &[&str] = &[
        "text/plain;charset=utf-8",
        "text/plain",
        "UTF8_STRING",
        "STRING",
        "TEXT",
        "text/uri-list",
        "application/json",
    ];
    for m in TEXT_MIMES {
        if let Some(bytes) = mime_data.get(*m) {
            if let Ok(s) = std::str::from_utf8(bytes) {
                return Some(s.to_string());
            }
        }
    }
    None
}
