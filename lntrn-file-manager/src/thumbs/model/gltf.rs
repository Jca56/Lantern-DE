//! The glTF container: its JSON, the buffers that JSON points into, and
//! accessors read out of them as plain numbers.
//!
//! Only what the picture needs is read. A GLB's binary chunk and a .gltf's
//! buffer files are read a buffer view at a time, positioned reads straight
//! from the file, so a model whose pictures run to hundreds of megabytes
//! costs its geometry and its base colour pictures, nothing more.

use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::Value;

/// Most JSON read: a GLB's JSON chunk, or a whole .gltf.
const MAX_JSON_BYTES: u64 = 64 * 1024 * 1024;
/// Most bytes read out of buffers and pictures for one model. A model that
/// needs more keeps the generic icon rather than hold that much in memory
/// on each of the thumbnail workers.
const MAX_READ_BYTES: u64 = 256 * 1024 * 1024;
/// Most numbers one accessor may hold (32M floats are 128 MB).
const MAX_COMPONENTS: usize = 32 * 1024 * 1024;

const GLB_MAGIC: &[u8; 4] = b"glTF";
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

const BYTE: u32 = 5120;
const UNSIGNED_BYTE: u32 = 5121;
const SHORT: u32 = 5122;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const FLOAT: u32 = 5126;

pub(super) struct Gltf {
    pub json: Value,
    /// Where relative URIs start from: the folder the file is in.
    dir: PathBuf,
    /// A GLB's binary chunk: the file, and where in it the chunk lies.
    glb_bin: Option<(Rc<File>, u64, u64)>,
    buffers: HashMap<usize, Rc<Buffer>>,
    /// Buffer views read for geometry, kept while the scene is built (an
    /// interleaved view serves several accessors).
    views: HashMap<usize, Rc<Vec<u8>>>,
    /// Bytes read so far, against [`MAX_READ_BYTES`].
    read: u64,
}

enum Buffer {
    /// A data: URI, decoded.
    Bytes(Vec<u8>),
    /// A file, or the part of one from `base`, `len` bytes long.
    File { file: Rc<File>, base: u64, len: u64 },
}

/// Where an accessor's numbers are and how they are written.
struct Layout {
    view: Option<usize>,
    offset: usize,
    ctype: u32,
    normalized: bool,
    comps: usize,
    count: usize,
    sparse: Option<Sparse>,
}

/// Replacements for some of an accessor's elements.
struct Sparse {
    count: usize,
    index_view: usize,
    index_offset: usize,
    index_type: u32,
    value_view: usize,
    value_offset: usize,
}

/// Open a `.glb` or `.gltf` and parse its JSON. Its buffers are read later,
/// as accessors ask for them.
pub(super) fn open(path: &Path) -> Option<Gltf> {
    let file = open_regular(path)?;
    let len = file.metadata().ok()?.len();
    let mut head = [0u8; 12];
    let is_glb = len >= 12 && file.read_exact_at(&mut head, 0).is_ok() && &head[..4] == GLB_MAGIC;
    let (text, glb_bin) = if is_glb {
        read_glb(file, len, &head)?
    } else {
        if len > MAX_JSON_BYTES {
            return None;
        }
        let mut text = vec![0u8; len as usize];
        file.read_exact_at(&mut text, 0).ok()?;
        (text, None)
    };
    // Some writers put a byte order mark in front, which the spec forbids
    // but JSON parsers choke on.
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&text);
    let json: Value = serde_json::from_slice(text).ok()?;
    // glTF 1.0 is laid out differently, and other JSON isn't glTF at all.
    let version = json.pointer("/asset/version").and_then(Value::as_str)?;
    if !version.starts_with("2.") {
        return None;
    }
    Some(Gltf {
        json,
        dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        glb_bin,
        buffers: HashMap::new(),
        views: HashMap::new(),
        read: 0,
    })
}

