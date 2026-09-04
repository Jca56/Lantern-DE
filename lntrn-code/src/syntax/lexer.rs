//! The generic line lexer: comments, strings (with prefixes, raw and
//! triple-quoted forms, character literals), numbers, identifiers sorted
//! by the language's word lists, attributes, and punctuation. One pass per
//! line, byte-wise; delimiters are ASCII so stepping through the bytes of
//! a multi-byte character never matches one.

use super::langs::{Attr, Spec};
use super::{LexState, StrDelim, Token, TokenKind};

pub fn lex_line(spec: &Spec, line: &str, state: LexState, out: &mut Vec<Token>) -> LexState {
    out.clear();
    let mut lx = Lexer { spec, line, b: line.as_bytes(), out, prev_word: None };
    let mut i = 0;
    let mut state = state;
    while i < lx.b.len() {
        match state {
            LexState::Comment { depth } => {
                let (end, depth) = lx.scan_comment(i, depth);
                lx.push(i, end, TokenKind::Comment);
                i = end;
                state = if depth == 0 { LexState::Normal } else { LexState::Comment { depth } };
            }
            LexState::Str { delim } => {
                let (end, closed) = lx.scan_string(i, delim, true);
                lx.push(i, end, TokenKind::String);
                i = end;
                if closed {
                    state = LexState::Normal;
                }
            }
            LexState::Normal => {
                let (end, next) = lx.token(i);
                i = end.max(i + 1);
                state = next;
            }
        }
    }
    state
}

struct Lexer<'a> {
    spec: &'a Spec,
    line: &'a str,
    b: &'a [u8],
    out: &'a mut Vec<Token>,
    /// The last identifier or keyword, for `fn name`.
    prev_word: Option<&'a str>,
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80
}

impl<'a> Lexer<'a> {
    fn push(&mut self, start: usize, end: usize, kind: TokenKind) {
        if end > start {
            self.out.push(Token { start: start as u32, end: end as u32, kind });
        }
    }

    fn starts(&self, i: usize, s: &str) -> bool {
        self.b[i..].starts_with(s.as_bytes())
    }

    /// The character at byte `i` and its length.
    fn char_at(&self, i: usize) -> (char, usize) {
        let c = self.line[i..].chars().next().unwrap_or('\0');
        (c, c.len_utf8().max(1))
    }

    /// Past the end of an identifier starting at `i`.
    fn ident_end(&self, mut i: usize) -> usize {
        while i < self.b.len() {
            let c = self.b[i];
            if c < 0x80 {
                if !is_ident_byte(c) {
                    break;
                }
                i += 1;
            } else {
                let (ch, len) = self.char_at(i);
                if !ch.is_alphanumeric() {
                    break;
                }
                i += len;
            }
        }
        i
    }

    /// Inside a block comment from `i` with `depth` opens outstanding:
    /// where it ends on this line (or the line's end) and the depth then.
    fn scan_comment(&self, mut i: usize, mut depth: u8) -> (usize, u8) {
        let n = self.b.len();
        let Some((open, close)) = self.spec.block_comment else {
            return (n, 0);
        };
        while i < n {
            if self.spec.nested_comments && self.starts(i, open) {
                depth = depth.saturating_add(1);
                i += open.len();
            } else if self.starts(i, close) {
                depth -= 1;
                i += close.len();
                if depth == 0 {
                    return (i, 0);
                }
            } else {
                i += 1;
            }
        }
        (n, depth)
    }

    /// Inside a string from `i`: where it ends (after its close) and
    /// whether it closed on this line.
    fn scan_string(&self, mut i: usize, delim: StrDelim, escapes_allowed: bool) -> (usize, bool) {
        let n = self.b.len();
        let mut raw_close = [b'"'; 9];
        let (close, escapes): (&[u8], bool) = match delim {
            StrDelim::Double => (b"\"", true),
            StrDelim::Single => (b"'", self.spec.single_escapes),
            StrDelim::Backtick => (b"`", true),
            StrDelim::TripleDouble => (b"\"\"\"", true),
            StrDelim::TripleSingle => (b"'''", true),
            StrDelim::Raw(h) => {
                let h = (h as usize).min(8);
                raw_close[1..=h].fill(b'#');
                (&raw_close[..=h], false)
            }
            StrDelim::Fence => return (n, false),
        };
        let escapes = escapes && escapes_allowed;
        while i < n {
            if escapes && self.b[i] == b'\\' {
                i = (i + 2).min(n);
                continue;
            }
            if self.b[i..].starts_with(close) {
                return (i + close.len(), true);
            }
            i += 1;
        }
        (n, false)
    }

