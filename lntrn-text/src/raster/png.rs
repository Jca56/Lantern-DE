//! Minimal PNG decoder for CBDT emoji bitmaps — pure std, including the
//! DEFLATE (RFC 1951) inflater.
//!
//! Scope: what Google's emoji strikes actually use — 8-bit depth, color
//! types gray/RGB/palette/gray-alpha/RGBA, non-interlaced, filters 0–4.
//! Interlaced or 16-bit images are rejected (never emitted by emoji tools).

pub(crate) struct Image {
    pub width: u32,
    pub height: u32,
    /// Straight (non-premultiplied) RGBA8.
    pub rgba: Vec<u8>,
}

pub(crate) fn decode(data: &[u8]) -> Option<Image> {
    const SIG: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 8 || data[..8] != SIG {
        return None;
    }
    let mut pos = 8usize;
    let (mut width, mut height, mut color_type) = (0u32, 0u32, 0u8);
    let mut depth;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();

    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data.get(pos..pos + 4)?.try_into().ok()?) as usize;
        let ctype = data.get(pos + 4..pos + 8)?;
        let body = data.get(pos + 8..pos + 8 + len)?;
        match ctype {
            b"IHDR" => {
                width = u32::from_be_bytes(body.get(0..4)?.try_into().ok()?);
                height = u32::from_be_bytes(body.get(4..8)?.try_into().ok()?);
                depth = *body.get(8)?;
                color_type = *body.get(9)?;
                if depth != 8 || *body.get(12)? != 0 {
                    return None; // 16-bit or interlaced: out of scope
                }
            }
            b"PLTE" => {
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len; // len + type + data + crc
    }
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }

    let channels = match color_type {
        0 => 1, // gray
        2 => 3, // rgb
        3 => 1, // palette
        4 => 2, // gray + alpha
        6 => 4, // rgba
        _ => return None,
    };
    let stride = width as usize * channels;
    let raw = inflate(&idat, (stride + 1) * height as usize)?;
    if raw.len() < (stride + 1) * height as usize {
        return None;
    }

    // Undo per-scanline filters in place.
    let mut lines = vec![0u8; stride * height as usize];
    for y in 0..height as usize {
        let src = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let filter = src[0];
        for x in 0..stride {
            let above = if y > 0 {
                lines[(y - 1) * stride + x]
            } else {
                0
            };
            let left = if x >= channels {
                lines[y * stride + x - channels]
            } else {
                0
            };
            let upleft = if y > 0 && x >= channels {
                lines[(y - 1) * stride + x - channels]
            } else {
                0
            };
            let v = src[1 + x];
            lines[y * stride + x] = match filter {
                0 => v,
                1 => v.wrapping_add(left),
                2 => v.wrapping_add(above),
                3 => v.wrapping_add(((left as u16 + above as u16) / 2) as u8),
                4 => v.wrapping_add(paeth(left, above, upleft)),
                _ => return None,
            };
        }
    }

    // Expand to RGBA8.
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for px in lines.chunks_exact(channels) {
        match color_type {
            0 => rgba.extend_from_slice(&[px[0], px[0], px[0], 255]),
            2 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
            3 => {
                let [r, g, b] = palette.get(px[0] as usize).copied().unwrap_or([0, 0, 0]);
                let a = trns.get(px[0] as usize).copied().unwrap_or(255);
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            4 => rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            _ => rgba.extend_from_slice(&[px[0], px[1], px[2], px[3]]),
        }
    }
    Some(Image {
        width,
        height,
        rgba,
    })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i16, b as i16, c as i16);
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

// ── DEFLATE (RFC 1951) via zlib wrapper (RFC 1950) ──────────────────────────

struct BitReader<'a> {
    d: &'a [u8],
    pos: usize,
    bit: u32,
    acc: u32,
}

impl<'a> BitReader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self {
            d,
            pos: 0,
            bit: 0,
            acc: 0,
        }
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        while self.bit < n {
            let byte = *self.d.get(self.pos)? as u32;
            self.pos += 1;
            self.acc |= byte << self.bit;
            self.bit += 8;
        }
        let v = self.acc & ((1u32 << n) - 1);
        self.acc >>= n;
        self.bit -= n;
        Some(v)
    }

    fn align_byte(&mut self) {
        self.acc = 0;
        self.bit = 0;
    }
}

/// Canonical Huffman decoder: per-length first-code/first-symbol tables.
struct Huffman {
    /// (first_code, first_symbol_index, count) per bit length 1..=15.
    lengths: [(u32, u32, u32); 15],
    symbols: Vec<u16>,
}

