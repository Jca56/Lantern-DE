//! Document persistence — plain text (with the markdown `- ` bullet trick)
//! and the `.lnote` rich format that carries full span + paragraph formatting.
//! Pure functions over `(lines, formats)`; `Editor::load_file`/`save_file`
//! own the file I/O and call in here.
//!
//! ## The .lnote format (version 1)
//!
//! A small line-oriented header, then the document text verbatim:
//!
//! ```text
//! lnote 1
//! para 0 align=center
//! para 3 bullet before=8
//! span 0 0 12 bold size=32
//! span 5 4 9 italic font=Lora color=cc3a34
//! text
//! <document text, lines joined with \n, completely unescaped>
//! ```
//!
//! - `para <line> [bullet] [align=center|right|justify] [before=F] [after=F]
//!   [indent=F]` — emitted only for lines whose paragraph attrs are non-default.
//! - `span <line> <start> <end> [bold] [italic] [underline] [strike] [size=F]
//!   [font=FileBase] [color=RRGGBB] [bg=RRGGBB]` — byte offsets into that
//!   line; emitted only for non-default spans.
//! - Fonts are stored by catalog filename base (`OpenSans` — stable across
//!   catalog reordering, never contains spaces); unknown fonts degrade to the
//!   default sans instead of failing.
//! - Everything after the `text` marker line is the body verbatim — no
//!   escaping, so the format can never mangle user text.
//! - Unknown records/tokens are skipped (forward compatible); a malformed
//!   header falls back to plain-text parsing so no file is ever unopenable.

use std::path::Path;

use crate::format::{Alignment, DocFormats, ParagraphAttrs, TextAttrs};
use crate::fonts;

/// True when this path should round-trip the rich `.lnote` format.
pub fn is_lnote(path: &Path) -> bool {
    path.extension()
        .map_or(false, |e| e.eq_ignore_ascii_case("lnote"))
}

/// Parse file content into document lines + formats, picking the format by
/// the path's extension. A `.lnote` that fails to parse degrades to plain.
pub fn parse(path: &Path, content: &str) -> (Vec<String>, DocFormats) {
    if is_lnote(path) {
        if let Some(doc) = parse_lnote(content) {
            return doc;
        }
    }
    parse_plain(content)
}

/// Serialize a document for the given path — rich for `.lnote`, plain text
/// (bullets as `- ` prefixes) for everything else.
pub fn serialize(path: &Path, lines: &[String], formats: &DocFormats) -> String {
    if is_lnote(path) {
        serialize_lnote(lines, formats)
    } else {
        serialize_plain(lines, formats)
    }
}

// ── Plain text ──────────────────────────────────────────────────────────────

fn parse_plain(content: &str) -> (Vec<String>, DocFormats) {
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    let mut formats = DocFormats::new(lines.len());
    // Markdown-style "- " prefixes become bullet paragraphs; serialize_plain
    // writes them back the same way so bullets round-trip losslessly.
    for (i, line) in lines.iter_mut().enumerate() {
        if line.starts_with("- ") {
            line.replace_range(..2, "");
            formats.get_mut(i).para.bullet = true;
        }
    }
    (lines, formats)
}

fn serialize_plain(lines: &[String], formats: &DocFormats) -> String {
    let mut content = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            content.push('\n');
        }
        if formats.get(i).para.bullet {
            content.push_str("- ");
        }
        content.push_str(line);
    }
    content
}

// ── .lnote serializer ───────────────────────────────────────────────────────

fn serialize_lnote(lines: &[String], formats: &DocFormats) -> String {
    let mut out = String::from("lnote 1\n");
    for (i, line) in lines.iter().enumerate() {
        let para = formats.get(i).para;
        if para != ParagraphAttrs::default() {
            push_para_record(&mut out, i, &para);
        }
        for span in formats.get(i).iter_spans(line.len()) {
            if !span.attrs.is_default() {
                push_span_record(&mut out, i, span.start, span.end, &span.attrs);
            }
        }
    }
    out.push_str("text\n");
    out.push_str(&lines.join("\n"));
    out
}

