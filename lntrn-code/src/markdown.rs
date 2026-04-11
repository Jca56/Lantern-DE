//! Lightweight Markdown → preview transformation. Hand-rolled inline parser
//! that builds a styled `Editor` snapshot from raw Markdown source.
//!
//! Supported (v1):
//! - Headings `#` … `######` (1–6, larger font + bold)
//! - Bold `**text**` and `__text__`
//! - Italic `*text*` and `_text_`
//! - Strikethrough `~~text~~`
//! - Inline code `` `code` ``
//! - Fenced code blocks ```` ```lang ... ``` ````
//! - Bullet lists `- item`, `* item`, `+ item` → bulleted with •
//! - Numbered lists `1. item` → preserved as-is
//!
//! Block comments / nested formatting are kept simple — first match wins.

use crate::editor::Editor;
use crate::format::DocFormats;

/// Packed RGB code text color — bright yellow/gold against the dark chip.
const CODE_COLOR: u32 = 0xFFC800;
/// Packed RGB chip background — dark grey, slightly lighter than pure black
/// so it reads as a chip rather than a void.
const CODE_BG: u32 = 0x3A3A3A;
/// Heading sizes (h1..h6).
const H_SIZES: [f32; 6] = [38.0, 32.0, 28.0, 24.0, 22.0, 20.0];

/// Quick rolling hash of `lines` so previews can detect changes cheaply.
pub fn hash_lines(lines: &[String]) -> u64 {
    let mut h: u64 = 1469598103934665603; // FNV-1a offset
    h = h.wrapping_add(lines.len() as u64);
    for line in lines {
        for &b in line.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        h ^= 0x0a;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Walk every tab and re-sync any preview tabs from their source. Cheap if
/// nothing has changed (just a hash compare per preview).
pub fn sync_all_previews(tabs: &mut [Editor]) {
    let mut updates: Vec<(usize, Vec<String>)> = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        if let Some(src_id) = tab.preview_of {
            if let Some(src) = tabs.iter().find(|t| t.tab_id == src_id) {
                updates.push((i, src.lines.clone()));
            }
        }
    }
    for (i, source_lines) in updates {
        sync_preview(&mut tabs[i], &source_lines);
    }
}

/// Sync the preview editor's content with `source_lines`. Replaces
/// `preview.lines` and `preview.formats` with the rendered representation.
pub fn sync_preview(preview: &mut Editor, source_lines: &[String]) {
    let hash = hash_lines(source_lines);
    if preview.preview_source_hash == hash && !preview.lines.is_empty() {
        return;
    }
    preview.preview_source_hash = hash;

    let mut out_lines: Vec<String> = Vec::with_capacity(source_lines.len());
    let mut out_formats = DocFormats::new(0);
    let mut in_code_block = false;

    for src_line in source_lines {
        // Detect a fenced code block boundary `\`\`\`...`.
        let trimmed = src_line.trim_start();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            // Render the fence line itself as plain code-colored text.
            out_lines.push(src_line.clone());
            out_formats = grow_formats(out_formats, out_lines.len());
            apply_code(&mut out_formats, out_lines.len() - 1, src_line.len());
            continue;
        }

        if in_code_block {
            out_lines.push(src_line.clone());
            out_formats = grow_formats(out_formats, out_lines.len());
            apply_code(&mut out_formats, out_lines.len() - 1, src_line.len());
            continue;
        }

        // Heading? `#`, `##`, … up to six.
        if let Some((level, body)) = parse_heading(src_line) {
            let line_idx = out_lines.len();
            out_lines.push(body.to_string());
            out_formats = grow_formats(out_formats, out_lines.len());
            let size = H_SIZES[(level - 1).min(5) as usize];
            out_formats.get_mut(line_idx).apply_format(0, body.len(), |a| {
                a.bold = true;
                a.font_size = Some(size);
            });
            continue;
        }

        // List bullet — replace marker with •.
        let (display_line, list_offset) = transform_list(src_line);
        let line_idx = out_lines.len();
        out_lines.push(display_line.clone());
        out_formats = grow_formats(out_formats, out_lines.len());

        // Inline parsing for bold / italic / strike / code.
        let inlines = parse_inline(&display_line, list_offset);
        for span in inlines {
            out_formats.get_mut(line_idx).apply_format(span.start, span.end, |a| {
                if span.bold {
                    a.bold = true;
                }
                if span.italic {
                    a.italic = true;
                }
                if span.strike {
                    a.strikethrough = true;
                }
                if span.code {
                    a.color = Some(CODE_COLOR);
                    a.bg_color = Some(CODE_BG);
                }
            });
        }
    }

    if out_lines.is_empty() {
        out_lines.push(String::new());
        out_formats = grow_formats(out_formats, 1);
    }

    preview.lines = out_lines;
    preview.formats = out_formats;
    preview.wrap_rows = vec![vec![0]; preview.lines.len()];
    preview.cursor_line = 0;
    preview.cursor_col = 0;
    preview.sel_anchor = None;
}

