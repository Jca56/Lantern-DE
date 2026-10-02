//! What sits inside ID3v2 frames: the four text encodings, picture frames
//! (v2.3/v2.4 `APIC` and v2.2 `PIC`), and the year of a date value.

/// Decode a text frame body (encoding byte + text). v2.4 multi-values
/// (NUL-separated) are joined with " / ".
pub fn decode_text(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let s = decode_with(data[0], &data[1..]);
    let parts: Vec<&str> = s
        .split('\0')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    parts.join(" / ")
}

fn decode_with(enc: u8, b: &[u8]) -> String {
    let s = match enc {
        0 => b.iter().map(|&c| c as char).collect(),
        1 => decode_utf16(b, None),
        2 => decode_utf16(b, Some(true)),
        _ => String::from_utf8_lossy(b).into_owned(),
    };
    s.trim_end_matches('\0').to_string()
}

fn decode_utf16(b: &[u8], big_endian: Option<bool>) -> String {
    let (be, body) = match (big_endian, b) {
        (_, [0xFF, 0xFE, rest @ ..]) => (false, rest),
        (_, [0xFE, 0xFF, rest @ ..]) => (true, rest),
        (Some(be), rest) => (be, rest),
        (None, rest) => (false, rest),
    };
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| {
            if be {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                u16::from_le_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Split off one encoding-terminated string; returns (string, remainder).
fn take_terminated(enc: u8, b: &[u8]) -> (String, &[u8]) {
    if enc == 1 || enc == 2 {
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == 0 && b[i + 1] == 0 {
                return (decode_with(enc, &b[..i]), &b[i + 2..]);
            }
            i += 2;
        }
        (decode_with(enc, b), &[])
    } else {
        match b.iter().position(|&c| c == 0) {
            Some(i) => (decode_with(enc, &b[..i]), &b[i + 1..]),
            None => (decode_with(enc, b), &[]),
        }
    }
}

/// A text frame body every ID3v2 version can read: Latin-1 when it fits,
/// else UTF-16 with a byte-order mark. (UTF-8 exists only in v2.4.)
pub fn encode_text(s: &str) -> Vec<u8> {
    let latin = s.chars().all(|c| (c as u32) < 0x100);
    let mut out = vec![if latin { 0u8 } else { 1 }];
    if latin {
        out.extend(s.chars().map(|c| c as u8));
    } else {
        out.extend_from_slice(&[0xFF, 0xFE]);
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out
}

// ── Pictures ────────────────────────────────────────────────────────────────

fn sniffed(mime: String, image: &[u8]) -> String {
    if mime.contains('/') && mime.len() > "image/".len() {
        mime
    } else {
        crate::audio_tags::sniff_image_mime(image)
            .unwrap_or("image/jpeg")
            .to_string()
    }
}

/// `APIC` body → (mime, picture type, image bytes)
pub fn parse_apic(data: &[u8]) -> Option<(String, u8, Vec<u8>)> {
    if data.len() < 4 {
        return None;
    }
    let enc = data[0];
    let (mime, rest) = take_terminated(0, &data[1..]);
    let pic_type = *rest.first()?;
    let (_desc, rest) = take_terminated(enc, &rest[1..]);
    if rest.is_empty() {
        return None;
    }
    Some((sniffed(mime, rest), pic_type, rest.to_vec()))
}

pub fn build_apic(mime: &str, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8]; // Latin-1 for the (empty) description
    out.extend_from_slice(mime.as_bytes());
    out.push(0);
    out.push(3); // front cover
    out.push(0); // description terminator
    out.extend_from_slice(data);
    out
}

/// v2.2 `PIC` body → (mime, picture type, image bytes). Same as `APIC`
/// except that the format is a fixed three-letter code instead of a MIME
/// string.
pub fn parse_pic(data: &[u8]) -> Option<(String, u8, Vec<u8>)> {
    if data.len() < 6 {
        return None;
    }
    let enc = data[0];
    let mime = match data[1..4].to_ascii_uppercase().as_slice() {
        b"PNG" => "image/png",
        b"JPG" => "image/jpeg",
        b"GIF" => "image/gif",
        b"BMP" => "image/bmp",
        _ => "",
    };
    let pic_type = data[4];
    let (_desc, rest) = take_terminated(enc, &data[5..]);
    if rest.is_empty() {
        return None;
    }
    Some((sniffed(mime.to_string(), rest), pic_type, rest.to_vec()))
}

/// None when the picture's format has no v2.2 code.
pub fn build_pic(mime: &str, data: &[u8]) -> Option<Vec<u8>> {
    let code: &[u8; 3] = match mime.to_ascii_lowercase().as_str() {
        "image/png" => b"PNG",
        "image/jpeg" | "image/jpg" => b"JPG",
        "image/gif" => b"GIF",
        "image/bmp" => b"BMP",
        _ => return None,
    };
    let mut out = vec![0u8]; // Latin-1 for the (empty) description
    out.extend_from_slice(code);
    out.push(3); // front cover
    out.push(0); // description terminator
    out.extend_from_slice(data);
    Some(out)
}

// ── Dates ───────────────────────────────────────────────────────────────────

/// Put a new year into a stored date. The editor shows and edits a year, but
/// the value on disk may be a full timestamp ("2024-05-17"); the part after
/// the year is not the user's edit and stays. Anything that is not a plain
/// four-digit year replacing a four-digit year replaces the whole value.
pub fn splice_year(stored: &str, year: &str) -> String {
    let (stored, year) = (stored.trim(), year.trim());
    let four_digits = |s: &str| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit());
    if four_digits(year) && stored.len() > 4 && stored.is_char_boundary(4) {
        let (head, rest) = stored.split_at(4);
        if four_digits(head) && rest.starts_with('-') {
            return format!("{year}{rest}");
        }
    }
    year.to_string()
}