fn push_para_record(out: &mut String, line: usize, para: &ParagraphAttrs) {
    out.push_str(&format!("para {line}"));
    if para.bullet {
        out.push_str(" bullet");
    }
    match para.alignment {
        Alignment::Left => {}
        Alignment::Center => out.push_str(" align=center"),
        Alignment::Right => out.push_str(" align=right"),
        Alignment::Justify => out.push_str(" align=justify"),
    }
    if para.space_before != 0.0 {
        out.push_str(&format!(" before={}", para.space_before));
    }
    if para.space_after != 0.0 {
        out.push_str(&format!(" after={}", para.space_after));
    }
    if para.first_indent != 0.0 {
        out.push_str(&format!(" indent={}", para.first_indent));
    }
    out.push('\n');
}

fn push_span_record(out: &mut String, line: usize, start: usize, end: usize, a: &TextAttrs) {
    out.push_str(&format!("span {line} {start} {end}"));
    if a.bold {
        out.push_str(" bold");
    }
    if a.italic {
        out.push_str(" italic");
    }
    if a.underline {
        out.push_str(" underline");
    }
    if a.strikethrough {
        out.push_str(" strike");
    }
    if let Some(fs) = a.font_size {
        out.push_str(&format!(" size={fs}"));
    }
    if let Some(base) = a.font.and_then(|i| fonts::file_base_for_index(i as usize)) {
        out.push_str(&format!(" font={base}"));
    }
    if let Some(c) = a.color {
        out.push_str(&format!(" color={c:06x}"));
    }
    if let Some(c) = a.bg_color {
        out.push_str(&format!(" bg={c:06x}"));
    }
    out.push('\n');
}

// ── .lnote parser ───────────────────────────────────────────────────────────

fn parse_lnote(content: &str) -> Option<(Vec<String>, DocFormats)> {
    let (first, mut rest) = content.split_once('\n')?;
    let mut hdr = first.split_whitespace();
    if hdr.next()? != "lnote" {
        return None;
    }
    let _version: u32 = hdr.next()?.parse().ok()?;

    // Records run until the body marker; the body is everything after it,
    // taken verbatim. A header that never reaches `text` is malformed.
    let mut records: Vec<&str> = Vec::new();
    let body = loop {
        let (line, tail) = rest.split_once('\n')?;
        if line == "text" {
            break tail;
        }
        records.push(line);
        rest = tail;
    };

    // split('\n') (not .lines()) preserves trailing empty lines exactly as
    // the serializer's join("\n") wrote them.
    let lines: Vec<String> = body.split('\n').map(|l| l.to_string()).collect();
    let mut formats = DocFormats::new(lines.len());

    for rec in records {
        let mut toks = rec.split_whitespace();
        match toks.next() {
            Some("para") => apply_para_record(&mut formats, &lines, toks),
            Some("span") => apply_span_record(&mut formats, &lines, toks),
            _ => {} // unknown record type — forward compatible, skip
        }
    }
    Some((lines, formats))
}

fn apply_para_record<'a>(
    formats: &mut DocFormats,
    lines: &[String],
    mut toks: impl Iterator<Item = &'a str>,
) {
    let Some(idx) = toks.next().and_then(|t| t.parse::<usize>().ok()) else {
        return;
    };
    if idx >= lines.len() {
        return;
    }
    let para = &mut formats.get_mut(idx).para;
    for tok in toks {
        if tok == "bullet" {
            para.bullet = true;
        } else if let Some(v) = tok.strip_prefix("align=") {
            para.alignment = match v {
                "center" => Alignment::Center,
                "right" => Alignment::Right,
                "justify" => Alignment::Justify,
                _ => Alignment::Left,
            };
        } else if let Some(v) = tok.strip_prefix("before=") {
            if let Ok(f) = v.parse::<f32>() {
                para.space_before = f;
            }
        } else if let Some(v) = tok.strip_prefix("after=") {
            if let Ok(f) = v.parse::<f32>() {
                para.space_after = f;
            }
        } else if let Some(v) = tok.strip_prefix("indent=") {
            if let Ok(f) = v.parse::<f32>() {
                para.first_indent = f;
            }
        }
        // unknown token — skip (forward compatible)
    }
}

