//! Zip archives: files packed one after another, and at the end a
//! directory saying where each of them is. A Word document is one.
//!
//! ```text
//! local header, name, data      once for each file
//! directory record, name        once for each file, all together
//! end record                    where the directory is and how many it lists
//! ```
//!
//! Written, every file is deflated, or stored as it is when deflating
//! makes it no smaller; names are UTF-8 and every date is the first a zip
//! can hold, so the same files always make the same bytes.
//!
//! Read, only the directory is believed, and nothing of it unchecked:
//! every place is looked for before it is read, every file must be as
//! long as it says and have the CRC-32 it says, and what an archive
//! unpacks to has a limit. Zip64, passwords and other ways of packing are
//! refused by name; nothing malformed is a panic.

use lntrn_image::{Compression, deflate, inflate};

/// The most an archive's files may come to once unpacked.
const MOST: u64 = 256 << 20;

/// What each kind of record starts with.
const LOCAL: u32 = 0x0403_4b50;
const RECORD: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// What stands before the end record of a zip64 archive.
const END_64: u32 = 0x0706_4b50;

/// The version of the format that deflate asks for: 2.0.
const VERSION: u16 = 20;
/// The flag saying names are UTF-8.
const UTF8: u16 = 1 << 11;
/// The flags of a file locked with a password, the old way and the new.
const LOCKED: u16 = 1 | (1 << 6);
/// 1 January 1980, with midnight for its time.
const DATE: u16 = (1 << 5) | 1;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

const ZIP64: &str = "the zip archive is in the zip64 format, which is not read";

/// The upper half of code page 437, which names are in when they are not
/// UTF-8.
const CP437: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";

/// CRC-32 tables for eight bytes at a step: `[0]` is the usual one a byte
/// at a time, and each one after it is the one before carried a byte on.
static TABLES: [[u32; 256]; 8] = tables();

const fn tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut bit = 0;
        while bit < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            bit += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            t[k][i] = (t[k - 1][i] >> 8) ^ t[0][(t[k - 1][i] & 0xFF) as usize];
            i += 1;
        }
        k += 1;
    }
    t
}

/// The CRC-32 of zip, gzip and PNG.
pub fn crc32(data: &[u8]) -> u32 {
    let byte = |table: usize, word: u32, shift: u32| TABLES[table][((word >> shift) & 0xFF) as usize];
    let mut crc = !0u32;
    let mut eights = data.chunks_exact(8);
    for c in &mut eights {
        let (lo, hi) = (crc ^ u32::from_le_bytes([c[0], c[1], c[2], c[3]]), u32::from_le_bytes([c[4], c[5], c[6], c[7]]));
        crc = byte(7, lo, 0) ^ byte(6, lo, 8) ^ byte(5, lo, 16) ^ byte(4, lo, 24) ^ byte(3, hi, 0) ^ byte(2, hi, 8) ^ byte(1, hi, 16) ^ byte(0, hi, 24);
    }
    for &b in eights.remainder() {
        crc = byte(0, crc ^ u32::from(b), 0) ^ (crc >> 8);
    }
    !crc
}

/// A length or a place as the four bytes a zip has for it.
fn four(n: usize) -> u32 {
    u32::try_from(n).expect("a zip without zip64 holds less than 4 GiB")
}

