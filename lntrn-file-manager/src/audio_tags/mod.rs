//! Audio tag engine — our own ID3v2 / RIFF-INFO / MPEG-frame parsers and
//! writers, no external crates. WAV and MP3 are simple enough that a few
//! hundred lines beat a dependency.
//!
//! * [`read`] sniffs the container by extension and returns tags + stream facts.
//! * [`write`] touches only metadata: WAV tags live in trailing chunks (O(1)
//!   tail append even on a 600 MB set), MP3 tags live in the leading ID3v2
//!   block (rewritten in place when the new tag fits the old padding).
//!
//! The rule for every writer here: audio bytes are never lost, only the
//! fields the user changed are changed on disk, and anything a parser did
//! not understand is either carried through byte for byte or the save is
//! refused with a message that says why. Refusing is always fine; dropping
//! data silently never is.

pub mod genres;
pub mod id3;
pub mod id3v1;
pub mod keys;
pub mod mp3;
pub mod wav;

#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Editable tags. Empty string = unset; `artwork: None` = no picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub track: String,
    pub bpm: String,
    pub key: String,
    pub artwork: Option<Artwork>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artwork {
    pub mime: String,
    pub data: Vec<u8>,
}

impl AudioTags {
    /// Equal as far as an edit is concerned: surrounding blanks do not count.
    pub fn same_as(&self, other: &AudioTags) -> bool {
        let pairs = [
            (&self.title, &other.title),
            (&self.artist, &other.artist),
            (&self.album, &other.album),
            (&self.year, &other.year),
            (&self.genre, &other.genre),
            (&self.track, &other.track),
            (&self.bpm, &other.bpm),
            (&self.key, &other.key),
        ];
        pairs.iter().all(|(a, b)| !changed(a, b)) && self.artwork == other.artwork
    }
}

/// Whether the user changed a text field.
pub(crate) fn changed(was: &str, now: &str) -> bool {
    was.trim() != now.trim()
}

const REFUSED: &str = "Fox left the file as it was.";

/// The error for a save that was refused before anything was written.
pub(crate) fn refusal(why: &str) -> String {
    format!("{why}. {REFUSED}")
}

/// Whether a [`write`] error is a refusal (the file is untouched) rather
/// than a write that failed part-way.
pub fn is_refusal(err: &str) -> bool {
    err.ends_with(REFUSED)
}

pub fn sniff_image_mime(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else if b.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// Read-only stream facts — shown, never written.
#[derive(Clone, Debug, Default)]
pub struct AudioFormat {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: Option<u16>,
    pub bitrate_kbps: Option<u32>,
    pub vbr: bool,
    pub duration_secs: Option<f64>,
}

impl AudioFormat {
    /// "2:13 · 48 kHz · 16-bit · Stereo · PCM"
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(d) = self.duration_secs {
            parts.push(format_duration(d));
        }
        if self.sample_rate > 0 {
            parts.push(format_sample_rate(self.sample_rate));
        }
        if let Some(b) = self.bits_per_sample {
            parts.push(format!("{b}-bit"));
        }
        if let Some(k) = self.bitrate_kbps {
            parts.push(if self.vbr {
                format!("VBR ~{k} kbps")
            } else {
                format!("{k} kbps")
            });
        }
        match self.channels {
            0 => {}
            1 => parts.push("Mono".into()),
            2 => parts.push("Stereo".into()),
            n => parts.push(format!("{n} ch")),
        }
        if !self.codec.is_empty() {
            parts.push(self.codec.clone());
        }
        parts.join(" · ")
    }
}

pub fn format_duration(secs: f64) -> String {
    let total = secs.round().max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn format_sample_rate(sr: u32) -> String {
    if sr % 1000 == 0 {
        format!("{} kHz", sr / 1000)
    } else {
        format!("{:.1} kHz", sr as f64 / 1000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Wav,
    Mp3,
}

#[derive(Clone, Debug)]
pub struct AudioMeta {
    #[allow(dead_code)] // informative; the UI keys off the summary string
    pub container: Container,
    pub tags: AudioTags,
    pub format: AudioFormat,
    /// Set when [`write`] would refuse this file, with the reason: the
    /// editor says so up front instead of after the user typed their edits.
    pub write_blocker: Option<String>,
}

pub fn container_for(path: &Path) -> Option<Container> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "wav" | "wave" => Some(Container::Wav),
        "mp3" => Some(Container::Mp3),
        _ => None,
    }
}

/// Tag code opens what it is given, and the name alone decides that. A FIFO
/// called `x.mp3` blocks forever on open; this runs on thumbnail workers for
/// every audio file in a folder being browsed.
fn require_regular_file(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err("Not a regular file".into()),
        Err(e) => Err(io_err(e)),
    }
}

