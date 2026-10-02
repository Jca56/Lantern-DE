// Blob transfers, streamed and verified.
//
// Blobs are named by the sha256 of their content, and everything else trusts
// that name: the other machine downloads "the blob called H" and records H as
// what it now has. So a blob must never hold bytes that do not hash to its
// name, and a download must never be accepted without checking.
//
// Upload: the hash in hand was computed by the scan, possibly minutes ago
// (or taken from the stat cache). `BlobSource` re-hashes the bytes as they
// are read for sending and HOLDS BACK THE LAST BYTES until the whole file has
// been hashed. If the file is no longer what the scan saw, the reader fails
// before the body is complete, the request is cut off short of its declared
// length and the server stores nothing. The path is then left for the next
// pass, which scans it again.
//
// Download: `HashingWriter` hashes what is written to the temp file; the
// caller compares before the rename and throws the temp file away on a
// mismatch.
//
// Neither direction holds a file in memory.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::hash::Hasher;

/// A local file to be stored as the blob named `sha256`.
pub struct BlobSource<'a> {
    pub path: &'a Path,
    pub sha256: &'a str,
    /// How many bytes hash to `sha256`: exactly this many are sent.
    pub size: u64,
    /// Set by a reader that found other bytes than `sha256` promises.
    changed: AtomicBool,
}

impl<'a> BlobSource<'a> {
    pub fn new(path: &'a Path, sha256: &'a str, size: u64) -> Self {
        Self {
            path,
            sha256,
            size,
            changed: AtomicBool::new(false),
        }
    }

    /// A reader over the file that gives out `size` bytes hashing to
    /// `sha256`, or fails before the last of them.
    pub fn open(&self) -> std::io::Result<VerifiedReader<'_>> {
        Ok(VerifiedReader {
            src: self,
            file: std::fs::File::open(self.path)?,
            hasher: Some(Hasher::new()),
            left: self.size,
            tail: Vec::new(),
            tail_pos: 0,
        })
    }

    /// Did an upload fail because the file is not what the scan hashed?
    /// (As opposed to: the network, the server.)
    pub fn changed(&self) -> bool {
        self.changed.load(Ordering::Relaxed)
    }
}

pub struct VerifiedReader<'a> {
    src: &'a BlobSource<'a>,
    file: std::fs::File,
    /// `None` once the hash has been checked.
    hasher: Option<Hasher>,
    /// Bytes of the file not read yet.
    left: u64,
    /// The last bytes, read and verified, not yet handed out.
    tail: Vec<u8>,
    tail_pos: usize,
}

impl VerifiedReader<'_> {
    fn changed(&self) -> std::io::Error {
        self.src.changed.store(true, Ordering::Relaxed);
        std::io::Error::other(format!(
            "{} changed while it was being uploaded",
            self.src.path.display()
        ))
    }

    fn hand_out_tail(&mut self, buf: &mut [u8]) -> usize {
        let rest = &self.tail[self.tail_pos..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.tail_pos += n;
        n
    }
}

impl Read for VerifiedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.hasher.is_none() {
            return Ok(self.hand_out_tail(buf));
        }
        if self.left > buf.len() as u64 {
            // More than this read's worth still to come: nothing handed out
            // here can be the end of the body.
            let n = self.file.read(buf)?;
            if n == 0 {
                // Shorter than the scan saw.
                return Err(self.changed());
            }
            if let Some(h) = self.hasher.as_mut() {
                h.update(&buf[..n]);
            }
            self.left -= n as u64;
            return Ok(n);
        }
        // The rest fits into this read. Take it all in and check the hash
        // before any of it goes out.
        let mut tail = vec![0u8; self.left as usize];
        if let Err(e) = self.file.read_exact(&mut tail) {
            return Err(if e.kind() == std::io::ErrorKind::UnexpectedEof {
                self.changed()
            } else {
                e
            });
        }
        let mut hasher = self.hasher.take().unwrap_or_default();
        hasher.update(&tail);
        self.left = 0;
        if hasher.finish() != self.src.sha256 {
            // `hasher` is gone, so later reads hand out the (empty) tail:
            // the body stays incomplete whatever the caller does next.
            return Err(self.changed());
        }
        self.tail = tail;
        Ok(self.hand_out_tail(buf))
    }
}

/// Counts and hashes what goes through to `inner`.
pub struct HashingWriter<W: Write> {
    inner: W,
    hasher: Hasher,
    written: u64,
}

