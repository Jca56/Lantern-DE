//! LSP stdio framing: `Content-Length: N\r\n\r\n<N bytes>`. No allocations
//! beyond the message buffer itself; headers are parsed byte-by-byte.

use std::io::{self, BufRead, Write};

/// Read one framed LSP message. Returns `Ok(None)` on clean EOF.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut content_len: Option<usize> = None;
    let mut line = Vec::with_capacity(64);

    // ── Headers ──────────────────────────────────────────────────────
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            // EOF before we saw the header terminator.
            return Ok(None);
        }
        // Strip CR/LF.
        while matches!(line.last(), Some(b'\r') | Some(b'\n')) {
            line.pop();
        }
        if line.is_empty() {
            break; // Blank line = end of headers.
        }
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue; // Skip malformed header, be lenient.
        };
        let name = &line[..colon];
        let value = line[colon + 1..].trim_ascii_start();
        if name.eq_ignore_ascii_case(b"Content-Length") {
            let s = std::str::from_utf8(value)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            content_len = Some(s.parse::<usize>().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("bad length: {e}"))
            })?);
        }
        // Content-Type is ignored — LSP always uses utf-8 JSON.
    }

    let len = content_len.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Write a framed message. The caller owns the JSON body.
pub fn write_message<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

// ── trim_ascii_start polyfill (stable since 1.80 — keep local for wider MSRV) ──

#[allow(dead_code)]
trait TrimAsciiStart {
    fn trim_ascii_start(&self) -> &[u8];
}
impl TrimAsciiStart for [u8] {
    fn trim_ascii_start(&self) -> &[u8] {
        let mut i = 0;
        while i < self.len() && self[i].is_ascii_whitespace() {
            i += 1;
        }
        &self[i..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip() {
        let mut buf = Vec::new();
        write_message(&mut buf, b"{\"hi\":1}").unwrap();
        let mut cur = Cursor::new(buf);
        let got = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(got, b"{\"hi\":1}");
    }

    #[test]
    fn tolerates_extra_headers() {
        let raw = b"Content-Type: utf-8\r\nContent-Length: 3\r\n\r\nabc";
        let mut cur = Cursor::new(&raw[..]);
        let got = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(got, b"abc");
    }
}
