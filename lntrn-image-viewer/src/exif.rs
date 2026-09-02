//! Minimal EXIF reader for the info overlay: camera, lens, date taken,
//! exposure, aperture, ISO, focal length, software. Input is the raw TIFF
//! blob `image` hands back from `exif_metadata()` (byte-order mark first);
//! anything malformed simply yields fewer fields. Our own on purpose — a
//! full EXIF crate is a lot of dependency for nine tags.

#[derive(Debug, Default, Clone, PartialEq)]
pub struct ExifInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub software: Option<String>,
    /// "2026-08-30 19:48"
    pub date_taken: Option<String>,
    /// "1/250 s"
    pub exposure: Option<String>,
    /// "f/2.8"
    pub aperture: Option<String>,
    pub iso: Option<u32>,
    /// "50 mm"
    pub focal: Option<String>,
}

impl ExifInfo {
    pub fn is_empty(&self) -> bool {
        self.make.is_none()
            && self.model.is_none()
            && self.lens.is_none()
            && self.software.is_none()
            && self.date_taken.is_none()
            && self.exposure.is_none()
            && self.aperture.is_none()
            && self.iso.is_none()
            && self.focal.is_none()
    }

    /// "Canon EOS R6" — most bodies already put the make in the model string,
    /// so only prepend it when they don't.
    pub fn camera(&self) -> Option<String> {
        match (&self.make, &self.model) {
            (Some(mk), Some(md)) => {
                if md.to_lowercase().starts_with(&mk.to_lowercase()) {
                    Some(md.clone())
                } else {
                    Some(format!("{mk} {md}"))
                }
            }
            (None, Some(md)) => Some(md.clone()),
            (Some(mk), None) => Some(mk.clone()),
            (None, None) => None,
        }
    }
}

const TAG_MAKE: u16 = 0x010f;
const TAG_MODEL: u16 = 0x0110;
const TAG_SOFTWARE: u16 = 0x0131;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_EXPOSURE_TIME: u16 = 0x829a;
const TAG_F_NUMBER: u16 = 0x829d;
const TAG_ISO: u16 = 0x8827;
const TAG_DATE_ORIGINAL: u16 = 0x9003;
const TAG_FOCAL_LENGTH: u16 = 0x920a;
const TAG_LENS_MODEL: u16 = 0xa434;

const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;

/// Sanity cap so a corrupt count can't make us walk the whole file.
const MAX_ENTRIES: usize = 256;

struct Entry {
    tag: u16,
    ty: u16,
    count: u32,
    /// Offset of the entry's 4-byte value/offset field.
    field: usize,
}