/// A GLB's JSON chunk, and where its binary chunk lies.
#[allow(clippy::type_complexity)]
fn read_glb(file: File, file_len: u64, head: &[u8; 12]) -> Option<(Vec<u8>, Option<(Rc<File>, u64, u64)>)> {
    if u32_at(head, 4) != 2 {
        return None;
    }
    let total = u64::from(u32_at(head, 8)).min(file_len);
    let mut at = 12u64;
    let mut json = None;
    let mut bin = None;
    while at + 8 <= total {
        let mut chunk = [0u8; 8];
        file.read_exact_at(&mut chunk, at).ok()?;
        let (len, kind) = (u64::from(u32_at(&chunk, 0)), u32_at(&chunk, 4));
        let data = at + 8;
        if data + len > total {
            return None; // cut short
        }
        match kind {
            CHUNK_JSON if json.is_none() => {
                if len > MAX_JSON_BYTES {
                    return None;
                }
                let mut text = vec![0u8; len as usize];
                file.read_exact_at(&mut text, data).ok()?;
                json = Some(text);
            }
            CHUNK_BIN if json.is_some() && bin.is_none() => bin = Some((data, len)),
            _ => {}
        }
        // Chunks are padded to four bytes; be lenient with writers that
        // count the padding out.
        at = (data + len + 3) & !3;
    }
    let bin = bin.map(|(base, len)| (Rc::new(file), base, len));
    Some((json?, bin))
}

impl Gltf {
    /// Numbers out of accessor `index`, normalized integers turned into
    /// their fractions: how many there are to an element, and all of them.
    pub fn floats(&mut self, index: usize) -> Option<(usize, Vec<f32>)> {
        let lay = self.layout(index)?;
        let (ctype, normalized) = (lay.ctype, lay.normalized);
        let data = self.gather(&lay, |b| decode_f32(ctype, normalized, b))?;
        Some((lay.comps, data))
    }

    /// Whole numbers out of accessor `index` (indices, joints), as
    /// [`Gltf::floats`] gives them.
    pub fn uints(&mut self, index: usize) -> Option<(usize, Vec<u32>)> {
        let lay = self.layout(index)?;
        if lay.ctype == FLOAT {
            return None;
        }
        let ctype = lay.ctype;
        let data = self.gather(&lay, |b| decode_u32(ctype, b))?;
        Some((lay.comps, data))
    }

    /// The encoded bytes (PNG, JPEG, WebP) of image `index`.
    pub fn image(&mut self, index: usize) -> Option<Vec<u8>> {
        let image = self.json.get("images")?.get(index)?;
        let view = uint(image, "bufferView");
        let uri = image.get("uri").and_then(Value::as_str).map(str::to_owned);
        if let Some(view) = view {
            return self.read_view(view);
        }
        let uri = uri?;
        if let Some(payload) = data_uri(&uri) {
            return self.decode_data(payload);
        }
        let file = open_regular(&self.resolve(&uri)?)?;
        let len = file.metadata().ok()?.len();
        self.charge(len)?;
        let mut bytes = vec![0u8; len as usize];
        file.read_exact_at(&mut bytes, 0).ok()?;
        Some(bytes)
    }

    fn layout(&self, index: usize) -> Option<Layout> {
        let a = self.json.get("accessors")?.get(index)?;
        let ctype = uint(a, "componentType")? as u32;
        let size = comp_size(ctype)?;
        let comps = match a.get("type")?.as_str()? {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            // Small-component matrices pad their columns; nothing drawn
            // here needs one.
            "MAT4" if size == 4 => 16,
            _ => return None,
        };
        let count = uint(a, "count")?;
        if count.checked_mul(comps)? > MAX_COMPONENTS {
            return None;
        }
        let sparse = match a.get("sparse") {
            Some(s) => {
                let (i, v) = (s.get("indices")?, s.get("values")?);
                Some(Sparse {
                    count: uint(s, "count")?,
                    index_view: uint(i, "bufferView")?,
                    index_offset: uint(i, "byteOffset").unwrap_or(0),
                    index_type: uint(i, "componentType")? as u32,
                    value_view: uint(v, "bufferView")?,
                    value_offset: uint(v, "byteOffset").unwrap_or(0),
                })
            }
            None => None,
        };
        Some(Layout {
            view: uint(a, "bufferView"),
            offset: uint(a, "byteOffset").unwrap_or(0),
            ctype,
            normalized: a.get("normalized").and_then(Value::as_bool).unwrap_or(false),
            comps,
            count,
            sparse,
        })
    }

