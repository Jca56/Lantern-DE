//! `LIST INFO`: RIFF's own tags, a run of four-letter ids with a text each.
//! Entries are kept as the bytes they were stored as, so one that was not
//! edited goes back unchanged whatever its encoding or padding (it used to
//! be decoded, trimmed and re-encoded as UTF-8 on every save).

pub struct Info {
    entries: Vec<([u8; 4], Vec<u8>)>,
    /// False when a list did not parse to its end. What follows the point
    /// where it stopped is not in `entries`, so the list cannot be rebuilt.
    pub clean: bool,
}

/// ffmpeg / Audacity write UTF-8; older Windows tools write Latin-1.
fn decode(raw: &[u8]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    let b = &raw[..end];
    match std::str::from_utf8(b) {
        Ok(s) => s.trim().to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect::<String>().trim().to_string(),
    }
}

impl Info {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            clean: true,
        }
    }

    /// Add the entries of one `LIST` chunk body (starting with "INFO").
    pub fn absorb(&mut self, b: &[u8]) {
        if b.len() < 4 || &b[..4] != b"INFO" {
            self.clean = false;
            return;
        }
        let mut p = 4;
        while p + 8 <= b.len() {
            let id = [b[p], b[p + 1], b[p + 2], b[p + 3]];
            let size = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
            p += 8;
            if size > b.len() - p {
                self.clean = false;
                return;
            }
            self.entries.push((id, b[p..p + size].to_vec()));
            p += size + (size & 1);
        }
        // `p` is one past the end when the last entry is odd and unpadded.
        if p < b.len() && b[p..].iter().any(|&x| x != 0) {
            self.clean = false;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn has(&self, id: &[u8; 4]) -> bool {
        self.entries.iter().any(|(k, _)| k == id)
    }

    pub fn get(&self, id: &[u8; 4]) -> String {
        self.entries
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, v)| decode(v))
            .unwrap_or_default()
    }

    /// Replace every entry `id` with one holding `v` (none when empty), in
    /// the place of the first old one.
    pub fn set(&mut self, id: &[u8; 4], v: &str) {
        let v = v.trim();
        let at = self.entries.iter().position(|(k, _)| k == id);
        self.entries.retain(|(k, _)| k != id);
        if !v.is_empty() {
            let mut bytes = v.as_bytes().to_vec();
            bytes.push(0);
            match at {
                Some(i) => self.entries.insert(i, (*id, bytes)),
                None => self.entries.push((*id, bytes)),
            }
        }
    }

    /// The body of a `LIST` chunk.
    pub fn build(&self) -> Vec<u8> {
        let mut body = b"INFO".to_vec();
        for (id, v) in &self.entries {
            body.extend_from_slice(id);
            body.extend_from_slice(&(v.len() as u32).to_le_bytes());
            body.extend_from_slice(v);
            if v.len() & 1 == 1 {
                body.push(0);
            }
        }
        body
    }
}
