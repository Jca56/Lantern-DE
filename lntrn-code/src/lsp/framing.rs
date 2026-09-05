//! LSP over stdio: `Content-Length: N\r\n\r\n` then N bytes of JSON.

use std::io::{self, BufRead, Write};

/// One framed message; `Ok(None)` at a clean end of the stream.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut length: Option<usize> = None;
    let mut line = Vec::with_capacity(64);
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }
        while matches!(line.last(), Some(b'\r' | b'\n')) {
            line.pop();
        }
        if line.is_empty() {
            break;
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            continue;
        };
        if line[..colon].eq_ignore_ascii_case(b"Content-Length") {
            let value = String::from_utf8_lossy(&line[colon + 1..]);
            length = Some(value.trim().parse().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad Content-Length: {e}")))?);
        }
    }
    let len = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Content-Length"))?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

pub fn write_message<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_and_extra_headers() {
        let mut buf = Vec::new();
        write_message(&mut buf, b"{\"a\":1}").unwrap();
        write_message(&mut buf, b"[]").unwrap();
        let mut cur = Cursor::new(buf);
        assert_eq!(read_message(&mut cur).unwrap().unwrap(), b"{\"a\":1}");
        assert_eq!(read_message(&mut cur).unwrap().unwrap(), b"[]");
        assert!(read_message(&mut cur).unwrap().is_none());
        let raw = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 3\r\n\r\nabc";
        assert_eq!(read_message(&mut Cursor::new(&raw[..])).unwrap().unwrap(), b"abc");
        assert!(read_message(&mut Cursor::new(&b"X: 1\r\n\r\n"[..])).is_err());
    }
}