/// The files of an archive, in order: name and bytes.
///
/// There is no zip64 here, so an archive has fewer than 65 535 files and
/// less than 4 GiB in it: more is a panic, not an archive that cannot be
/// read back.
pub fn write(entries: &[(&str, &[u8])]) -> Vec<u8> {
    assert!(entries.len() < 0xFFFF, "a zip without zip64 lists fewer than 65535 files");
    let (mut out, mut directory) = (Vec::new(), Vec::new());
    for (name, data) in entries {
        // lntrn-image deflates into a zlib stream: two bytes before the
        // deflated data and an Adler-32 after it, neither of which a zip
        // has.
        let zlib = if data.is_empty() { Vec::new() } else { deflate::zlib(data, Compression::Default) };
        let (method, body) = match zlib.get(2..zlib.len().saturating_sub(4)) {
            Some(raw) if raw.len() < data.len() => (DEFLATED, raw),
            _ => (STORED, *data),
        };
        // What a file's local header and its directory record both say.
        let mut both = Vec::with_capacity(24);
        for n in [VERSION, UTF8, method, 0, DATE] {
            both.extend_from_slice(&n.to_le_bytes());
        }
        for n in [crc32(data), four(body.len()), four(data.len())] {
            both.extend_from_slice(&n.to_le_bytes());
        }
        both.extend_from_slice(&u16::try_from(name.len()).expect("a name in a zip is under 64 KiB").to_le_bytes());
        let at = four(out.len());
        out.extend_from_slice(&LOCAL.to_le_bytes());
        out.extend_from_slice(&both);
        out.extend_from_slice(&[0; 2]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        directory.extend_from_slice(&RECORD.to_le_bytes());
        directory.extend_from_slice(&VERSION.to_le_bytes());
        directory.extend_from_slice(&both);
        // Nothing extra, no comment, the first disk, no attributes.
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&at.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let (start, count) = (four(out.len()), entries.len() as u16);
    out.extend_from_slice(&directory);
    out.extend_from_slice(&END.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&four(directory.len()).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

/// The number in the two bytes at `at`, when they are there.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// Where the end record is: the last place its mark stands with room
/// after it for the comment it says it has.
fn end_record(bytes: &[u8]) -> Option<usize> {
    let last = bytes.len().checked_sub(22)?;
    (last.saturating_sub(0xFFFF)..=last).rev().find(|&at| u32_at(bytes, at) == Some(END) && u16_at(bytes, at + 20).is_some_and(|comment| at + 22 + usize::from(comment) <= bytes.len()))
}

/// A name as text: UTF-8 when it is that, else code page 437.
fn name_of(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(name) => name.to_owned(),
        Err(_) => raw.iter().map(|&b| if b < 0x80 { char::from(b) } else { CP437.chars().nth(usize::from(b) - 0x80).unwrap_or('?') }).collect(),
    }
}

/// Whether unpacking a file called `name` into a folder could put it
/// somewhere else: a path from the top, from a drive, or up out of it.
fn escapes(name: &str) -> bool {
    let drive = name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) && name.as_bytes().get(1) == Some(&b':');
    drive || name.starts_with(['/', '\\']) || name.split(['/', '\\']).any(|part| part == "..")
}

/// The start of a name, for a message: one may be 64 KiB long.
fn brief(name: &str) -> &str {
    name.char_indices().nth(80).map_or(name, |(end, _)| &name[..end])
}

/// Deflated data unpacked, when it comes to exactly `size` bytes.
fn unpack(raw: &[u8], size: usize) -> Option<Vec<u8>> {
    // lntrn-image inflates a zlib stream, so the two bytes one starts
    // with go in front. The Adler-32 one ends with is never looked at.
    let mut zlib = Vec::with_capacity(raw.len() + 2);
    zlib.extend_from_slice(&[0x78, 0x9C]);
    zlib.extend_from_slice(raw);
    inflate::inflate_limit(&zlib, size).filter(|out| out.len() == size)
}

/// A file as the directory lists it.
struct Listed {
    name: String,
    method: u16,
    crc: u32,
    packed: u32,
    size: u32,
    header: u32,
}

/// The files of an archive in the directory's order, folders left out,
/// or why it is not read.
pub fn read(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let cut = || "the zip archive is cut short or damaged".to_owned();
    let n16 = |at: usize| u16_at(bytes, at).ok_or_else(cut);
    let n32 = |at: usize| u32_at(bytes, at).ok_or_else(cut);
    // What starts as an archive and has no end record has lost its end.
    let end = end_record(bytes).ok_or_else(|| if u32_at(bytes, 0) == Some(LOCAL) { cut() } else { "not a zip archive".to_owned() })?;
    let (disk, here, count, length, start) = (n16(end + 4)?, n16(end + 8)?, n16(end + 10)?, n32(end + 12)?, n32(end + 16)?);
    if count == u16::MAX || length == u32::MAX || start == u32::MAX || end.checked_sub(20).and_then(|at| u32_at(bytes, at)) == Some(END_64) {
        return Err(ZIP64.to_owned());
    }
    if disk != 0 || here != count {
        return Err("the zip archive is one part of several".to_owned());
    }
    // Whatever stands before the archive (a program that unpacks it, say)
    // moves every place in it along by its own length.
    let directory = end.checked_sub(length as usize).ok_or_else(cut)?;
    let shift = directory.checked_sub(start as usize).ok_or_else(cut)?;
    // All of the directory is read before any file is unpacked, so what
    // the whole archive comes to is known first.
    let mut listed = Vec::new();
    let (mut at, mut total) = (directory, 0u64);
    for _ in 0..count {
        if n32(at)? != RECORD {
            return Err(cut());
        }
        let (flags, method, crc, packed, size, header) = (n16(at + 8)?, n16(at + 10)?, n32(at + 16)?, n32(at + 20)?, n32(at + 24)?, n32(at + 42)?);
        let (name_len, more) = (usize::from(n16(at + 28)?), usize::from(n16(at + 30)?) + usize::from(n16(at + 32)?));
        let name = name_of(bytes.get(at + 46..at + 46 + name_len).ok_or_else(cut)?);
        at += 46 + name_len + more;
        if escapes(&name) {
            return Err(format!("the zip archive has a file that would be put outside it: {}", brief(&name)));
        }
        if name.ends_with('/') {
            continue;
        }
        if flags & LOCKED != 0 {
            return Err("the zip archive is locked with a password".to_owned());
        }
        if packed == u32::MAX || size == u32::MAX || header == u32::MAX {
            return Err(ZIP64.to_owned());
        }
        if method != STORED && method != DEFLATED {
            return Err(format!("{} in the zip archive is packed in a way that is not read (method {method})", brief(&name)));
        }
        total += u64::from(size);
        if total > MOST {
            return Err(format!("the zip archive unpacks to more than {} MiB", MOST >> 20));
        }
        listed.push(Listed { name, method, crc, packed, size, header });
    }
    let mut files = Vec::with_capacity(listed.len());
    for file in listed {
        let damaged = || format!("{} in the zip archive is damaged", brief(&file.name));
        let local = (file.header as usize).checked_add(shift).ok_or_else(cut)?;
        if n32(local)? != LOCAL {
            return Err(cut());
        }
        // The header before a file has its name again and may have more
        // after it, neither as long as the directory's need be.
        let from = local + 30 + usize::from(n16(local + 26)?) + usize::from(n16(local + 28)?);
        let data = from.checked_add(file.packed as usize).and_then(|to| bytes.get(from..to)).ok_or_else(cut)?;
        // A file is never let grow past the size it was counted as.
        let unpacked = match file.method {
            STORED if file.packed == file.size => data.to_vec(),
            STORED => return Err(damaged()),
            _ => unpack(data, file.size as usize).ok_or_else(damaged)?,
        };
        if crc32(&unpacked) != file.crc {
            return Err(damaged());
        }
        files.push((file.name, unpacked));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes that do not pack: the same well mixed run every time.
    fn noise(len: usize) -> Vec<u8> {
        let mut x = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 32) as u8
        };
        (0..len).map(|_| next()).collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Where each of the directory's records is, in an archive with no
    /// comment.
    fn records(zip: &[u8]) -> Vec<usize> {
        let end = zip.len() - 22;
        let mut at = u32_at(zip, end + 16).unwrap() as usize;
        let mut next = || {
            let here = at;
            at += 46 + [28, 30, 32].iter().map(|field| usize::from(u16_at(zip, here + field).unwrap())).sum::<usize>();
            here
        };
        (0..u16_at(zip, end + 10).unwrap()).map(|_| next()).collect()
    }

    /// `zip` with `bytes` written over what is at `at`.
    fn with(zip: &[u8], at: usize, bytes: &[u8]) -> Vec<u8> {
        let mut out = zip.to_vec();
        out[at..at + bytes.len()].copy_from_slice(bytes);
        out
    }

    /// What `python3 -c script args` printed, or `None` on a machine
    /// without python3.
    fn python(script: &str, args: &[&str]) -> Option<Vec<u8>> {
        let out = std::process::Command::new("python3").arg("-c").arg(script).args(args).output().ok()?;
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        Some(out.stdout)
    }

    #[test]
    fn what_is_packed_comes_back_as_it_was() {
        assert_eq!((crc32(b""), crc32(b"a"), crc32(b"123456789"), crc32(b"The quick brown fox jumps over the lazy dog")), (0, 0xE8B7_BE43, 0xCBF4_3926, 0x414F_A339));
        // Nothing at all, a file too small to pack, words that pack and
        // are long enough to be deflated in parts, bytes that don't, and
        // a name that is not ASCII.
        assert_eq!((write(&[]).len(), read(&write(&[])).unwrap().len()), (22, 0));
        let (words, binary, every) = (b"The same words over and over. ".repeat(40_000), noise(70_000), (0..=255).collect::<Vec<u8>>());
        let entries: [(&str, &[u8]); 6] = [("empty.txt", b""), ("tiny.txt", b"hi"), ("deep/in/words.txt", &words), ("noise.bin", &binary), ("every byte", &every), ("caf\u{e9}/\u{1F3EE} lantern.txt", "light".as_bytes())];
        let zip = write(&entries);
        assert!(zip.len() < words.len() / 50 + binary.len() + every.len() + 1_000, "the words pack: {} bytes", zip.len());
        assert_eq!(records(&zip).iter().map(|r| u16_at(&zip, r + 10).unwrap()).collect::<Vec<_>>(), [STORED, STORED, DEFLATED, STORED, STORED, STORED]);
        let back = read(&zip).unwrap();
        assert_eq!(back.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), entries.map(|(name, _)| name));
        assert!(back.iter().zip(&entries).all(|((_, got), (_, put))| got == put));
        assert_eq!(write(&entries), zip, "the same files make the same bytes");
        // Thousands of files, each found again.
        let many: Vec<(String, Vec<u8>)> = (0..3000).map(|i| (format!("part/{i}.txt"), format!("file {i} ").repeat(i % 40).into_bytes())).collect();
        let zip = write(&many.iter().map(|(name, data)| (name.as_str(), data.as_slice())).collect::<Vec<_>>());
        assert!(read(&zip).unwrap() == many);
        // A folder's own entry is no file; something before the archive
        // and a comment after it change nothing; an old name is read in
        // the old code page.
        let zip = write(&[("folder/", b""), ("folder/cafX.txt", b"one")]);
        assert_eq!(read(&zip).unwrap(), [("folder/cafX.txt".to_owned(), b"one".to_vec())]);
        let mut wrapped = [b"#!/bin/sh\nexit\n".as_slice(), &zip, b"a comment"].concat();
        let comment = wrapped.len() - 11;
        wrapped[comment] = 9;
        assert_eq!(read(&wrapped).unwrap(), read(&zip).unwrap());
        let mut old = zip.clone();
        for at in (0..zip.len() - 3).filter(|&at| &zip[at..at + 4] == b"cafX") {
            old[at + 3] = 0x82;
        }
        assert_eq!(read(&old).unwrap()[0].0, "folder/caf\u{e9}.txt");
    }

    #[test]
    fn a_broken_or_hostile_archive_is_refused_not_believed() {
        let words = b"words ".repeat(60);
        let zip = write(&[("a.txt", b"hello"), ("b.txt", &words)]);
        let (r, end) = (records(&zip), zip.len() - 22);
        let refused = |zip: &[u8]| read(zip).unwrap_err();
        // Cut short anywhere it is refused, and no one byte of it changed
        // is a panic; a changed byte of a file is caught.
        for len in 0..zip.len() {
            assert!(read(&zip[..len]).is_err(), "cut to {len} bytes");
        }
        for at in 0..zip.len() {
            for bits in [0x01, 0x80, 0xFF] {
                let changed = read(&with(&zip, at, &[zip[at] ^ bits]));
                assert!(changed.is_err() || !(35..40).contains(&at), "byte {at} of a file changed");
            }
        }
        for text in ["", "PK", "PK\x05\x06", "some text long enough to hold an end record"] {
            assert_eq!(refused(text.as_bytes()), "not a zip archive");
        }
        // The wrong checksum, and a file that is not as long as it says:
        // stored, deflated to more, deflated to less.
        assert_eq!(refused(&with(&zip, r[0] + 16, &[1, 2, 3, 4])), "a.txt in the zip archive is damaged");
        assert_eq!(refused(&with(&zip, r[0] + 24, &6u32.to_le_bytes())), "a.txt in the zip archive is damaged");
        for size in [0u32, 10, words.len() as u32 - 1, words.len() as u32 + 1] {
            assert_eq!(refused(&with(&zip, r[1] + 24, &size.to_le_bytes())), "b.txt in the zip archive is damaged");
        }
        // More than may be unpacked, in one file or between them, is
        // refused before any of it is.
        let much = (200u32 << 20).to_le_bytes();
        assert!(refused(&with(&zip, r[1] + 24, &(300u32 << 20).to_le_bytes())).contains("more than 256 MiB"));
        assert!(refused(&with(&with(&zip, r[0] + 24, &much), r[1] + 24, &much)).contains("more than 256 MiB"));
        assert!(refused(&with(&zip, r[1] + 24, &much)).contains("b.txt in the zip archive is damaged"), "under the limit it is unpacked, and found out");
        // Names that lead out of the folder an archive is unpacked into.
        for name in ["../evil.txt", "/etc/passwd", "a/../../b", "..", "C:\\boot.ini", "c:evil", "\\\\server\\share", "a\\..\\b"] {
            assert!(refused(&write(&[(name, b"x")])).contains("outside it"), "{name}");
        }
        assert!(read(&write(&[("a..b/c...txt", b"x"), ("..a/b..", b"y"), ("cc:d", b"z")])).is_ok());
        assert!(refused(&write(&[(&format!("../{}", "x".repeat(60_000)), b"x")])).len() < 200, "a name is not quoted at its whole length");
        // What is not read is refused by name.
        assert!(refused(&with(&zip, r[0] + 8, &(UTF8 | 1).to_le_bytes())).contains("password"));
        assert!(refused(&with(&zip, r[1] + 10, &12u16.to_le_bytes())).contains("b.txt in the zip archive is packed in a way that is not read (method 12)"));
        assert!(refused(&with(&zip, r[0] + 24, &[0xFF; 4])).contains("zip64") && refused(&with(&zip, end + 10, &[0xFF; 2])).contains("zip64"));
        assert!(refused(&with(&zip, end + 4, &[1])).contains("one part of several"));
        // A directory or a file that is not where it is said to be.
        for (at, bytes) in [(r[1], [0u8; 4]), (r[1] + 42, [0xF0, 0xFF, 0, 0]), (end + 16, [1, 0, 0, 0]), (end + 12, [0xFF, 0xFF, 0, 0]), (0, *b"JUNK")] {
            assert!(refused(&with(&zip, at, &bytes)).contains("cut short or damaged"), "{at}");
        }
    }

    #[test]
    fn python_reads_what_this_writes_and_this_reads_what_python_writes() {
        let dir = std::env::temp_dir().join(format!("lntrn-notepad-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (ours, theirs) = (dir.join("ours.zip"), dir.join("theirs.zip"));
        let (words, every) = (b"The same words over and over. ".repeat(20_000), (0..=255).collect::<Vec<u8>>().repeat(40));
        let binary = noise(5_000);
        let entries: [(&str, &[u8]); 4] = [("empty", b""), ("deep/in/words.txt", &words), ("noise.bin", &binary), ("caf\u{e9} \u{1F3EE}.txt", "light".as_bytes())];
        std::fs::write(&ours, write(&entries)).unwrap();
        // Python finds nothing wrong with ours, and the same names, bytes
        // and checksums in it.
        let list = "import sys, zipfile, zlib\nz = zipfile.ZipFile(sys.argv[1])\nprint(z.testzip())\nfor i in z.infolist():\n    d = z.read(i)\n    print(i.filename.encode().hex(), d.hex(), zlib.crc32(d))";
        let pack = "import sys, zipfile\nout = open(sys.argv[1], 'wb') if len(sys.argv) > 1 else sys.stdout.buffer\nwith zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as z:\n    z.writestr('words.txt', b'The same words over and over. ' * 20000)\n    z.writestr('folder/', b'')\n    z.writestr('folder/caf\\u00e9.bin', bytes(range(256)) * 40)\n    z.writestr(zipfile.ZipInfo('stored.txt'), b'as it is', zipfile.ZIP_STORED)\n    z.writestr('empty', b'')\n    z.comment = b'made by python'";
        if let Some(seen) = python(list, &[ours.to_str().unwrap()]) {
            let expected = entries.iter().map(|(name, data)| format!("{} {} {}\n", hex(name.as_bytes()), hex(data), crc32(data))).collect::<String>();
            assert!(String::from_utf8_lossy(&seen) == format!("None\n{expected}"), "python read something else out of it");
            // What Python deflated is read: from a file, and as it writes
            // to a pipe, when sizes and checksums come after each file.
            let theirs_holds: [(&str, &[u8]); 4] = [("words.txt", &words), ("folder/caf\u{e9}.bin", &every), ("stored.txt", b"as it is"), ("empty", b"")];
            let theirs_holds = theirs_holds.map(|(name, data)| (name.to_owned(), data.to_vec()));
            python(pack, &[theirs.to_str().unwrap()]).unwrap();
            let (file, piped) = (std::fs::read(&theirs).unwrap(), python(pack, &[]).unwrap());
            assert!(file != piped && records(&file[..file.len() - 14]).iter().any(|r| u16_at(&file, r + 10) == Some(DEFLATED)));
            assert!(read(&file).unwrap() == theirs_holds && read(&piped).unwrap() == theirs_holds);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