struct Tiff<'a> {
    data: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.data.get(at..at + 2)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.data.get(at..at + 4)?;
        Some(if self.le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    fn entries(&self, ifd: usize) -> Vec<Entry> {
        let Some(n) = self.u16(ifd) else {
            return Vec::new();
        };
        (0..(n as usize).min(MAX_ENTRIES))
            .filter_map(|i| {
                let e = ifd + 2 + i * 12;
                Some(Entry {
                    tag: self.u16(e)?,
                    ty: self.u16(e + 2)?,
                    count: self.u32(e + 4)?,
                    field: e + 8,
                })
            })
            .collect()
    }

    fn type_size(ty: u16) -> usize {
        match ty {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => 0,
        }
    }

    /// Where an entry's payload lives: inline in the field when it fits in
    /// 4 bytes, otherwise at the offset the field holds. Returns (start, len).
    fn payload(&self, e: &Entry) -> Option<(usize, usize)> {
        let size = Self::type_size(e.ty).checked_mul(e.count as usize)?;
        if size == 0 {
            return None;
        }
        let start = if size <= 4 {
            e.field
        } else {
            self.u32(e.field)? as usize
        };
        if start.checked_add(size)? > self.data.len() {
            return None;
        }
        Some((start, size))
    }

    fn ascii(&self, e: &Entry) -> Option<String> {
        if e.ty != TYPE_ASCII {
            return None;
        }
        let (start, len) = self.payload(e)?;
        let bytes = &self.data[start..start + len];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(len);
        let s = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    /// SHORT or LONG scalar (values are left-justified in the field).
    fn uint(&self, e: &Entry) -> Option<u32> {
        match e.ty {
            TYPE_SHORT => self.u16(e.field).map(u32::from),
            TYPE_LONG => self.u32(e.field),
            _ => None,
        }
    }

    fn rational(&self, e: &Entry) -> Option<(u32, u32)> {
        if e.ty != TYPE_RATIONAL {
            return None;
        }
        let (start, _) = self.payload(e)?;
        Some((self.u32(start)?, self.u32(start + 4)?))
    }
}

/// Parse the tags we display out of a raw EXIF/TIFF blob. `None` only when
/// the header itself is unrecognisable.
pub fn parse(blob: &[u8]) -> Option<ExifInfo> {
    // JPEG APP1 payloads carry an "Exif\0\0" prefix; `image` usually strips
    // it, but be tolerant either way.
    let data = blob.strip_prefix(b"Exif\0\0").unwrap_or(blob);
    let le = match data.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let t = Tiff { data, le };
    if t.u16(2)? != 42 {
        return None;
    }
    let ifd0 = t.u32(4)? as usize;

    let mut info = ExifInfo::default();
    let mut exif_ifd = None;
    for e in t.entries(ifd0) {
        match e.tag {
            TAG_MAKE => info.make = t.ascii(&e),
            TAG_MODEL => info.model = t.ascii(&e),
            TAG_SOFTWARE => info.software = t.ascii(&e),
            TAG_EXIF_IFD => exif_ifd = t.uint(&e).map(|v| v as usize),
            _ => {}
        }
    }
    if let Some(off) = exif_ifd {
        for e in t.entries(off) {
            match e.tag {
                TAG_DATE_ORIGINAL => info.date_taken = t.ascii(&e).map(format_exif_date),
                TAG_EXPOSURE_TIME => info.exposure = t.rational(&e).map(format_exposure),
                TAG_F_NUMBER => info.aperture = t.rational(&e).and_then(format_aperture),
                TAG_ISO => info.iso = t.uint(&e),
                TAG_FOCAL_LENGTH => info.focal = t.rational(&e).and_then(format_focal),
                TAG_LENS_MODEL => info.lens = t.ascii(&e),
                _ => {}
            }
        }
    }
    Some(info)
}

/// "2026:08:30 19:48:34" → "2026-08-30 19:48". Anything else passes through.
fn format_exif_date(raw: String) -> String {
    let b = raw.as_bytes();
    if b.len() >= 16 && b[4] == b':' && b[7] == b':' && b[10] == b' ' {
        format!(
            "{}-{}-{} {}",
            &raw[0..4],
            &raw[5..7],
            &raw[8..10],
            &raw[11..16]
        )
    } else {
        raw
    }
}

fn format_exposure((n, d): (u32, u32)) -> String {
    if d == 0 || n == 0 {
        return format!("{n} s");
    }
    if n < d {
        format!("1/{} s", (d as f64 / n as f64).round() as u64)
    } else {
        let v = n as f64 / d as f64;
        if v.fract() == 0.0 {
            format!("{v:.0} s")
        } else {
            format!("{v:.1} s")
        }
    }
}

fn format_aperture((n, d): (u32, u32)) -> Option<String> {
    if d == 0 {
        return None;
    }
    let f = n as f64 / d as f64;
    Some(if (f - f.round()).abs() < 0.05 {
        format!("f/{f:.0}")
    } else {
        format!("f/{f:.1}")
    })
}

fn format_focal((n, d): (u32, u32)) -> Option<String> {
    if d == 0 {
        return None;
    }
    let mm = n as f64 / d as f64;
    Some(if (mm - mm.round()).abs() < 0.05 {
        format!("{mm:.0} mm")
    } else {
        format!("{mm:.1} mm")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a little-endian TIFF with IFD0 (Make, Model, ExifIFD) and an
    /// Exif IFD (ExposureTime, FNumber, ISO, DateTimeOriginal, FocalLength).
    fn blob() -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(b"II");
        b.extend_from_slice(&42u16.to_le_bytes());
        b.extend_from_slice(&8u32.to_le_bytes()); // IFD0 at 8

        // Layout: IFD0 @8 (3 entries = 2+36+4 = 42 bytes → ends at 50),
        // strings after, Exif IFD after those.
        let make = b"Canon\0";
        let model = b"Canon EOS R6\0";
        let make_off = 50u32;
        let model_off = make_off + make.len() as u32;
        let exif_off = model_off + model.len() as u32;

        let mut entry = |b: &mut Vec<u8>, tag: u16, ty: u16, count: u32, field: [u8; 4]| {
            b.extend_from_slice(&tag.to_le_bytes());
            b.extend_from_slice(&ty.to_le_bytes());
            b.extend_from_slice(&count.to_le_bytes());
            b.extend_from_slice(&field);
        };

        b.extend_from_slice(&3u16.to_le_bytes());
        entry(
            &mut b,
            TAG_MAKE,
            TYPE_ASCII,
            make.len() as u32,
            make_off.to_le_bytes(),
        );
        entry(
            &mut b,
            TAG_MODEL,
            TYPE_ASCII,
            model.len() as u32,
            model_off.to_le_bytes(),
        );
        entry(&mut b, TAG_EXIF_IFD, TYPE_LONG, 1, exif_off.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        assert_eq!(b.len(), 50);
        b.extend_from_slice(make);
        b.extend_from_slice(model);
        assert_eq!(b.len() as u32, exif_off);

        // Exif IFD: 5 entries = 2 + 60 + 4 = 66 bytes; rationals + date after.
        let date = b"2026:08:30 19:48:34\0";
        let rat_base = exif_off + 66;
        let exp_off = rat_base;
        let fnum_off = rat_base + 8;
        let focal_off = rat_base + 16;
        let date_off = rat_base + 24;
        b.extend_from_slice(&5u16.to_le_bytes());
        entry(
            &mut b,
            TAG_EXPOSURE_TIME,
            TYPE_RATIONAL,
            1,
            exp_off.to_le_bytes(),
        );
        entry(
            &mut b,
            TAG_F_NUMBER,
            TYPE_RATIONAL,
            1,
            fnum_off.to_le_bytes(),
        );
        entry(&mut b, TAG_ISO, TYPE_SHORT, 1, [0x90, 0x01, 0, 0]); // 400 inline
        entry(
            &mut b,
            TAG_DATE_ORIGINAL,
            TYPE_ASCII,
            date.len() as u32,
            date_off.to_le_bytes(),
        );
        entry(
            &mut b,
            TAG_FOCAL_LENGTH,
            TYPE_RATIONAL,
            1,
            focal_off.to_le_bytes(),
        );
        b.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(b.len() as u32, rat_base);
        for (n, d) in [(1u32, 250u32), (28, 10), (50, 1)] {
            b.extend_from_slice(&n.to_le_bytes());
            b.extend_from_slice(&d.to_le_bytes());
        }
        b.extend_from_slice(date);
        b
    }

    #[test]
    fn parses_the_tags_we_show() {
        let info = parse(&blob()).expect("valid header");
        assert_eq!(info.camera().as_deref(), Some("Canon EOS R6"));
        assert_eq!(info.exposure.as_deref(), Some("1/250 s"));
        assert_eq!(info.aperture.as_deref(), Some("f/2.8"));
        assert_eq!(info.iso, Some(400));
        assert_eq!(info.date_taken.as_deref(), Some("2026-08-30 19:48"));
        assert_eq!(info.focal.as_deref(), Some("50 mm"));
        assert!(info.lens.is_none());
    }

    #[test]
    fn tolerates_app1_prefix_and_garbage() {
        let mut with_prefix = b"Exif\0\0".to_vec();
        with_prefix.extend_from_slice(&blob());
        assert!(parse(&with_prefix).unwrap().iso.is_some());
        assert!(parse(b"nope").is_none());
        assert!(parse(&[]).is_none());
        // Truncated blob: header ok, entries fall off the end → empty info.
        let short = &blob()[..12];
        assert!(parse(short).unwrap().is_empty());
    }

    #[test]
    fn camera_avoids_doubling_the_make() {
        let info = ExifInfo {
            make: Some("SONY".into()),
            model: Some("ILCE-7M4".into()),
            ..Default::default()
        };
        assert_eq!(info.camera().as_deref(), Some("SONY ILCE-7M4"));
    }
}