impl<W: Write> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Hasher::new(),
            written: 0,
        }
    }

    /// (sha256 hex, byte count) of everything written.
    pub fn finish(self) -> (String, u64) {
        (self.hasher.finish(), self.written)
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::super::hash::sha256_bytes;
    use super::super::TestDir;
    use super::*;

    /// What an HTTP client does with a body: read it in pieces of `piece`
    /// bytes, counting what arrived before any error.
    fn send(src: &BlobSource, piece: usize) -> (Vec<u8>, std::io::Result<()>) {
        let mut sent = Vec::new();
        let mut reader = match src.open() {
            Ok(r) => r,
            Err(e) => return (sent, Err(e)),
        };
        let mut buf = vec![0u8; piece];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return (sent, Ok(())),
                Ok(n) => sent.extend_from_slice(&buf[..n]),
                Err(e) => return (sent, Err(e)),
            }
        }
    }

    fn content(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn an_unchanged_file_goes_out_whole() {
        let dir = TestDir::new("transfer-whole");
        let path = dir.join("f.bin");
        for len in [0usize, 1, 7, 4096, 10_000] {
            let bytes = content(len);
            std::fs::write(&path, &bytes).unwrap();
            let sha = sha256_bytes(&bytes);
            for piece in [1usize, 3, 4096, 64 * 1024] {
                let src = BlobSource::new(&path, &sha, len as u64);
                let (sent, result) = send(&src, piece);
                assert!(result.is_ok(), "len {len} piece {piece}");
                assert_eq!(sent, bytes, "len {len} piece {piece}");
                assert!(!src.changed());
            }
        }
    }

    #[test]
    fn a_changed_file_never_completes_the_body() {
        let dir = TestDir::new("transfer-changed");
        let path = dir.join("f.bin");
        let scanned = content(10_000);
        let sha = sha256_bytes(&scanned);

        // Same length, one byte different near the start, near the end.
        for flip in [0usize, 5_000, 9_999] {
            let mut now = scanned.clone();
            now[flip] ^= 0xff;
            std::fs::write(&path, &now).unwrap();
            for piece in [1usize, 1000, 4096, 64 * 1024] {
                let src = BlobSource::new(&path, &sha, scanned.len() as u64);
                let (sent, result) = send(&src, piece);
                assert!(result.is_err(), "flip {flip} piece {piece}");
                assert!(src.changed());
                // Whatever went out, it was not the whole declared length:
                // the server has nothing to store.
                assert!(sent.len() < scanned.len(), "flip {flip} piece {piece}");
            }
        }

        // Truncated since the scan.
        std::fs::write(&path, &scanned[..4_000]).unwrap();
        let src = BlobSource::new(&path, &sha, scanned.len() as u64);
        let (sent, result) = send(&src, 1000);
        assert!(result.is_err() && src.changed());
        assert!(sent.len() < scanned.len());

        // Gone since the scan: an error, but not "changed under the upload".
        std::fs::remove_file(&path).unwrap();
        let src = BlobSource::new(&path, &sha, scanned.len() as u64);
        let (sent, result) = send(&src, 1000);
        assert!(result.is_err() && sent.is_empty());
    }

    #[test]
    fn a_file_that_grew_is_sent_as_the_scan_saw_it() {
        // Appended to after the scan: the first `size` bytes are still the
        // bytes the hash names, and exactly those go out. The longer file is
        // picked up by the next scan.
        let dir = TestDir::new("transfer-grew");
        let path = dir.join("log.txt");
        let scanned = content(5_000);
        let sha = sha256_bytes(&scanned);
        let mut now = scanned.clone();
        now.extend_from_slice(b"one more line\n");
        std::fs::write(&path, &now).unwrap();
        let src = BlobSource::new(&path, &sha, scanned.len() as u64);
        let (sent, result) = send(&src, 4096);
        assert!(result.is_ok());
        assert_eq!(sent, scanned);
    }

    #[test]
    fn a_failed_reader_stays_failed() {
        let dir = TestDir::new("transfer-retry");
        let path = dir.join("f.bin");
        std::fs::write(&path, b"other bytes").unwrap();
        let sha = sha256_bytes(b"scan's bytes");
        let src = BlobSource::new(&path, &sha, 11);
        let mut reader = src.open().unwrap();
        let mut buf = [0u8; 64];
        assert!(reader.read(&mut buf).is_err());
        // A caller that reads on after the error gets no more bytes.
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn the_writer_hashes_what_it_passes_on() {
        let mut out = Vec::new();
        let mut w = HashingWriter::new(&mut out);
        w.write_all(b"hello ").unwrap();
        w.write_all(b"world").unwrap();
        let (sha, len) = w.finish();
        assert_eq!(out, b"hello world");
        assert_eq!(len, 11);
        assert_eq!(sha, sha256_bytes(b"hello world"));
    }
}