    /// One token from `i` in the normal state: `(end, state after)`.
    fn token(&mut self, i: usize) -> (usize, LexState) {
        let n = self.b.len();
        let c = self.b[i];
        let normal = LexState::Normal;
        if c.is_ascii_whitespace() {
            return (i + 1, normal);
        }
        // Comments.
        for p in self.spec.line_comment {
            let shell_hash_in_var = c == b'#' && i > 0 && self.b[i - 1] == b'$';
            if self.starts(i, p) && !shell_hash_in_var {
                self.push(i, n, TokenKind::Comment);
                return (n, normal);
            }
        }
        if let Some((open, _)) = self.spec.block_comment
            && self.starts(i, open)
        {
            let (end, depth) = self.scan_comment(i + open.len(), 1);
            self.push(i, end, TokenKind::Comment);
            return (end, if depth == 0 { normal } else { LexState::Comment { depth } });
        }
        // Attributes, decorators, directives, shell variables.
        if let Some(end) = self.attribute(i) {
            return (end, normal);
        }
        // TOML table headers.
        if self.spec.toml_tables && c == b'[' && self.line[..i].trim().is_empty() {
            let end = self.line[i..].rfind(']').map_or(n, |k| i + k + 1);
            self.push(i, end, TokenKind::Type);
            return (end, normal);
        }
        // Character literals and lifetimes.
        if self.spec.char_literal && c == b'\'' {
            return (self.quote(i), normal);
        }
        // Strings, with their prefixes.
        if let Some(r) = self.string_start(i) {
            return r;
        }
        // Numbers.
        if c.is_ascii_digit() || (c == b'.' && self.b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let end = self.number_end(i);
            self.push(i, end, TokenKind::Number);
            return (end, normal);
        }
        // Identifiers and keywords.
        if c == b'_' || c.is_ascii_alphabetic() || (c >= 0x80 && self.char_at(i).0.is_alphabetic()) {
            return (self.word(i), normal);
        }
        // Punctuation and operators.
        if b"()[]{},;.".contains(&c) {
            self.push(i, i + 1, TokenKind::Punct);
            return (i + 1, normal);
        }
        if b"+-*/%=<>!&|^~?:@#$\\".contains(&c) {
            self.push(i, i + 1, TokenKind::Operator);
            return (i + 1, normal);
        }
        // Anything else (a symbol outside ASCII): step over it.
        (i + self.char_at(i).1, normal)
    }