pub fn read(path: &Path) -> Result<AudioMeta, String> {
    require_regular_file(path)?;
    match container_for(path).ok_or("Unsupported audio format")? {
        Container::Wav => wav::read(path),
        Container::Mp3 => mp3::read(path),
    }
}

/// Save the editor's changes. `shown` is what the editor loaded and showed;
/// `tags` is what it holds now. Only the fields that differ between the two
/// are written, onto whatever the file holds at this moment: if another
/// program (cloud sync, a tagger) changed the file's tags while the dialog
/// was open, those changes stay.
pub fn write_from(path: &Path, shown: &AudioTags, tags: &AudioTags) -> Result<(), String> {
    require_regular_file(path)?;
    let done = match container_for(path).ok_or("Unsupported audio format")? {
        Container::Wav => wav::write_from(path, Some(shown), tags),
        Container::Mp3 => mp3::write_from(path, Some(shown), tags),
    };
    if done.is_ok() {
        remember_own_save(path);
    }
    done
}

/// The file Fox itself saved last, and its modification time then. Where
/// the kernel cannot say whether somebody has a file open for writing,
/// `writer_check` goes by how recently it changed, and Fox's own save a
/// moment ago is not "another program still writing".
static OWN_SAVE: std::sync::Mutex<Option<(PathBuf, std::time::SystemTime)>> =
    std::sync::Mutex::new(None);

fn remember_own_save(path: &Path) {
    let stamp = std::fs::canonicalize(path)
        .ok()
        .zip(std::fs::metadata(path).and_then(|m| m.modified()).ok());
    *OWN_SAVE.lock().unwrap_or_else(|e| e.into_inner()) = stamp;
}

fn is_own_save(path: &Path, f: &File) -> bool {
    let saved = OWN_SAVE.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::fs::canonicalize(path)
        .ok()
        .zip(f.metadata().and_then(|m| m.modified()).ok());
    saved.is_some() && *saved == now
}

/// Make the file's tags `tags`: every field that differs from what the file
/// holds now is written.
#[cfg(test)]
pub fn write(path: &Path, tags: &AudioTags) -> Result<(), String> {
    require_regular_file(path)?;
    match container_for(path).ok_or("Unsupported audio format")? {
        Container::Wav => wav::write(path, tags),
        Container::Mp3 => mp3::write(path, tags),
    }
}

/// What a file looked like when it was read: a save goes ahead only if it
/// still looks like that.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Seen {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl Seen {
    pub(crate) fn of(f: &File) -> io::Result<Self> {
        let meta = f.metadata()?;
        Ok(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

pub(crate) const CHANGED_MEANWHILE: &str = "The file changed while it was being saved";

/// Refuse a file another program has open for writing: a recording still
/// running, a copy or a download not finished. Saving tags then would
/// either cut the file off where it stands (the writer goes on writing into
/// a file that no longer has a name) or stamp a length into its header that
/// is wrong the moment the writer continues.
///
/// Asked of the kernel: a read lease is only granted on a file nobody has
/// open for writing. Where leases are not available (some network and FUSE
/// filesystems, a file owned by someone else) the answer is "unknown", and
/// the callers fall back on how recently the file changed.
pub(crate) fn writer_check(path: &Path, f: &File) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let fd = f.as_raw_fd();
    // A lease is broken by a signal, SIGIO unless told otherwise, and
    // SIGIO's default is to end the process. SIGURG is ignored by default.
    // (Linux's F_SETSIG, which the libc crate does not name.)
    const F_SETSIG: libc::c_int = 10;
    let granted = unsafe {
        libc::fcntl(fd, F_SETSIG, libc::SIGURG);
        libc::fcntl(fd, libc::F_SETLEASE, libc::F_RDLCK)
    };
    if granted == 0 {
        unsafe {
            libc::fcntl(fd, libc::F_SETLEASE, libc::F_UNLCK);
        }
        return Ok(());
    }
    let busy = io::Error::last_os_error().raw_os_error() == Some(libc::EAGAIN);
    if busy || (recently_modified(f) && !is_own_save(path, f)) {
        return Err(refusal("Another program is still writing this file"));
    }
    Ok(())
}

/// The fallback for `writer_check`: written to within the last few seconds.
fn recently_modified(f: &File) -> bool {
    const QUIET: std::time::Duration = std::time::Duration::from_secs(10);
    f.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| at.elapsed().ok())
        .is_some_and(|age| age < QUIET)
}

