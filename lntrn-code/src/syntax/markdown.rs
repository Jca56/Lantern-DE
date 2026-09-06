//! Markdown, line by line: headings, quotes, rules, list markers, fenced
//! code, and inline code, emphasis and links.

use super::{Language, LexState, Token, TokenKind, langs, lexer};

pub fn lex_line(line: &str, state: LexState, out: &mut Vec<Token>) -> LexState {
    out.clear();
    let n = line.len() as u32;
    let trimmed = line.trim_start();
    let indent = (line.len() - trimmed.len()) as u32;
    let push = |out: &mut Vec<Token>, start: u32, end: u32, kind: TokenKind| {
        if end > start {
            out.push(Token { start, end, kind });
        }
    };
    let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
    if let LexState::Fenced { lang, inner } = state {
        if fence {
            push(out, 0, n, TokenKind::Code);
            return LexState::Normal;
        }
        if lang == Language::Plain || lang == Language::Markdown {
            push(out, 0, n, TokenKind::Code);
            return state;
        }
        // The fence's language colors its lines, carrying its own state.
        let after = lexer::lex_line(langs::spec(lang), line, inner.into(), out);
        return LexState::Fenced { lang, inner: after.into() };
    }
    if fence {
        push(out, 0, n, TokenKind::Code);
        let lang = Language::from_fence(&trimmed[3..]);
        return LexState::Fenced { lang, inner: super::Inner::Normal };
    }
    if trimmed.starts_with('#') {
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if hashes <= 6 && trimmed[hashes..].starts_with(' ') || trimmed.len() == hashes {
            push(out, 0, n, TokenKind::Heading);
            return LexState::Normal;
        }
    }
    if trimmed.starts_with('>') {
        push(out, 0, n, TokenKind::Comment);
        return LexState::Normal;
    }
    if is_rule(trimmed) {
        push(out, 0, n, TokenKind::Punct);
        return LexState::Normal;
    }
    let mut body = indent as usize;
    if let Some(m) = list_marker(trimmed) {
        push(out, indent, indent + m as u32, TokenKind::Keyword);
        body += m;
    }
    inline(line, body, out);
    LexState::Normal
}

/// `---`, `***`, `___` (three or more, spaces allowed).
fn is_rule(t: &str) -> bool {
    let c = t.chars().next();
    matches!(c, Some('-' | '*' | '_')) && t.chars().filter(|&x| x != ' ').all(|x| Some(x) == c) && t.chars().filter(|&x| x != ' ').count() >= 3
}

/// Length of a `- `, `* `, `+ `, `1. ` or `- [ ] ` marker.
fn list_marker(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    let mut i = if matches!(b.first(), Some(b'-' | b'*' | b'+')) && b.get(1) == Some(&b' ') {
        2
    } else {
        let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 || !matches!(b.get(digits), Some(b'.' | b')')) || b.get(digits + 1) != Some(&b' ') {
            return None;
        }
        digits + 2
    };
    if t[i..].starts_with("[ ] ") || t[i..].starts_with("[x] ") || t[i..].starts_with("[X] ") {
        i += 4;
    }
    Some(i)
}

fn inline(line: &str, from: usize, out: &mut Vec<Token>) {
    let b = line.as_bytes();
    let n = b.len();
    let mut i = from;
    let push = |out: &mut Vec<Token>, s: usize, e: usize, k: TokenKind| out.push(Token { start: s as u32, end: e as u32, kind: k });
    while i < n {
        let c = b[i];
        match c {
            b'\\' => i += 2,
            b'`' => {
                let run = b[i..].iter().take_while(|&&x| x == b'`').count();
                let open = &line[i..i + run];
                match line[i + run..].find(open) {
                    Some(k) => {
                        push(out, i, i + run + k + run, TokenKind::Code);
                        i += run + k + run;
                    }
                    None => i += run,
                }
            }
            b'*' | b'_' => {
                let double = b.get(i + 1) == Some(&c);
                let len = if double { 2 } else { 1 };
                let marker = &line[i..i + len];
                let after = b.get(i + len).copied();
                if after.is_none_or(|a| a == b' ') {
                    i += len;
                    continue;
                }
                match line[i + len..].find(marker) {
                    Some(k) if k > 0 => {
                        push(out, i, i + len + k + len, TokenKind::Emphasis);
                        i += len + k + len;
                    }
                    _ => i += len,
                }
            }
            b'[' => {
                let rest = &line[i..];
                if let Some(close) = rest.find("](")
                    && let Some(end) = rest[close + 2..].find(')')
                {
                    push(out, i, i + close + 2 + end + 1, TokenKind::Link);
                    i += close + 2 + end + 1;
                } else {
                    i += 1;
                }
            }
            b'<' => {
                let rest = &line[i..];
                if (rest.starts_with("<http://") || rest.starts_with("<https://")) && let Some(k) = rest.find('>') {
                    push(out, i, i + k + 1, TokenKind::Link);
                    i += k + 1;
                } else {
                    i += 1;
                }
            }
            b'h' if line[i..].starts_with("http://") || line[i..].starts_with("https://") => {
                let k = line[i..].find(|ch: char| ch.is_whitespace() || ch == ')' || ch == '>').unwrap_or(n - i);
                push(out, i, i + k, TokenKind::Link);
                i += k;
            }
            _ => i += line[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(String, TokenKind)> {
        let mut out = Vec::new();
        lex_line(line, LexState::Normal, &mut out);
        out.iter().map(|t| (line[t.start as usize..t.end as usize].to_owned(), t.kind)).collect()
    }

    #[test]
    fn blocks_and_inline() {
        assert_eq!(kinds("## Title")[0].1, TokenKind::Heading);
        assert!(kinds("#hashtag").is_empty());
        assert_eq!(kinds("> quote")[0].1, TokenKind::Comment);
        assert_eq!(kinds("---")[0].1, TokenKind::Punct);
        let k = kinds("- [ ] do `x` and **bold** or *it* see [here](http://a) https://b.c/d");
        assert_eq!(k[0], ("- [ ] ".to_owned(), TokenKind::Keyword));
        assert!(k.contains(&("`x`".to_owned(), TokenKind::Code)));
        assert!(k.contains(&("**bold**".to_owned(), TokenKind::Emphasis)));
        assert!(k.contains(&("*it*".to_owned(), TokenKind::Emphasis)));
        assert!(k.contains(&("[here](http://a)".to_owned(), TokenKind::Link)));
        assert!(k.contains(&("https://b.c/d".to_owned(), TokenKind::Link)));
        assert_eq!(kinds("12. item")[0].0, "12. ");
        assert!(kinds("a * b * c").is_empty(), "spaced stars are not emphasis");
        let mut out = Vec::new();
        let s = lex_line("```rust", LexState::Normal, &mut out);
        assert!(matches!(s, LexState::Fenced { lang: Language::Rust, .. }));
        let s = lex_line("let x = 1;", s, &mut out);
        assert!(out.iter().any(|t| t.kind == TokenKind::Keyword), "rust inside the fence is rust");
        assert!(out.iter().any(|t| t.kind == TokenKind::Number));
        let s = lex_line("```", s, &mut out);
        assert_eq!(s, LexState::Normal);
        let s = lex_line("```", LexState::Normal, &mut out);
        let s = lex_line("anything", s, &mut out);
        assert_eq!(out[0].kind, TokenKind::Code, "a fence with no language is code");
        assert!(matches!(s, LexState::Fenced { lang: Language::Plain, .. }));
    }
}