    fn attribute(&mut self, i: usize) -> Option<usize> {
        let n = self.b.len();
        let c = self.b[i];
        match self.spec.attribute {
            Attr::None => None,
            Attr::RustHash if c == b'#' && (self.starts(i, "#[") || self.starts(i, "#![")) => {
                let mut depth = 0i32;
                let mut j = i;
                while j < n {
                    match self.b[j] {
                        b'[' => depth += 1,
                        b']' => {
                            depth -= 1;
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                self.push(i, j, TokenKind::Attribute);
                Some(j)
            }
            Attr::Decorator if c == b'@' && self.b.get(i + 1).is_some_and(|&d| is_ident_byte(d)) => {
                let mut j = self.ident_end(i + 1);
                while self.b.get(j) == Some(&b'.') && self.b.get(j + 1).is_some_and(|&d| is_ident_byte(d)) {
                    j = self.ident_end(j + 1);
                }
                self.push(i, j, TokenKind::Attribute);
                Some(j)
            }
            Attr::Preprocessor if c == b'#' && self.line[..i].trim().is_empty() => {
                let mut j = i + 1;
                while j < n && self.b[j] == b' ' {
                    j += 1;
                }
                let word_end = self.ident_end(j);
                self.push(i, word_end, TokenKind::Attribute);
                let directive = &self.line[j..word_end];
                if matches!(directive, "include" | "import" | "error" | "warning") {
                    let mut k = word_end;
                    while k < n && self.b[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    self.push(k, n, TokenKind::String);
                    return Some(n);
                }
                Some(word_end)
            }
            Attr::ShellVar if c == b'$' => {
                let next = *self.b.get(i + 1)?;
                let end = if next == b'{' {
                    self.line[i..].find('}').map_or(n, |k| i + k + 1)
                } else if is_ident_byte(next) {
                    self.ident_end(i + 1)
                } else if b"@*#?$!-".contains(&next) {
                    i + 2
                } else {
                    return None;
                };
                self.push(i, end, TokenKind::Attribute);
                Some(end)
            }
            _ => None,
        }
    }

    /// A `'` in a language with character literals: the literal, or a
    /// Rust lifetime, or a lone quote.
    fn quote(&mut self, i: usize) -> usize {
        let n = self.b.len();
        let rest = &self.line[i + 1..];
        // '\n', '\u{1F600}', '\'' : an escape then the closing quote.
        if let Some(after) = rest.strip_prefix('\\') {
            if let Some(k) = after.find('\'').filter(|&k| k < 12) {
                let end = i + 1 + 1 + k + 1;
                self.push(i, end, TokenKind::String);
                return end;
            }
        } else if let Some(ch) = rest.chars().next()
            && ch != '\''
            && rest[ch.len_utf8()..].starts_with('\'')
        {
            let end = i + 1 + ch.len_utf8() + 1;
            self.push(i, end, TokenKind::String);
            return end;
        }
        if !self.spec.lifetimes {
            // C: an unterminated or multi-character literal runs to the next quote.
            let end = rest.find('\'').map_or(n, |k| i + 1 + k + 1);
            self.push(i, end, TokenKind::String);
            return end;
        }
        if self.b.get(i + 1).is_some_and(|&d| is_ident_byte(d)) {
            let end = self.ident_end(i + 1);
            self.push(i, end, TokenKind::Type);
            return end;
        }
        self.push(i, i + 1, TokenKind::Operator);
        i + 1
    }

    /// A string starting at `i` (after up to two prefix letters), if one does.
    fn string_start(&mut self, i: usize) -> Option<(usize, LexState)> {
        let n = self.b.len();
        let mut j = i;
        let mut raw = false;
        while j < n && j - i < 2 && self.b[j].is_ascii_alphabetic() && self.spec.string_prefixes.contains(&self.b[j].to_ascii_lowercase()) {
            raw |= self.b[j].eq_ignore_ascii_case(&b'r');
            j += 1;
        }
        if raw && self.spec.raw_hash_strings {
            let hashes = self.b[j..].iter().take_while(|&&d| d == b'#').count();
            if hashes <= 8 && self.b.get(j + hashes) == Some(&b'"') {
                let delim = StrDelim::Raw(hashes as u8);
                let (end, closed) = self.scan_string(j + hashes + 1, delim, false);
                self.push(i, end, TokenKind::String);
                return Some((end, if closed { LexState::Normal } else { LexState::Str { delim } }));
            }
        }
        for sp in self.spec.strings {
            if self.starts(j, sp.open) {
                let (end, closed) = self.scan_string(j + sp.open.len(), sp.delim, !raw);
                self.push(i, end, TokenKind::String);
                if self.spec.json_keys && closed {
                    let mut k = end;
                    while k < n && self.b[k] == b' ' {
                        k += 1;
                    }
                    if self.b.get(k) == Some(&b':')
                        && let Some(t) = self.out.last_mut()
                    {
                        t.kind = TokenKind::Attribute;
                    }
                }
                let state = if closed || !sp.multiline { LexState::Normal } else { LexState::Str { delim: sp.delim } };
                return Some((end, state));
            }
        }
        None
    }

    fn number_end(&self, i: usize) -> usize {
        let n = self.b.len();
        let b = self.b;
        let mut j = i;
        let radix = if self.starts(i, "0x") || self.starts(i, "0X") {
            16
        } else if self.starts(i, "0b") || self.starts(i, "0B") {
            2
        } else if self.starts(i, "0o") || self.starts(i, "0O") {
            8
        } else {
            10
        };
        if radix != 10 {
            j += 2;
            while j < n && ((b[j] as char).is_digit(radix) || b[j] == b'_') {
                j += 1;
            }
        } else {
            while j < n && (b[j].is_ascii_digit() || b[j] == b'_') {
                j += 1;
            }
            if j < n && b[j] == b'.' && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
                j += 1;
                while j < n && (b[j].is_ascii_digit() || b[j] == b'_') {
                    j += 1;
                }
            }
            if j < n && (b[j] == b'e' || b[j] == b'E') {
                let mut k = j + 1;
                if k < n && (b[k] == b'+' || b[k] == b'-') {
                    k += 1;
                }
                if k < n && b[k].is_ascii_digit() {
                    j = k;
                    while j < n && b[j].is_ascii_digit() {
                        j += 1;
                    }
                }
            }
        }
        // Type suffixes: u32, f64, L, n.
        while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
        j
    }

    /// An identifier from `i`: classified by the word lists and what is
    /// around it.
    fn word(&mut self, i: usize) -> usize {
        let n = self.b.len();
        let mut end = self.ident_end(i);
        let word = &self.line[i..end];
        let spec = self.spec;
        let after = self.b[end..].iter().position(|c| *c != b' ').map(|k| self.b[end + k]);
        let kind = if spec.keywords.contains(&word) {
            TokenKind::Keyword
        } else if spec.constants.contains(&word) {
            TokenKind::Constant
        } else if spec.types.contains(&word) {
            TokenKind::Type
        } else if spec.builtins.contains(&word) || self.prev_word.is_some_and(|p| spec.fn_keywords.contains(&p)) {
            TokenKind::Function
        } else if spec.macros && self.b.get(end) == Some(&b'!') && self.b.get(end + 1) != Some(&b'=') {
            end += 1;
            TokenKind::Attribute
        } else if (spec.type_case && word.chars().next().is_some_and(char::is_uppercase)) || (spec.type_suffix_t && word.ends_with("_t")) {
            TokenKind::Type
        } else if spec.key_colon && after == Some(b':') && self.line[..i].trim().is_empty() {
            TokenKind::Attribute
        } else if spec.calls && after == Some(b'(') {
            TokenKind::Function
        } else {
            TokenKind::Text
        };
        if kind != TokenKind::Text {
            self.push(i, end.min(n), kind);
        }
        self.prev_word = Some(word);
        end
    }
}

#[cfg(test)]
mod tests {
    use super::super::langs::spec;
    use super::super::{Language, LexState, StrDelim, TokenKind};
    use super::*;

    fn kinds(lang: Language, line: &str) -> Vec<(String, TokenKind)> {
        let mut out = Vec::new();
        lex_line(spec(lang), line, LexState::Normal, &mut out);
        out.iter().map(|t| (line[t.start as usize..t.end as usize].to_owned(), t.kind)).collect()
    }

    #[test]
    fn rust_line() {
        let k = kinds(Language::Rust, r#"pub fn go<'a>(x: &'a Vec<u8>) -> Option<i32> { println!("hi {}", 0xFF_u8); 'c' } // done"#);
        let find = |s: &str| k.iter().find(|(t, _)| t == s).map(|(_, k)| *k);
        assert_eq!(find("pub"), Some(TokenKind::Keyword));
        assert_eq!(find("go"), Some(TokenKind::Function));
        assert_eq!(find("'a"), Some(TokenKind::Type));
        assert_eq!(find("Vec"), Some(TokenKind::Type));
        assert_eq!(find("u8"), Some(TokenKind::Type));
        assert_eq!(find("println!"), Some(TokenKind::Attribute));
        assert_eq!(find("\"hi {}\""), Some(TokenKind::String));
        assert_eq!(find("0xFF_u8"), Some(TokenKind::Number));
        assert_eq!(find("'c'"), Some(TokenKind::String));
        assert_eq!(find("// done"), Some(TokenKind::Comment));
        let k = kinds(Language::Rust, r##"let s = r#"raw "x" "#; #[derive(Debug)] Self::new()"##);
        assert_eq!(k[0].0, "let");
        assert!(k.iter().any(|(t, kind)| t == r##"r#"raw "x" "#"## && *kind == TokenKind::String));
        assert!(k.iter().any(|(t, kind)| t == "#[derive(Debug)]" && *kind == TokenKind::Attribute));
        assert!(k.iter().any(|(t, kind)| t == "Self" && *kind == TokenKind::Type));
    }

    #[test]
    fn multi_line_strings_and_comments_carry_state() {
        let mut out = Vec::new();
        let s = lex_line(spec(Language::Python), "x = \"\"\"start", LexState::Normal, &mut out);
        assert_eq!(s, LexState::Str { delim: StrDelim::TripleDouble });
        let s = lex_line(spec(Language::Python), "still\"\"\" + 1", s, &mut out);
        assert_eq!(s, LexState::Normal);
        assert_eq!(out[0].kind, TokenKind::String);
        assert_eq!(out.last().unwrap().kind, TokenKind::Number);
        let s = lex_line(spec(Language::Rust), "/* a /* nested */", LexState::Normal, &mut out);
        assert_eq!(s, LexState::Comment { depth: 1 });
        let s = lex_line(spec(Language::C), "int x; /* a /* not nested */ y", LexState::Normal, &mut out);
        assert_eq!(s, LexState::Normal);
        let s = lex_line(spec(Language::C), "char *s = \"unterminated", LexState::Normal, &mut out);
        assert_eq!(s, LexState::Normal, "C strings do not span lines");
        let s = lex_line(spec(Language::JavaScript), "const t = `multi", LexState::Normal, &mut out);
        assert_eq!(s, LexState::Str { delim: StrDelim::Backtick });
    }

    #[test]
    fn other_languages() {
        let k = kinds(Language::Python, "@dataclass\nclass Foo:");
        assert_eq!(k[0], ("@dataclass".to_owned(), TokenKind::Attribute));
        let k = kinds(Language::Python, "def run(self): return print(f\"x{1}\")");
        assert!(k.iter().any(|(t, kind)| t == "run" && *kind == TokenKind::Function));
        assert!(k.iter().any(|(t, kind)| t == "self" && *kind == TokenKind::Constant));
        assert!(k.iter().any(|(t, kind)| t == "f\"x{1}\"" && *kind == TokenKind::String));
        let k = kinds(Language::Json, r#"{"name": "x", "n": 3, "ok": true}"#);
        assert!(k.iter().any(|(t, kind)| t == "\"name\"" && *kind == TokenKind::Attribute));
        assert!(k.iter().any(|(t, kind)| t == "\"x\"" && *kind == TokenKind::String));
        assert!(k.iter().any(|(t, kind)| t == "true" && *kind == TokenKind::Constant));
        let k = kinds(Language::Shell, "echo \"$HOME\" ${x} $# # comment");
        assert_eq!(k[0], ("echo".to_owned(), TokenKind::Function));
        assert!(k.iter().any(|(t, kind)| t == "${x}" && *kind == TokenKind::Attribute));
        assert!(k.iter().any(|(t, kind)| t == "$#" && *kind == TokenKind::Attribute));
        assert!(k.iter().any(|(t, kind)| t == "# comment" && *kind == TokenKind::Comment));
        let k = kinds(Language::Toml, "[package.metadata]");
        assert_eq!(k[0].1, TokenKind::Type);
        let k = kinds(Language::C, "#include <stdio.h>");
        assert_eq!(k[0], ("#include".to_owned(), TokenKind::Attribute));
        assert_eq!(k[1], ("<stdio.h>".to_owned(), TokenKind::String));
        let k = kinds(Language::C, "size_t n = sizeof(uint8_t);");
        assert_eq!(k[0], ("size_t".to_owned(), TokenKind::Type));
        let k = kinds(Language::Yaml, "name: value # c");
        assert_eq!(k[0], ("name".to_owned(), TokenKind::Attribute));
        assert!(kinds(Language::Rust, "héllo wörld").is_empty(), "unicode identifiers are plain text");
    }
}
