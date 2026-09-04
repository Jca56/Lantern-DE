//! A WebSocket server end (RFC 6455): the HTTP upgrade with the bearer
//! header Claude Code sends, and framing both ways. Client frames are
//! masked; ours are not. Fragments are joined before a message is
//! reported.

use std::io::{self, Read, Write};
use std::net::TcpStream;

use super::sha1::{base64, sha1};

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
/// The header the CLI presents its token in.
pub const AUTH_HEADER: &str = "x-claude-code-ide-authorization";
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

pub enum Message {
    Text(String),
    Binary,
    Ping(Vec<u8>),
    Pong,
    Close,
}

/// Read the client's upgrade request and answer it. `token` must match
/// the authorization header or the connection is refused.
pub fn handshake(stream: &mut TcpStream, token: &str) -> io::Result<()> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 || buf.len() > 16 * 1024 {
            return Err(io::Error::other("bad upgrade request"));
        }
        buf.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&buf);
    let mut key = None;
    let mut upgrade = false;
    let mut auth = None;
    let mut protocol: Option<String> = None;
    for line in text.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "sec-websocket-key" => key = Some(value.to_owned()),
            "upgrade" if value.eq_ignore_ascii_case("websocket") => upgrade = true,
            n if n == AUTH_HEADER => auth = Some(value.to_owned()),
            // The MCP client asks for its subprotocol; a server that does
            // not answer with it is hung up on at once.
            "sec-websocket-protocol" => protocol = value.split(',').next().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()),
            _ => {}
        }
    }
    if auth.as_deref() != Some(token) {
        let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        return Err(io::Error::other("wrong or missing authorization"));
    }
    let (Some(key), true) = (key, upgrade) else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return Err(io::Error::other("not a websocket upgrade"));
    };
    let accept = base64(&sha1(format!("{key}{GUID}").as_bytes()));
    let proto = protocol.map(|p| format!("Sec-WebSocket-Protocol: {p}\r\n")).unwrap_or_default();
    stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{proto}\r\n").as_bytes())
}

fn read_exact(stream: &mut TcpStream, n: usize) -> io::Result<Vec<u8>> {
    let mut v = vec![0u8; n];
    stream.read_exact(&mut v)?;
    Ok(v)
}

/// One frame: `(fin, opcode, payload)`, unmasked.
fn read_frame(stream: &mut TcpStream) -> io::Result<(bool, u8, Vec<u8>)> {
    let head = read_exact(stream, 2)?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let mut len = usize::from(head[1] & 0x7F);
    if len == 126 {
        let b = read_exact(stream, 2)?;
        len = usize::from(u16::from_be_bytes([b[0], b[1]]));
    } else if len == 127 {
        let b = read_exact(stream, 8)?;
        len = usize::try_from(u64::from_be_bytes(b.try_into().expect("8 bytes"))).map_err(|_| io::Error::other("frame too large"))?;
    }
    if len > MAX_MESSAGE {
        return Err(io::Error::other("frame too large"));
    }
    let mask = if masked { Some(read_exact(stream, 4)?) } else { None };
    let mut payload = read_exact(stream, len)?;
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    Ok((fin, opcode, payload))
}

/// The next whole message; control frames come through as they are.
pub fn read_message(stream: &mut TcpStream) -> io::Result<Message> {
    let mut data: Vec<u8> = Vec::new();
    let mut kind = 0u8;
    loop {
        let (fin, opcode, payload) = read_frame(stream)?;
        match opcode {
            0x8 => return Ok(Message::Close),
            0x9 => return Ok(Message::Ping(payload)),
            0xA => return Ok(Message::Pong),
            0x1 | 0x2 => {
                kind = opcode;
                data = payload;
            }
            0x0 => data.extend_from_slice(&payload),
            _ => return Err(io::Error::other("unknown opcode")),
        }
        if data.len() > MAX_MESSAGE {
            return Err(io::Error::other("message too large"));
        }
        if fin {
            return Ok(if kind == 0x1 { Message::Text(String::from_utf8(data).map_err(|_| io::Error::other("text frame is not UTF-8"))?) } else { Message::Binary });
        }
    }
}

