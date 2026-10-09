//! The Wayland wire format, as much of it as output management needs. A
//! message is two 32-bit words (the object it is for, then its length in
//! bytes over its opcode) and its arguments, each a whole number of
//! words in the machine's byte order: an integer, a 24.8 fixed-point
//! number, an object's id, or a string (its length with the NUL, the
//! bytes, the NUL, padding). No message here carries a file descriptor,
//! so a plain socket is all it takes.

/// The header: the object and the length-and-opcode word.
const HEADER: usize = 8;

/// Splits what arrives on the socket into messages, however the bytes
/// were cut up on the way.
#[derive(Default)]
pub struct Reader {
    buf: Vec<u8>,
}

/// One message from the compositor.
#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub object: u32,
    pub opcode: u16,
    pub body: Vec<u8>,
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

impl Reader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, when all of it is in. `Err` when the
    /// stream can't be a Wayland one any more (a length shorter than its
    /// own header): nothing after it can be trusted.
    pub fn next(&mut self) -> Result<Option<Message>, String> {
        let (Some(object), Some(second)) = (word(&self.buf, 0), word(&self.buf, 4)) else { return Ok(None) };
        let size = (second >> 16) as usize;
        if size < HEADER || !size.is_multiple_of(4) {
            return Err(format!("a message {size} bytes long for object {object}"));
        }
        if self.buf.len() < size {
            return Ok(None);
        }
        let body = self.buf[HEADER..size].to_vec();
        self.buf.drain(..size);
        Ok(Some(Message { object, opcode: (second & 0xFFFF) as u16, body }))
    }
}

/// A message's arguments, read in order. Each read is `None` past the
/// end, so a message cut short reads as nothing rather than as rubbish.
pub struct Args<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Args<'a> {
    pub fn new(body: &'a [u8]) -> Self {
        Self { body, at: 0 }
    }

    pub fn uint(&mut self) -> Option<u32> {
        let v = word(self.body, self.at)?;
        self.at += 4;
        Some(v)
    }

    pub fn int(&mut self) -> Option<i32> {
        self.uint().map(|v| v as i32)
    }

    /// A 24.8 fixed-point number.
    pub fn fixed(&mut self) -> Option<f64> {
        self.int().map(|v| f64::from(v) / 256.0)
    }

    pub fn string(&mut self) -> Option<String> {
        let len = self.uint()? as usize;
        if len == 0 {
            return Some(String::new());
        }
        let bytes = self.body.get(self.at..self.at + len)?;
        self.at += len.next_multiple_of(4);
        // The last byte is the NUL.
        Some(String::from_utf8_lossy(&bytes[..len - 1]).into_owned())
    }
}

/// A request being written. Its length goes into the header when it is
/// [`Request::finish`]ed.
pub struct Request {
    bytes: Vec<u8>,
}

impl Request {
    pub fn new(object: u32, opcode: u16) -> Self {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&object.to_ne_bytes());
        bytes.extend_from_slice(&u32::from(opcode).to_ne_bytes());
        Self { bytes }
    }

    pub fn uint(mut self, v: u32) -> Self {
        self.bytes.extend_from_slice(&v.to_ne_bytes());
        self
    }

    pub fn int(self, v: i32) -> Self {
        self.uint(v as u32)
    }

    /// A 24.8 fixed-point number, to the nearest 256th.
    pub fn fixed(self, v: f64) -> Self {
        self.int((v * 256.0).round() as i32)
    }

    pub fn string(mut self, s: &str) -> Self {
        let len = s.len() + 1;
        self = self.uint(len as u32);
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.resize(self.bytes.len() + len.next_multiple_of(4) - s.len(), 0);
        self
    }

    /// The finished message, appended to `out`.
    pub fn finish(mut self, out: &mut Vec<u8>) {
        let second = ((self.bytes.len() as u32) << 16) | word(&self.bytes, 4).unwrap_or(0);
        self.bytes[4..8].copy_from_slice(&second.to_ne_bytes());
        out.extend_from_slice(&self.bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(r: Request) -> Vec<u8> {
        let mut out = Vec::new();
        r.finish(&mut out);
        out
    }

    #[test]
    fn a_request_reads_back_as_it_was_written() {
        let bytes = built(Request::new(7, 3).uint(42).int(-5).fixed(1.4).string("DP-1").string("").uint(9));
        assert_eq!(bytes.len() % 4, 0);
        let mut r = Reader::default();
        r.push(&bytes);
        let m = r.next().unwrap().unwrap();
        assert_eq!((m.object, m.opcode), (7, 3));
        let mut a = Args::new(&m.body);
        assert_eq!((a.uint(), a.int()), (Some(42), Some(-5)));
        // 1.4 is not a whole number of 256ths: it comes back as the nearest.
        assert!((a.fixed().unwrap() - 1.4).abs() < 1.0 / 256.0);
        assert_eq!(a.string().as_deref(), Some("DP-1"));
        // An empty string is still a NUL and its padding.
        assert_eq!((a.string().as_deref(), a.uint()), (Some(""), Some(9)));
        assert_eq!(a.uint(), None, "nothing past the end");
        assert_eq!(r.next(), Ok(None));
    }

    #[test]
    fn strings_are_padded_to_whole_words() {
        // Three letters and the NUL fill a word; four need a second.
        assert_eq!(built(Request::new(1, 0).string("abc")).len(), 8 + 4 + 4);
        assert_eq!(built(Request::new(1, 0).string("abcd")).len(), 8 + 4 + 8);
        // A null string is a length of zero and no bytes.
        let null = [0u8; 4];
        assert_eq!(Args::new(&null).string().as_deref(), Some(""));
    }

    #[test]
    fn messages_come_out_whole_however_the_bytes_were_cut() {
        let mut stream = built(Request::new(2, 1).uint(1).string("zwlr_output_manager_v1").uint(4));
        stream.extend(built(Request::new(3, 0)));
        let mut r = Reader::default();
        let mut got = Vec::new();
        for chunk in stream.chunks(5) {
            r.push(chunk);
            while let Some(m) = r.next().unwrap() {
                got.push((m.object, m.opcode, m.body.len()));
            }
        }
        assert_eq!(got, vec![(2, 1, 4 + 4 + 24 + 4), (3, 0, 0)]);
    }

    #[test]
    fn a_length_that_cant_be_is_the_end_of_the_stream() {
        let mut r = Reader::default();
        // Length 4: shorter than the header it is in.
        r.push(&[1, 0, 0, 0]);
        r.push(&(4u32 << 16).to_ne_bytes());
        assert!(r.next().is_err());
        // A message cut short reads as nothing, not as rubbish.
        let short = 9u32.to_ne_bytes();
        let mut a = Args::new(&short);
        assert_eq!(a.string(), None);
    }
}
