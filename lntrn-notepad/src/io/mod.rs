//! Documents as files: reading one by what its name says it is, and
//! writing one so that a failure never leaves half a file.

pub mod lnote;
pub mod plain;
pub mod markdown;
pub mod docx;
pub mod xml;
pub mod zip;
pub mod pdf;

use std::path::Path;

use crate::doc::Doc;

/// What a file is, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Notepad's own: everything a document has.
    Lnote,
    Markdown,
    Docx,
    /// Anything else is text.
    Plain,
}

impl Kind {
    pub fn of(path: &Path) -> Kind {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("lnote") => Kind::Lnote,
            Some("md" | "markdown") => Kind::Markdown,
            Some("docx") => Kind::Docx,
            _ => Kind::Plain,
        }
    }

    /// Whether a file of this kind can hold everything `doc` has. When
    /// it can't, saving loses something, and the user is asked first.
    pub fn holds(self, doc: &Doc) -> bool {
        match self {
            Kind::Lnote | Kind::Docx => true,
            Kind::Markdown => markdown::holds(doc),
            Kind::Plain => plain::holds(doc),
        }
    }

    /// What it is called in a message.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Lnote => "Notepad document",
            Kind::Markdown => "Markdown file",
            Kind::Docx => "Word document",
            Kind::Plain => "text file",
        }
    }
}

/// A file's bytes as text: UTF-8, with or without its mark; failing
/// that, Latin-1, which any bytes are. A file with a zero byte in it is
/// not text at all.
fn text_of(bytes: &[u8]) -> Result<String, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(_) if bytes.contains(&0) => Err("This doesn't look like a text file.".to_owned()),
        Err(_) => Ok(bytes.iter().map(|b| char::from(*b)).collect()),
    }
}

/// Read the file at `path` as whatever its name says it is.
pub fn load(path: &Path) -> Result<Doc, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match Kind::of(path) {
        Kind::Docx => docx::read(&bytes),
        // One of ours that doesn't start as one is read as the text it is.
        Kind::Lnote => text_of(&bytes).map(|text| lnote::read(&text).unwrap_or_else(|| plain::read(&text))),
        Kind::Markdown => text_of(&bytes).map(|text| markdown::read(&text, path.parent())),
        Kind::Plain => text_of(&bytes).map(|text| plain::read(&text)),
    }
}

/// `doc` as a file of the kind `path` names. `body` is body text's size,
/// for the kinds that have sizes of their own.
pub fn bytes_for(path: &Path, doc: &Doc, body: f32) -> Vec<u8> {
    match Kind::of(path) {
        Kind::Lnote => lnote::write(doc).into_bytes(),
        Kind::Docx => docx::write(doc, body, crate::paper::Paper::here()),
        Kind::Markdown => markdown::write(doc).into_bytes(),
        _ => {
            let mut text = plain::write(doc);
            text.push('\n');
            text.into_bytes()
        }
    }
}

/// Write `bytes` to `path` whole or not at all: to a file beside it
/// first, which then takes its place.
pub fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("{}: {e}", path.display())
    })
}

/// Save `doc` at `path`.
pub fn save(path: &Path, doc: &Doc, body: f32) -> Result<(), String> {
    write_whole(path, &bytes_for(path, doc, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{List, Pos, Style};

    #[test]
    fn a_file_is_read_and_written_as_what_its_name_says() {
        let dir = std::env::temp_dir().join(format!("lntrn-notepad-io-{}", std::process::id()));
        let mut doc = Doc::from_text("Title\nan item");
        doc.set_paras(0, 0, |a| a.style = Style::Title);
        doc.set_paras(1, 1, |a| a.list = List::Bullet);
        doc.format(Pos::new(1, 0), Pos::new(1, 2), |a| a.bold = true);
        // Ours holds all of it, in a folder that wasn't there yet.
        let rich = dir.join("deep/note.lnote");
        assert!(Kind::of(&rich).holds(&doc));
        save(&rich, &doc, 24.0).unwrap();
        assert_eq!(load(&rich).unwrap().paras, doc.paras);
        assert!(!dir.join("deep/note.lnote.part").exists());
        // Text holds the words and the bullet, and says it can't hold
        // the rest.
        let text = dir.join("note.TXT");
        assert_eq!((Kind::of(&text), Kind::of(&text).holds(&doc)), (Kind::Plain, false));
        save(&text, &doc, 24.0).unwrap();
        assert_eq!(std::fs::read_to_string(&text).unwrap(), "Title\n- an item\n");
        let back = load(&text).unwrap();
        assert_eq!((back.text(), back.para(1).attrs.list, back.para(0).attrs.style), ("Title\nan item".to_owned(), List::Bullet, Style::Body));
        assert!(Kind::of(&text).holds(&back));
        // Bytes that aren't UTF-8 are read as Latin-1; a marked file
        // loses its mark; one with zero bytes in it is refused, as is one
        // that isn't there. A save that can't be made says so.
        std::fs::write(&text, b"caf\xE9\r\n").unwrap();
        assert_eq!(load(&text).unwrap().text(), "caf\u{e9}");
        std::fs::write(&text, b"\xEF\xBB\xBFmarked").unwrap();
        assert_eq!(load(&text).unwrap().text(), "marked");
        std::fs::write(&text, b"\x00\x01\xFF\xFE").unwrap();
        assert!(load(&text).is_err() && load(&dir.join("nowhere.txt")).is_err());
        assert!(save(&dir.join("note.TXT/inside.txt"), &doc, 24.0).is_err());
        // A file named as ours that isn't one is read as text, not refused.
        std::fs::write(&rich, "just words").unwrap();
        assert_eq!(load(&rich).unwrap().text(), "just words");
        assert_eq!((Kind::of(Path::new("a.MD")), Kind::of(Path::new("b.docx")), Kind::of(Path::new("noext"))), (Kind::Markdown, Kind::Docx, Kind::Plain));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