impl Huffman {
    fn build(code_lengths: &[u8]) -> Option<Huffman> {
        let mut count = [0u32; 16];
        for &l in code_lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        let mut symbols = Vec::with_capacity(code_lengths.len());
        let mut lengths = [(0u32, 0u32, 0u32); 15];
        let mut code = 0u32;
        let mut index = 0u32;
        for len in 1..=15usize {
            code <<= 1;
            lengths[len - 1] = (code, index, count[len]);
            code += count[len];
            index += count[len];
        }
        for len in 1..=15u8 {
            for (sym, &l) in code_lengths.iter().enumerate() {
                if l == len {
                    symbols.push(sym as u16);
                }
            }
        }
        Some(Huffman { lengths, symbols })
    }

    fn decode(&self, br: &mut BitReader) -> Option<u16> {
        let mut code = 0u32;
        for len in 1..=15usize {
            code = (code << 1) | br.bits(1)?;
            let (first, index, count) = self.lengths[len - 1];
            if count > 0 && code >= first && code < first + count {
                return self.symbols.get((index + code - first) as usize).copied();
            }
        }
        None
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn inflate(zlib: &[u8], size_hint: usize) -> Option<Vec<u8>> {
    if zlib.len() < 2 || zlib[0] & 0x0F != 8 || zlib[1] & 0x20 != 0 {
        return None; // not deflate / preset dictionary unsupported
    }
    let mut br = BitReader::new(&zlib[2..]);
    let mut out: Vec<u8> = Vec::with_capacity(size_hint);
    loop {
        let last = br.bits(1)?;
        match br.bits(2)? {
            0 => {
                // Stored block.
                br.align_byte();
                let len = br.bits(16)? as usize;
                let _nlen = br.bits(16)?;
                for _ in 0..len {
                    out.push(br.bits(8)? as u8);
                }
            }
            1 => {
                // Fixed Huffman tables.
                let mut litlen = [0u8; 288];
                for (i, l) in litlen.iter_mut().enumerate() {
                    *l = match i {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let lit = Huffman::build(&litlen)?;
                let dist = Huffman::build(&[5u8; 30])?;
                inflate_block(&mut br, &lit, &dist, &mut out)?;
            }
            2 => {
                // Dynamic Huffman tables.
                let hlit = br.bits(5)? as usize + 257;
                let hdist = br.bits(5)? as usize + 1;
                let hclen = br.bits(4)? as usize + 4;
                let mut cl_lengths = [0u8; 19];
                for &slot in CL_ORDER.iter().take(hclen) {
                    cl_lengths[slot] = br.bits(3)? as u8;
                }
                let cl = Huffman::build(&cl_lengths)?;
                let mut lengths = vec![0u8; hlit + hdist];
                let mut i = 0;
                while i < lengths.len() {
                    match cl.decode(&mut br)? {
                        sym @ 0..=15 => {
                            lengths[i] = sym as u8;
                            i += 1;
                        }
                        16 => {
                            let prev = *lengths.get(i.checked_sub(1)?)?;
                            let n = br.bits(2)? as usize + 3;
                            for _ in 0..n.min(lengths.len() - i) {
                                lengths[i] = prev;
                                i += 1;
                            }
                        }
                        17 => i += br.bits(3)? as usize + 3,
                        18 => i += br.bits(7)? as usize + 11,
                        _ => return None,
                    }
                }
                let lit = Huffman::build(&lengths[..hlit])?;
                let dist = Huffman::build(&lengths[hlit..])?;
                inflate_block(&mut br, &lit, &dist, &mut out)?;
            }
            _ => return None,
        }
        if last == 1 {
            return Some(out);
        }
    }
}

fn inflate_block(
    br: &mut BitReader,
    lit: &Huffman,
    dist: &Huffman,
    out: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let sym = lit.decode(br)?;
        match sym {
            0..=255 => out.push(sym as u8),
            256 => return Some(()),
            257..=285 => {
                let li = sym as usize - 257;
                let length = LENGTH_BASE[li] as usize + br.bits(LENGTH_EXTRA[li] as u32)? as usize;
                let ds = dist.decode(br)? as usize;
                if ds >= 30 {
                    return None;
                }
                let distance = DIST_BASE[ds] as usize + br.bits(DIST_EXTRA[ds] as u32)? as usize;
                if distance == 0 || distance > out.len() {
                    return None;
                }
                let start = out.len() - distance;
                for k in 0..length {
                    let b = out[start + k];
                    out.push(b);
                }
            }
            _ => return None,
        }
    }
}