fn apply_span_record<'a>(
    formats: &mut DocFormats,
    lines: &[String],
    mut toks: impl Iterator<Item = &'a str>,
) {
    let next_usize = |toks: &mut dyn Iterator<Item = &'a str>| -> Option<usize> {
        toks.next().and_then(|t| t.parse::<usize>().ok())
    };
    let (Some(idx), Some(start), Some(end)) = (
        next_usize(&mut toks),
        next_usize(&mut toks),
        next_usize(&mut toks),
    ) else {
        return;
    };
    if idx >= lines.len() {
        return;
    }
    // Defend against hand-edited offsets: clamp to the line, require valid
    // char boundaries, skip rather than fail.
    let line = &lines[idx];
    let end = end.min(line.len());
    if start >= end || !line.is_char_boundary(start) || !line.is_char_boundary(end) {
        return;
    }

    let mut attrs = TextAttrs::default();
    for tok in toks {
        match tok {
            "bold" => attrs.bold = true,
            "italic" => attrs.italic = true,
            "underline" => attrs.underline = true,
            "strike" => attrs.strikethrough = true,
            _ => {
                if let Some(v) = tok.strip_prefix("size=") {
                    if let Ok(f) = v.parse::<f32>() {
                        attrs.font_size = Some(f);
                    }
                } else if let Some(v) = tok.strip_prefix("font=") {
                    // Unknown family (catalog changed) degrades to default.
                    if let Some(i) = fonts::family_index_for_file(v) {
                        attrs.font = Some(i as u8);
                    }
                } else if let Some(v) = tok.strip_prefix("color=") {
                    if let Ok(c) = u32::from_str_radix(v, 16) {
                        attrs.color = Some(c);
                    }
                } else if let Some(v) = tok.strip_prefix("bg=") {
                    if let Ok(c) = u32::from_str_radix(v, 16) {
                        attrs.bg_color = Some(c);
                    }
                }
                // unknown token — skip (forward compatible)
            }
        }
    }
    if !attrs.is_default() {
        formats.get_mut(idx).apply_format(start, end, |a| *a = attrs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn lnote_path() -> PathBuf {
        PathBuf::from("doc.lnote")
    }

    /// Every span + paragraph attribute must survive serialize → parse.
    #[test]
    fn lnote_round_trips_all_formatting() {
        let lines: Vec<String> = vec![
            "Big Centered Title".into(),
            "".into(),
            "body with styled words".into(),
            "a bullet item".into(),
        ];
        let mut formats = DocFormats::new(lines.len());
        formats.get_mut(0).para.alignment = Alignment::Center;
        formats.get_mut(0).para.space_after = 8.0;
        formats.get_mut(0).apply_format(0, 18, |a| {
            a.bold = true;
            a.font_size = Some(32.0);
        });
        formats.get_mut(2).para.first_indent = 24.0;
        formats.get_mut(2).apply_format(10, 16, |a| {
            a.italic = true;
            a.underline = true;
            a.strikethrough = true;
            a.font = Some(16); // Lora
            a.color = Some(0xcc3a34);
            a.bg_color = Some(0xfff0c0);
        });
        formats.get_mut(3).para.bullet = true;

        let text = serialize(&lnote_path(), &lines, &formats);
        let (lines2, formats2) = parse(&lnote_path(), &text);

        assert_eq!(lines2, lines);
        assert_eq!(formats2.get(0).para.alignment, Alignment::Center);
        assert_eq!(formats2.get(0).para.space_after, 8.0);
        let title = formats2.get(0).attrs_at(5);
        assert!(title.bold);
        assert_eq!(title.font_size, Some(32.0));
        assert_eq!(formats2.get(2).para.first_indent, 24.0);
        let styled = formats2.get(2).attrs_at(12);
        assert!(styled.italic && styled.underline && styled.strikethrough);
        assert_eq!(styled.font, Some(16));
        assert_eq!(styled.color, Some(0xcc3a34));
        assert_eq!(styled.bg_color, Some(0xfff0c0));
        assert!(formats2.get(2).attrs_at(2).is_default(), "unstyled stays plain");
        assert!(formats2.get(3).para.bullet);
        assert!(!formats2.get(1).para.bullet);
    }

    /// Body text that looks like header records must come through verbatim —
    /// everything after the `text` marker is untouched.
    #[test]
    fn lnote_body_is_verbatim() {
        let lines: Vec<String> = vec![
            "text".into(),
            "span 0 0 4 bold".into(),
            "- not a bullet here".into(),
            "".into(),
        ];
        let formats = DocFormats::new(lines.len());
        let text = serialize(&lnote_path(), &lines, &formats);
        let (lines2, formats2) = parse(&lnote_path(), &text);
        assert_eq!(lines2, lines);
        assert!(formats2.get(0).attrs_at(0).is_default());
        assert!(!formats2.get(2).para.bullet, "no '- ' trick inside .lnote");
    }

    /// A corrupt .lnote header falls back to plain text instead of erroring.
    #[test]
    fn corrupt_lnote_falls_back_to_plain() {
        let (lines, _) = parse(&lnote_path(), "not an lnote\njust text");
        assert_eq!(lines, vec!["not an lnote", "just text"]);
    }

    /// Unknown records and tokens are ignored (forward compatibility), and
    /// out-of-range / mid-char offsets are skipped, not fatal.
    #[test]
    fn lnote_parser_is_lenient() {
        let content = "lnote 2\n\
            comment future record type\n\
            para 0 bullet sparkle=yes\n\
            para 99 bullet\n\
            span 0 1 2 bold\n\
            span 0 50 60 bold\n\
            span 99 0 1 bold\n\
            text\n\
            héllo";
        let (lines, formats) = parse(&lnote_path(), content);
        assert_eq!(lines, vec!["héllo"]);
        assert!(formats.get(0).para.bullet);
        // "span 0 1 2" lands mid-é (é = 2 bytes at offset 1) — skipped.
        assert!(formats.get(0).attrs_at(1).is_default());
    }

    /// Plain .txt keeps the markdown bullet trick, exact bytes.
    #[test]
    fn plain_text_bullets_round_trip() {
        let path = PathBuf::from("notes.txt");
        let (lines, formats) = parse(&path, "title\n- first\n-\n- second");
        assert_eq!(lines, vec!["title", "first", "-", "second"]);
        assert!(!formats.get(0).para.bullet);
        assert!(formats.get(1).para.bullet);
        assert!(!formats.get(2).para.bullet, "lone dash is plain text");
        assert!(formats.get(3).para.bullet);
        assert_eq!(serialize(&path, &lines, &formats), "title\n- first\n-\n- second");
    }

    /// End-to-end through the editor's real file I/O.
    #[test]
    fn editor_save_load_lnote_on_disk() {
        let path = std::env::temp_dir().join("lntrn_notepad_rich_roundtrip.lnote");
        let mut e = crate::editor::Editor::new();
        e.lines = vec!["hello world".to_string()];
        e.formats = DocFormats::new(1);
        e.formats.get_mut(0).apply_format(6, 11, |a| a.bold = true);
        e.formats.get_mut(0).para.bullet = true;
        e.file_path = Some(path.clone());
        e.save_file().unwrap();

        let mut e2 = crate::editor::Editor::new();
        e2.load_file(path.clone()).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(e2.lines, vec!["hello world"]);
        assert!(e2.formats.get(0).attrs_at(7).bold);
        assert!(!e2.formats.get(0).attrs_at(2).bold);
        assert!(e2.formats.get(0).para.bullet);
    }
}
