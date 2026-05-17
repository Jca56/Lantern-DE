//! In-house markdown parser. Block-level: paragraphs, headings (#..######),
//! fenced code blocks (```lang…```), bullet lists, numbered lists, hr (---),
//! blockquotes. Inline: **bold**, *italic*, `inline code`. Not a CommonMark
//! conformer — just enough for assistant replies to render nicely.

#[derive(Debug, Clone)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading { level: u8, inlines: Vec<Inline> },
    Code { lang: String, body: String },
    Bullet(Vec<Vec<Inline>>),
    Numbered(Vec<Vec<Inline>>),
    Quote(Vec<Inline>),
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
}

impl Inline {
    pub fn as_text(&self) -> &str {
        match self {
            Inline::Text(s) | Inline::Bold(s) | Inline::Italic(s) | Inline::Code(s) => s,
        }
    }
}

pub fn parse(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim().to_string();
            let mut body = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            // skip closing fence (if present)
            if i < lines.len() { i += 1; }
            // strip trailing newline
            if body.ends_with('\n') { body.pop(); }
            out.push(Block::Code { lang, body });
            continue;
        }

        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        if let Some(level) = heading_level(trimmed) {
            let rest = trimmed[level as usize..].trim_start();
            out.push(Block::Heading { level, inlines: parse_inlines(rest) });
            i += 1;
            continue;
        }

        if is_rule(trimmed) {
            out.push(Block::Rule);
            i += 1;
            continue;
        }

        if let Some(rest) = strip_quote(trimmed) {
            let mut buf = String::from(rest);
            i += 1;
            while i < lines.len() {
                if let Some(r) = strip_quote(lines[i].trim_start()) {
                    buf.push(' ');
                    buf.push_str(r);
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Block::Quote(parse_inlines(&buf)));
            continue;
        }

        if let Some(item) = bullet_body(trimmed) {
            let mut items = vec![parse_inlines(item)];
            i += 1;
            while i < lines.len() {
                if let Some(body) = bullet_body(lines[i].trim_start()) {
                    items.push(parse_inlines(body));
                    i += 1;
                } else { break; }
            }
            out.push(Block::Bullet(items));
            continue;
        }

        if let Some(item) = numbered_body(trimmed) {
            let mut items = vec![parse_inlines(item)];
            i += 1;
            while i < lines.len() {
                if let Some(body) = numbered_body(lines[i].trim_start()) {
                    items.push(parse_inlines(body));
                    i += 1;
                } else { break; }
            }
            out.push(Block::Numbered(items));
            continue;
        }

        // Paragraph: collect until blank line / block boundary.
        let mut buf = String::from(line);
        i += 1;
        while i < lines.len() {
            let next = lines[i];
            let nt = next.trim_start();
            if nt.is_empty() || nt.starts_with("```") || heading_level(nt).is_some()
                || is_rule(nt) || bullet_body(nt).is_some()
                || numbered_body(nt).is_some() || strip_quote(nt).is_some()
            {
                break;
            }
            buf.push(' ');
            buf.push_str(nt);
            i += 1;
        }
        out.push(Block::Paragraph(parse_inlines(&buf)));
    }
    out
}

fn heading_level(s: &str) -> Option<u8> {
    let mut n = 0u8;
    for c in s.chars() {
        if c == '#' { n += 1; } else { break; }
    }
    if n >= 1 && n <= 6 && s.chars().nth(n as usize) == Some(' ') {
        Some(n)
    } else { None }
}

fn is_rule(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty()
        && t.chars().all(|c| c == '-' || c == '*' || c == '_')
        && t.len() >= 3
}

fn strip_quote(s: &str) -> Option<&str> {
    s.strip_prefix("> ").or_else(|| if s == ">" { Some("") } else { None })
}

fn bullet_body(s: &str) -> Option<&str> {
    s.strip_prefix("- ").or_else(|| s.strip_prefix("* ")).or_else(|| s.strip_prefix("+ "))
}

fn numbered_body(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let mut digits = 0;
    let mut end = 0;
    for (i, c) in chars.by_ref() {
        if c.is_ascii_digit() { digits += 1; end = i + 1; } else { break; }
    }
    if digits == 0 { return None; }
    let rest = &s[end..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

pub fn parse_inlines(src: &str) -> Vec<Inline> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'`' {
            flush(&mut out, &mut buf);
            // find closing backtick
            let start = i + 1;
            if let Some(end) = src[start..].find('`') {
                out.push(Inline::Code(src[start..start + end].to_string()));
                i = start + end + 1;
                continue;
            }
        }
        if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            flush(&mut out, &mut buf);
            let start = i + 2;
            if let Some(end) = src[start..].find("**") {
                out.push(Inline::Bold(src[start..start + end].to_string()));
                i = start + end + 2;
                continue;
            }
        }
        if b == b'*' {
            flush(&mut out, &mut buf);
            let start = i + 1;
            if let Some(end) = src[start..].find('*') {
                out.push(Inline::Italic(src[start..start + end].to_string()));
                i = start + end + 1;
                continue;
            }
        }
        if b == b'_' && i + 1 < bytes.len() && bytes[i + 1] == b'_' {
            flush(&mut out, &mut buf);
            let start = i + 2;
            if let Some(end) = src[start..].find("__") {
                out.push(Inline::Bold(src[start..start + end].to_string()));
                i = start + end + 2;
                continue;
            }
        }
        // append one char
        let ch = src[i..].chars().next().unwrap();
        buf.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut out, &mut buf);
    out
}

fn flush(out: &mut Vec<Inline>, buf: &mut String) {
    if !buf.is_empty() {
        out.push(Inline::Text(std::mem::take(buf)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: a code-fenced message used to infinite-loop the parser
    /// because the inner while-loop never advanced `i`. OOM kill.
    #[test]
    fn fenced_code_block_parses_in_finite_time() {
        let src = "intro\n\n```bash\necho hi\nexit 0\n```\n\noutro";
        let blocks = parse(src);
        assert_eq!(blocks.len(), 3);
        match &blocks[1] {
            Block::Code { lang, body } => {
                assert_eq!(lang, "bash");
                assert!(body.contains("echo hi"));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }
}