pub(crate) fn io_err(e: io::Error) -> String {
    e.to_string()
}

/// Carry over what a rewrite would otherwise lose: the owner, and the
/// extended attributes (a download's origin, ACLs, labels). Best effort: an
/// attribute the destination will not take is skipped, never an error.
fn carry_attributes(src: &File, dst: &File) {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::AsRawFd;
    let (from, to) = (src.as_raw_fd(), dst.as_raw_fd());
    if let Ok(meta) = src.metadata() {
        // Refused unless we are root or already the owner; either is fine.
        // The group alone can often still be kept (a shared music folder).
        unsafe {
            if libc::fchown(to, meta.uid(), meta.gid()) != 0 {
                libc::fchown(to, u32::MAX, meta.gid());
            }
        }
    }
    let size = unsafe { libc::flistxattr(from, std::ptr::null_mut(), 0) };
    if size <= 0 {
        return;
    }
    let mut names = vec![0u8; size as usize];
    let got = unsafe { libc::flistxattr(from, names.as_mut_ptr().cast(), names.len()) };
    if got <= 0 {
        return;
    }
    names.truncate(got as usize);
    for name in names.split(|b| *b == 0).filter(|n| !n.is_empty()) {
        let Ok(c_name) = std::ffi::CString::new(name) else {
            continue;
        };
        let len = unsafe { libc::fgetxattr(from, c_name.as_ptr(), std::ptr::null_mut(), 0) };
        if len < 0 {
            continue;
        }
        let mut value = vec![0u8; len as usize];
        let read =
            unsafe { libc::fgetxattr(from, c_name.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
        if read < 0 {
            continue;
        }
        unsafe {
            libc::fsetxattr(to, c_name.as_ptr(), value.as_ptr().cast(), read as usize, 0);
        }
    }
}

/// Replace `path` via a sibling temp file + rename so a crash mid-write never
/// leaves a half-written audio file behind. `fill` gets (temp, original).
///
/// The new file is the old one in everything but its bytes: same mode,
/// owner and extended attributes. What cannot be kept is a second name: a
/// file with hard links is refused, because the rename would leave the
/// other names pointing at the old content without a word.
pub(crate) fn replace_file(
    path: &Path,
    fill: impl FnOnce(&mut File, &mut File) -> io::Result<()>,
) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);

    // Work on the real file. Renaming the temp onto a symlink would replace
    // the LINK with a private copy and leave the library file untouched.
    let real = std::fs::canonicalize(path).map_err(io_err)?;
    let dir = real.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Unique per save: two saves of the same file (dialog closed and
    // reopened mid-save) must not write through one temp, and a temp left
    // by a crash must not block the next save.
    let tmp: PathBuf = dir.join(format!(
        ".{name}.{}.{}.lntrn-tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> io::Result<()> {
        let mut src = File::open(&real)?;
        let before = Seen::of(&src)?;
        let meta = src.metadata()?;
        if meta.nlink() > 1 {
            return Err(io::Error::other(refusal(
                "This file has more than one name (hard links), and saving these tags would split them",
            )));
        }
        let perms = meta.permissions();
        // create_new: never follow a symlink someone planted at the temp
        // name. The mode is set at creation, not after the data is in.
        let mut dst = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(perms.mode())
            .open(&tmp)?;
        fill(&mut dst, &mut src)?;
        dst.flush()?;
        carry_attributes(&src, &dst);
        // Last: the umask may have trimmed the creation mode, and a change
        // of owner clears the setuid/setgid bits.
        std::fs::set_permissions(&tmp, perms)?;
        dst.sync_all()?;
        // Whatever wrote to the original while it was being copied is not
        // in the copy: the original stays.
        if Seen::of(&src)? != before {
            return Err(io::Error::other(refusal(CHANGED_MEANWHILE)));
        }
        std::fs::rename(&tmp, &real)?;
        // The rename itself is only durable once the folder is.
        if let Ok(folder) = File::open(&dir) {
            let _ = folder.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(io_err)
}