fn grow_formats(mut formats: DocFormats, target: usize) -> DocFormats {
    while formats.len() < target {
        formats.insert_line(formats.len(), Default::default());
    }
    formats
}

fn apply_code(formats: &mut DocFormats, line_idx: usize, len: usize) {
    if len == 0 {
        return;
    }
    formats.get_mut(line_idx).apply_format(0, len, |a| {
        a.color = Some(CODE_COLOR);
        a.bg_color = Some(CODE_BG);
    });
}

/// Returns `(level, body_text)` if the line begins with one to six `#` chars
/// followed by whitespace.
fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b'#' && i < 6 {
        i += 1;
    }
    if i == 0 || i > 6 || i >= bytes.len() {
        return None;
    }
    if bytes[i] != b' ' {
        return None;
    }
    Some((i as u8, &line[i + 1..]))
}

/// Detect bullet/numbered list markers and return the rendered display line +
/// the byte offset where inline parsing should begin (so list markers don't
/// get treated as italic).
fn transform_list(line: &str) -> (String, usize) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();

    // Bullet: -, *, +
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        let mut s = String::with_capacity(line.len() + 2);
        for _ in 0..indent {
            s.push(' ');
        }
        s.push_str("•  ");
        let body_start = s.len();
        s.push_str(rest);
        return (s, body_start);
    }

    // Numbered: digits followed by ". "
    let mut digits = 0;
    while digits < trimmed.len() && trimmed.as_bytes()[digits].is_ascii_digit() {
        digits += 1;
    }
    if digits > 0 && trimmed[digits..].starts_with(". ") {
        return (line.to_string(), indent + digits + 2);
    }

    (line.to_string(), 0)
}

#[derive(Clone, Copy, Default)]
struct InlineSpan {
    start: usize,
    end: usize,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

/// Parse inline markdown markers in `text` starting from `body_start`.
/// Returns spans where formatting should be applied. The markers are NOT
/// stripped from `text` — for v1 we just style the marker chars too. This
/// keeps offset bookkeeping trivial.
fn parse_inline(text: &str, body_start: usize) -> Vec<InlineSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = body_start;
    while i < bytes.len() {
        // Inline code `…`
        if bytes[i] == b'`' {
            if let Some(end) = find_marker(bytes, i + 1, b'`', 1) {
                spans.push(InlineSpan {
                    start: i,
                    end: end + 1,
                    code: true,
                    ..Default::default()
                });
                i = end + 1;
                continue;
            }
        }
        // Bold **…** (must check before single *)
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(end) = find_double(bytes, i + 2, b'*') {
                spans.push(InlineSpan {
                    start: i,
                    end: end + 2,
                    bold: true,
                    ..Default::default()
                });
                i = end + 2;
                continue;
            }
        }
        // Italic *…*
        if bytes[i] == b'*' {
            if let Some(end) = find_marker(bytes, i + 1, b'*', 1) {
                spans.push(InlineSpan {
                    start: i,
                    end: end + 1,
                    italic: true,
                    ..Default::default()
                });
                i = end + 1;
                continue;
            }
        }
        // Strike ~~…~~
        if i + 1 < bytes.len() && bytes[i] == b'~' && bytes[i + 1] == b'~' {
            if let Some(end) = find_double(bytes, i + 2, b'~') {
                spans.push(InlineSpan {
                    start: i,
                    end: end + 2,
                    strike: true,
                    ..Default::default()
                });
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    spans
}

fn find_marker(bytes: &[u8], from: usize, marker: u8, _len: usize) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_double(bytes: &[u8], from: usize, marker: u8) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == marker && bytes[i + 1] == marker {
            return Some(i);
        }
        i += 1;
    }
    None
}