/// A frame as the server sends it (no mask).
pub fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        out.push(len as u8);
    } else if len <= 0xFFFF {
        out.push(126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

pub fn write_text(stream: &mut TcpStream, text: &str) -> io::Result<()> {
    stream.write_all(&frame(0x1, text.as_bytes()))
}

pub fn write_pong(stream: &mut TcpStream, payload: &[u8]) -> io::Result<()> {
    stream.write_all(&frame(0xA, payload))
}

pub fn write_close(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(&frame(0x8, &[]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A client-side masked frame, as a browser or the CLI would send.
    fn masked(opcode: u8, payload: &[u8], fin: bool) -> Vec<u8> {
        let mut out = vec![if fin { 0x80 } else { 0 } | opcode];
        let len = payload.len();
        if len < 126 {
            out.push(0x80 | len as u8);
        } else {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        let mask = [1u8, 2, 3, 4];
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    #[test]
    fn handshake_and_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            c.write_all(format!("GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: mcp, other\r\n{AUTH_HEADER}: secret\r\n\r\n").as_bytes()).unwrap();
            let mut resp = [0u8; 200];
            let n = c.read(&mut resp).unwrap();
            let resp = String::from_utf8_lossy(&resp[..n]).into_owned();
            // A message in two fragments, a ping, then a big one.
            c.write_all(&masked(0x1, b"hel", false)).unwrap();
            c.write_all(&masked(0x0, b"lo", true)).unwrap();
            c.write_all(&masked(0x9, b"p", true)).unwrap();
            let big = vec![b'x'; 70_000];
            let mut f = vec![0x81, 0x80 | 127];
            f.extend_from_slice(&(big.len() as u64).to_be_bytes());
            f.extend_from_slice(&[0, 0, 0, 0]);
            f.extend_from_slice(&big);
            c.write_all(&f).unwrap();
            // Read the server's text frame back.
            let mut head = [0u8; 2];
            c.read_exact(&mut head).unwrap();
            let mut body = vec![0u8; usize::from(head[1])];
            c.read_exact(&mut body).unwrap();
            (resp, String::from_utf8(body).unwrap())
        });
        let (mut s, _) = listener.accept().unwrap();
        handshake(&mut s, "secret").unwrap();
        assert!(matches!(read_message(&mut s).unwrap(), Message::Text(t) if t == "hello"));
        assert!(matches!(read_message(&mut s).unwrap(), Message::Ping(p) if p == b"p"));
        assert!(matches!(read_message(&mut s).unwrap(), Message::Text(t) if t.len() == 70_000));
        write_text(&mut s, "ok").unwrap();
        let (resp, echoed) = client.join().unwrap();
        assert!(resp.starts_with("HTTP/1.1 101"));
        assert!(resp.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
        assert!(resp.contains("Sec-WebSocket-Protocol: mcp\r\n"), "the subprotocol is answered: {resp}");
        assert_eq!(echoed, "ok");
        // A wrong token is refused.
        let listener2 = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr2 = listener2.local_addr().unwrap();
        let bad = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr2).unwrap();
            c.write_all(format!("GET / HTTP/1.1\r\nUpgrade: websocket\r\nSec-WebSocket-Key: a\r\n{AUTH_HEADER}: nope\r\n\r\n").as_bytes()).unwrap();
            let mut resp = [0u8; 64];
            let n = c.read(&mut resp).unwrap();
            String::from_utf8_lossy(&resp[..n]).into_owned()
        });
        let (mut s2, _) = listener2.accept().unwrap();
        assert!(handshake(&mut s2, "secret").is_err());
        assert!(bad.join().unwrap().starts_with("HTTP/1.1 401"));
    }
}
