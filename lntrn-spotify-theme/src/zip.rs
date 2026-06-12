//! Minimal ZIP container reader/writer.
//!
//! `xpui.spa` is a plain ZIP archive of DEFLATE-compressed entries. We parse
//! the central directory ourselves (it's just a byte layout — no rocket
//! science) and lean on `flate2` only for the raw DEFLATE codec + CRC32,
//! which would be genuinely painful to reimplement.
//!
//! The rewrite path copies every untouched entry's compressed bytes verbatim
//! (no recompression drift) and only re-deflates the one entry we mutate.
//! No ZIP64 — `xpui.spa` is ~40 MB across ~500 files, well under every limit.

use std::io::{Read, Write};

const EOCD_SIG: u32 = 0x0605_4b50; // End Of Central Directory
const CDH_SIG: u32 = 0x0201_4b50; // Central Directory Header
const LFH_SIG: u32 = 0x0403_4b50; // Local File Header
const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

pub type ZResult<T> = Result<T, String>;

/// One archive member. Holds the entry's metadata plus its *compressed* bytes
/// exactly as stored, so untouched entries round-trip byte-for-byte.
pub struct Entry {
    pub name: String,
    pub version_made_by: u16,
    pub version_needed: u16,
    pub gp_flag: u16,
    pub method: u16,
    pub mod_time: u16,
    pub mod_date: u16,
    pub crc: u32,
    pub comp_size: u32,
    pub uncomp_size: u32,
    pub internal_attrs: u16,
    pub external_attrs: u32,
    pub local_extra: Vec<u8>,
    pub central_extra: Vec<u8>,
    pub comment: Vec<u8>,
    /// Raw compressed bytes (DEFLATE stream or stored bytes).
    pub data: Vec<u8>,
}

impl Entry {
    /// Decompress this entry's payload into raw bytes.
    pub fn decompressed(&self) -> ZResult<Vec<u8>> {
        match self.method {
            METHOD_STORE => Ok(self.data.clone()),
            METHOD_DEFLATE => {
                let mut out = Vec::with_capacity(self.uncomp_size as usize);
                flate2::read::DeflateDecoder::new(&self.data[..])
                    .read_to_end(&mut out)
                    .map_err(|e| format!("inflate {}: {e}", self.name))?;
                Ok(out)
            }
            m => Err(format!("{}: unsupported compression method {m}", self.name)),
        }
    }

    /// Replace this entry's payload, re-deflating and refreshing crc/sizes.
    pub fn set_contents(&mut self, raw: &[u8]) -> ZResult<()> {
        let mut enc =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(raw).map_err(|e| format!("deflate write: {e}"))?;
        let comp = enc.finish().map_err(|e| format!("deflate finish: {e}"))?;

        let mut crc = flate2::Crc::new();
        crc.update(raw);

        self.method = METHOD_DEFLATE;
        self.crc = crc.sum();
        self.uncomp_size = raw.len() as u32;
        self.comp_size = comp.len() as u32;
        self.gp_flag &= !0x0008; // clear streaming data-descriptor bit
        self.data = comp;
        Ok(())
    }
}

pub struct Archive {
    pub entries: Vec<Entry>,
}