    /// An accessor's elements, each component through `conv`: from its
    /// buffer view (zeros without one), then its sparse replacements.
    fn gather<T: Copy + Default>(&mut self, lay: &Layout, conv: impl Fn(&[u8]) -> T) -> Option<Vec<T>> {
        let size = comp_size(lay.ctype)?;
        let elem = size * lay.comps;
        let mut out = vec![T::default(); lay.count * lay.comps];
        if let Some(view) = lay.view {
            let stride = self
                .json
                .get("bufferViews")?
                .get(view)?
                .get("byteStride")
                .and_then(Value::as_u64)
                .map_or(elem, |s| s as usize)
                .max(elem);
            let bytes = self.view(view)?;
            if lay.count > 0 {
                let end = lay
                    .offset
                    .checked_add(stride.checked_mul(lay.count - 1)?)?
                    .checked_add(elem)?;
                if end > bytes.len() {
                    return None;
                }
            }
            for (i, dst) in out.chunks_exact_mut(lay.comps).enumerate() {
                let at = lay.offset + i * stride;
                for (c, d) in dst.iter_mut().enumerate() {
                    *d = conv(&bytes[at + c * size..]);
                }
            }
        }
        if let Some(s) = &lay.sparse {
            let index_size = comp_size(s.index_type)?;
            let indices = self.view(s.index_view)?;
            let values = self.view(s.value_view)?;
            if s.index_offset.checked_add(s.count.checked_mul(index_size)?)? > indices.len()
                || s.value_offset.checked_add(s.count.checked_mul(elem)?)? > values.len()
            {
                return None;
            }
            for k in 0..s.count {
                let target = decode_u32(s.index_type, &indices[s.index_offset + k * index_size..]) as usize;
                let dst = out.get_mut(target * lay.comps..(target + 1) * lay.comps)?;
                let at = s.value_offset + k * elem;
                for (c, d) in dst.iter_mut().enumerate() {
                    *d = conv(&values[at + c * size..]);
                }
            }
        }
        Some(out)
    }

    /// Buffer view `index`, read once and kept.
    fn view(&mut self, index: usize) -> Option<Rc<Vec<u8>>> {
        if let Some(bytes) = self.views.get(&index) {
            return Some(Rc::clone(bytes));
        }
        let bytes = Rc::new(self.read_view(index)?);
        self.views.insert(index, Rc::clone(&bytes));
        Some(bytes)
    }

    /// Buffer view `index`, read now and not kept (a picture's bytes are
    /// decoded once and dropped).
    fn read_view(&mut self, index: usize) -> Option<Vec<u8>> {
        let v = self.json.get("bufferViews")?.get(index)?;
        let buffer = uint(v, "buffer")?;
        let offset = uint(v, "byteOffset").unwrap_or(0) as u64;
        let len = uint(v, "byteLength")? as u64;
        let end = offset.checked_add(len)?;
        self.charge(len)?;
        match &*self.buffer(buffer)? {
            Buffer::Bytes(b) => b.get(offset as usize..end as usize).map(<[u8]>::to_vec),
            Buffer::File { file, base, len: have } => {
                if end > *have {
                    return None;
                }
                let mut out = vec![0u8; len as usize];
                file.read_exact_at(&mut out, base + offset).ok()?;
                Some(out)
            }
        }
    }

    /// Buffer `index`: opened (or decoded) the first time it is asked for.
    fn buffer(&mut self, index: usize) -> Option<Rc<Buffer>> {
        if let Some(b) = self.buffers.get(&index) {
            return Some(Rc::clone(b));
        }
        let b = self.json.get("buffers")?.get(index)?;
        let len = uint(b, "byteLength")? as u64;
        let uri = b.get("uri").and_then(Value::as_str).map(str::to_owned);
        let buffer = match uri {
            // The GLB's own binary chunk, which only the first buffer may be.
            None => {
                let (file, base, have) = self.glb_bin.clone().filter(|_| index == 0)?;
                if have < len {
                    return None;
                }
                Buffer::File { file, base, len }
            }
            Some(uri) => match data_uri(&uri) {
                Some(payload) => Buffer::Bytes(self.decode_data(payload)?),
                None => {
                    let file = open_regular(&self.resolve(&uri)?)?;
                    if file.metadata().ok()?.len() < len {
                        return None;
                    }
                    Buffer::File { file: Rc::new(file), base: 0, len }
                }
            },
        };
        let buffer = Rc::new(buffer);
        self.buffers.insert(index, Rc::clone(&buffer));
        Some(buffer)
    }

    fn decode_data(&mut self, payload: &str) -> Option<Vec<u8>> {
        self.charge(payload.len() as u64 / 4 * 3)?;
        base64(payload)
    }

    /// Count `n` more bytes read; `None` once the model has had its share.
    fn charge(&mut self, n: u64) -> Option<()> {
        self.read = self.read.checked_add(n)?;
        (self.read <= MAX_READ_BYTES).then_some(())
    }

    /// A URI beside the model as a path. Only relative references are
    /// followed: a scheme (http:, file:) or an absolute path isn't a file
    /// that belongs with the model.
    fn resolve(&self, uri: &str) -> Option<PathBuf> {
        if uri.split('/').next().unwrap_or("").contains(':') {
            return None;
        }
        let decoded = urlencoding::decode(uri).ok()?;
        let rel = Path::new(decoded.as_ref());
        if rel.is_absolute() {
            return None;
        }
        Some(self.dir.join(rel))
    }
}

/// A regular file opened for reading. Never blocks on a FIFO and never
/// adopts a terminal (O_NONBLOCK, O_NOCTTY); anything but a regular file
/// is refused, so a symlink to /dev/zero can't be read without end.
pub(super) fn open_regular(path: &Path) -> Option<File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

/// The base64 payload of a `data:...;base64,` URI.
fn data_uri(uri: &str) -> Option<&str> {
    let (head, payload) = uri.strip_prefix("data:")?.split_once(',')?;
    head.ends_with(";base64").then_some(payload)
}

/// Standard base64 (and its URL-safe letters), stopping at the padding.
pub(super) fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in text.as_bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

fn comp_size(ctype: u32) -> Option<usize> {
    match ctype {
        BYTE | UNSIGNED_BYTE => Some(1),
        SHORT | UNSIGNED_SHORT => Some(2),
        UNSIGNED_INT | FLOAT => Some(4),
        _ => None,
    }
}

/// One component at the start of `b` (bounds already checked).
fn decode_f32(ctype: u32, normalized: bool, b: &[u8]) -> f32 {
    match ctype {
        BYTE => {
            let v = f32::from(b[0] as i8);
            if normalized { (v / 127.0).max(-1.0) } else { v }
        }
        UNSIGNED_BYTE => {
            let v = f32::from(b[0]);
            if normalized { v / 255.0 } else { v }
        }
        SHORT => {
            let v = f32::from(i16::from_le_bytes([b[0], b[1]]));
            if normalized { (v / 32767.0).max(-1.0) } else { v }
        }
        UNSIGNED_SHORT => {
            let v = f32::from(u16::from_le_bytes([b[0], b[1]]));
            if normalized { v / 65535.0 } else { v }
        }
        UNSIGNED_INT => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
        _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    }
}

fn decode_u32(ctype: u32, b: &[u8]) -> u32 {
    match ctype {
        BYTE | UNSIGNED_BYTE => u32::from(b[0]),
        SHORT | UNSIGNED_SHORT => u32::from(u16::from_le_bytes([b[0], b[1]])),
        _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

// ── JSON helpers ─────────────────────────────────────────────────────────────

/// `v[key]` as an index or count.
pub(super) fn uint(v: &Value, key: &str) -> Option<usize> {
    v.get(key)?.as_u64().and_then(|n| usize::try_from(n).ok())
}

pub(super) fn number(v: &Value, key: &str) -> Option<f32> {
    v.get(key)?.as_f64().map(|n| n as f32)
}

/// `v[key]` as exactly `N` numbers.
pub(super) fn numbers<const N: usize>(v: &Value, key: &str) -> Option<[f32; N]> {
    let a = v.get(key)?.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (o, n) in out.iter_mut().zip(a) {
        *o = n.as_f64()? as f32;
    }
    Some(out)
}