impl Archive {
    pub fn find_mut(&mut self, name: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.name == name)
    }

    /// Parse a full archive from bytes.
    pub fn read(buf: &[u8]) -> ZResult<Archive> {
        let eocd = find_eocd(buf).ok_or("EOCD record not found (not a zip?)")?;
        let count = u16(buf, eocd + 10)? as usize;
        let cd_offset = u32(buf, eocd + 16)? as usize;

        let mut entries = Vec::with_capacity(count);
        let mut p = cd_offset;
        for _ in 0..count {
            if u32(buf, p)? != CDH_SIG {
                return Err("bad central directory header signature".into());
            }
            let version_made_by = u16(buf, p + 4)?;
            let version_needed = u16(buf, p + 6)?;
            let gp_flag = u16(buf, p + 8)?;
            let method = u16(buf, p + 10)?;
            let mod_time = u16(buf, p + 12)?;
            let mod_date = u16(buf, p + 14)?;
            let crc = u32(buf, p + 16)?;
            let comp_size = u32(buf, p + 20)?;
            let uncomp_size = u32(buf, p + 24)?;
            let name_len = u16(buf, p + 28)? as usize;
            let extra_len = u16(buf, p + 30)? as usize;
            let comment_len = u16(buf, p + 32)? as usize;
            let internal_attrs = u16(buf, p + 36)?;
            let external_attrs = u32(buf, p + 38)?;
            let local_offset = u32(buf, p + 42)? as usize;

            let name_start = p + 46;
            let name = String::from_utf8_lossy(slice(buf, name_start, name_len)?).into_owned();
            let central_extra = slice(buf, name_start + name_len, extra_len)?.to_vec();
            let comment =
                slice(buf, name_start + name_len + extra_len, comment_len)?.to_vec();

            let (local_extra, data) =
                read_local(buf, local_offset, comp_size as usize)?;

            entries.push(Entry {
                name,
                version_made_by,
                version_needed,
                gp_flag,
                method,
                mod_time,
                mod_date,
                crc,
                comp_size,
                uncomp_size,
                internal_attrs,
                external_attrs,
                local_extra,
                central_extra,
                comment,
                data,
            });
            p = name_start + name_len + extra_len + comment_len;
        }
        Ok(Archive { entries })
    }

    /// Serialize back to a valid ZIP byte stream.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut offsets = Vec::with_capacity(self.entries.len());

        // Local file headers + data.
        for e in &self.entries {
            offsets.push(out.len() as u32);
            push_u32(&mut out, LFH_SIG);
            push_u16(&mut out, e.version_needed);
            push_u16(&mut out, e.gp_flag);
            push_u16(&mut out, e.method);
            push_u16(&mut out, e.mod_time);
            push_u16(&mut out, e.mod_date);
            push_u32(&mut out, e.crc);
            push_u32(&mut out, e.comp_size);
            push_u32(&mut out, e.uncomp_size);
            push_u16(&mut out, e.name.len() as u16);
            push_u16(&mut out, e.local_extra.len() as u16);
            out.extend_from_slice(e.name.as_bytes());
            out.extend_from_slice(&e.local_extra);
            out.extend_from_slice(&e.data);
        }

        // Central directory.
        let cd_start = out.len() as u32;
        for (e, &off) in self.entries.iter().zip(offsets.iter()) {
            push_u32(&mut out, CDH_SIG);
            push_u16(&mut out, e.version_made_by);
            push_u16(&mut out, e.version_needed);
            push_u16(&mut out, e.gp_flag);
            push_u16(&mut out, e.method);
            push_u16(&mut out, e.mod_time);
            push_u16(&mut out, e.mod_date);
            push_u32(&mut out, e.crc);
            push_u32(&mut out, e.comp_size);
            push_u32(&mut out, e.uncomp_size);
            push_u16(&mut out, e.name.len() as u16);
            push_u16(&mut out, e.central_extra.len() as u16);
            push_u16(&mut out, e.comment.len() as u16);
            push_u16(&mut out, 0); // disk number start
            push_u16(&mut out, e.internal_attrs);
            push_u32(&mut out, e.external_attrs);
            push_u32(&mut out, off);
            out.extend_from_slice(e.name.as_bytes());
            out.extend_from_slice(&e.central_extra);
            out.extend_from_slice(&e.comment);
        }
        let cd_size = out.len() as u32 - cd_start;

        // End of central directory.
        let n = self.entries.len() as u16;
        push_u32(&mut out, EOCD_SIG);
        push_u16(&mut out, 0); // this disk
        push_u16(&mut out, 0); // cd start disk
        push_u16(&mut out, n);
        push_u16(&mut out, n);
        push_u32(&mut out, cd_size);
        push_u32(&mut out, cd_start);
        push_u16(&mut out, 0); // comment length
        out
    }
}

/// Read a local header at `offset` and return (local_extra, compressed_data).
fn read_local(buf: &[u8], offset: usize, comp_size: usize) -> ZResult<(Vec<u8>, Vec<u8>)> {
    if u32(buf, offset)? != LFH_SIG {
        return Err("bad local file header signature".into());
    }
    let name_len = u16(buf, offset + 26)? as usize;
    let extra_len = u16(buf, offset + 28)? as usize;
    let extra_start = offset + 30 + name_len;
    let local_extra = slice(buf, extra_start, extra_len)?.to_vec();
    let data = slice(buf, extra_start + extra_len, comp_size)?.to_vec();
    Ok((local_extra, data))
}

/// Scan backwards for the EOCD signature (comment may trail it, up to 64 KiB).
fn find_eocd(buf: &[u8]) -> Option<usize> {
    if buf.len() < 22 {
        return None;
    }
    let min = buf.len().saturating_sub(22 + 0xFFFF);
    for i in (min..=buf.len() - 22).rev() {
        if u32(buf, i).ok() == Some(EOCD_SIG) {
            return Some(i);
        }
    }
    None
}

// ── Byte helpers (little-endian) ─────────────────────────────────────────────

fn u16(b: &[u8], o: usize) -> ZResult<u16> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| "unexpected EOF reading u16".into())
}

fn u32(b: &[u8], o: usize) -> ZResult<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "unexpected EOF reading u32".into())
}

fn slice(b: &[u8], o: usize, len: usize) -> ZResult<&[u8]> {
    b.get(o..o + len).ok_or_else(|| "unexpected EOF reading slice".into())
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
