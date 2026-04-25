//! Lightweight syntax highlighting for Rust and Python. No external crates,
//! no semantic analysis — just a hand-rolled lexer that classifies each
//! token in a single line of source.
//!
//! Block comments (`/* */`) and triple-quoted strings (`"""..."""`) are
//! recognized only on the line they begin on. Multi-line spanning will be
//! added later if it bugs anyone.

use lntrn_render::{Color, FontStyle, FontWeight, TextRenderer};

use crate::theme::Theme;

/// Source language. `None` means render as plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    None,
    Rust,
    Python,
}

/// Token classification used for color lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Macro,
    Lifetime,
    Decorator,
    /// `True`/`False`/`None` (Python) or `true`/`false` (Rust). Most editors
    /// give booleans/null a distinct color from control-flow keywords.
    Boolean,
}

/// A single classified slice of a line, in byte offsets.
#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// Detect the source language from a filename.
pub fn detect_from_filename(filename: &str) -> Language {
    let lower = filename.to_ascii_lowercase();
    let ext = std::path::Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "rs" => Language::Rust,
        "py" | "pyi" | "pyx" => Language::Python,
        _ => Language::None,
    }
}

/// Tokenize a single line of source. Returns tokens in source order; gaps
/// (whitespace, punctuation) are not emitted — the renderer just paints those
/// regions in the default text color.
pub fn tokenize_line(text: &str, lang: Language) -> Vec<Token> {
    match lang {
        Language::Rust => tokenize_rust(text),
        Language::Python => tokenize_python(text),
        Language::None => Vec::new(),
    }
}

// ── Rust ─────────────────────────────────────────────────────────────────────

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "box", "yield",
];

const RUST_PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
    "i128", "isize", "f32", "f64", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
];

fn tokenize_rust(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];

        // Line comment // ...
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            let start = i;
            i = bytes.len();
            tokens.push(Token { start, end: i, kind: TokenKind::Comment });
            continue;
        }
        // Block comment /* ... */ (single-line MVP — eats to */ or EOL)
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            if i < bytes.len() && tokens_last_close(bytes, i) {
                // already advanced
            } else if i + 1 == bytes.len() {
                i = bytes.len();
            }
            tokens.push(Token { start, end: i, kind: TokenKind::Comment });
            continue;
        }

        // String literal "..."  (handles \" \\ escapes)
        if c == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(Token { start, end: i, kind: TokenKind::String });
            continue;
        }

        // Raw string r"..." or r#"..."#
        if c == b'r' && i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let start = i;
            i += 1;
            let mut hashes = 0;
            while i < bytes.len() && bytes[i] == b'#' {
                hashes += 1;
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        let mut close_hashes = 0;
                        let mut j = i + 1;
                        while j < bytes.len() && bytes[j] == b'#' && close_hashes < hashes {
                            close_hashes += 1;
                            j += 1;
                        }
                        if close_hashes == hashes {
                            i = j;
                            break;
                        }
                    }
                    i += 1;
                }
                tokens.push(Token { start, end: i, kind: TokenKind::String });
                continue;
            }
            // Wasn't actually a raw string — fall through with i reset.
            i = start;
        }

        // Char literal or lifetime: '
        if c == b'\'' {
            let start = i;
            // Look-ahead: char if `'<X>'`, lifetime if `'<ident>` without closing
            // quote. Walk one char and see what follows.
            let mut j = i + 1;
            if j < bytes.len() && bytes[j] == b'\\' && j + 1 < bytes.len() {
                j += 2; // escaped char
                if j < bytes.len() && bytes[j] == b'\'' {
                    j += 1;
                    tokens.push(Token { start, end: j, kind: TokenKind::String });
                    i = j;
                    continue;
                }
            }
            if j < bytes.len() {
                let next = bytes[j];
                let is_ident_start = next.is_ascii_alphabetic() || next == b'_';
                if is_ident_start {
                    // Could be lifetime or 'a' (single char).
                    let id_start = j;
                    while j < bytes.len()
                        && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
                    {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'\'' && j == id_start + 1 {
                        // 'a' - single char literal
                        j += 1;
                        tokens.push(Token { start, end: j, kind: TokenKind::String });
                    } else {
                        tokens.push(Token { start, end: j, kind: TokenKind::Lifetime });
                    }
                    i = j;
                    continue;
                }
                // Treat as a char literal of one byte
                j += 1;
                if j < bytes.len() && bytes[j] == b'\'' {
                    j += 1;
                }
                tokens.push(Token { start, end: j, kind: TokenKind::String });
                i = j;
                continue;
            }
            i += 1;
            continue;
        }

        // Number
        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            tokens.push(Token { start, end: i, kind: TokenKind::Number });
            continue;
        }

        // Identifier / keyword / type / function / macro
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            // macro_name!(...)
            if i < bytes.len() && bytes[i] == b'!' {
                let end = i + 1;
                tokens.push(Token { start, end, kind: TokenKind::Macro });
                i = end;
                continue;
            }
            let kind = if matches!(word, "true" | "false") {
                TokenKind::Boolean
            } else if RUST_KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else if RUST_PRIMITIVES.contains(&word)
                || word.chars().next().map_or(false, |c| c.is_uppercase())
            {
                TokenKind::Type
            } else if i < bytes.len() && bytes[i] == b'(' {
                TokenKind::Function
            } else {
                // Identifiers without highlighting fall through to default text color
                i = start + word.len();
                continue;
            };
            tokens.push(Token { start, end: i, kind });
            continue;
        }

        // Skip everything else (whitespace / punctuation)
        i += 1;
    }

    tokens
}

#[inline]
fn tokens_last_close(bytes: &[u8], i: usize) -> bool {
    i >= 2 && bytes[i - 2] == b'*' && bytes[i - 1] == b'/'
}

// ── Python ───────────────────────────────────────────────────────────────────

const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case",
];

const PYTHON_BUILTINS: &[&str] = &[
    "abs", "all", "any", "bin", "bool", "bytes", "callable", "chr", "dict", "dir", "enumerate",
    "eval", "exec", "filter", "float", "format", "frozenset", "getattr", "globals", "hasattr",
    "hash", "help", "hex", "id", "input", "int", "isinstance", "issubclass", "iter", "len",
    "list", "map", "max", "min", "next", "object", "oct", "open", "ord", "pow", "print", "range",
    "repr", "reversed", "round", "set", "setattr", "slice", "sorted", "str", "sum", "super",
    "tuple", "type", "vars", "zip", "self", "cls",
];

fn tokenize_python(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];

        // Comment
        if c == b'#' {
            let start = i;
            i = bytes.len();
            tokens.push(Token { start, end: i, kind: TokenKind::Comment });
            continue;
        }

        // String literal — handles f/r/b/rb/br prefixes and triple quotes
        if c == b'"' || c == b'\'' {
            let start = i;
            i = consume_python_string(bytes, i);
            tokens.push(Token { start, end: i, kind: TokenKind::String });
            continue;
        }
        if (c == b'f' || c == b'F' || c == b'r' || c == b'R' || c == b'b' || c == b'B')
            && i + 1 < bytes.len()
        {
            let after_prefix = if (bytes[i + 1] == b'r' || bytes[i + 1] == b'R'
                || bytes[i + 1] == b'b' || bytes[i + 1] == b'B'
                || bytes[i + 1] == b'f' || bytes[i + 1] == b'F')
                && i + 2 < bytes.len()
            {
                i + 2
            } else {
                i + 1
            };
            if bytes[after_prefix] == b'"' || bytes[after_prefix] == b'\'' {
                let start = i;
                i = consume_python_string(bytes, after_prefix);
                tokens.push(Token { start, end: i, kind: TokenKind::String });
                continue;
            }
        }

        // Decorator
        if c == b'@' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(Token { start, end: i, kind: TokenKind::Decorator });
            continue;
        }

        // Number
        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            tokens.push(Token { start, end: i, kind: TokenKind::Number });
            continue;
        }

        // Identifier / keyword / builtin / function / class
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            let kind = if matches!(word, "True" | "False" | "None") {
                TokenKind::Boolean
            } else if PYTHON_KEYWORDS.contains(&word) {
                TokenKind::Keyword
            } else if PYTHON_BUILTINS.contains(&word) {
                TokenKind::Type
            } else if word.chars().next().map_or(false, |c| c.is_uppercase()) {
                TokenKind::Type
            } else if i < bytes.len() && bytes[i] == b'(' {
                TokenKind::Function
            } else {
                continue;
            };
            tokens.push(Token { start, end: i, kind });
            continue;
        }

        i += 1;
    }

    tokens
}

/// Consume a python string starting at `start` (which must point at a quote
/// character). Handles triple quotes (single line only — stops at EOL if no
/// closing triple is found) and \\-escapes.
fn consume_python_string(bytes: &[u8], start: usize) -> usize {
    let q = bytes[start];
    let mut i = start + 1;
    // Triple quote?
    let triple = i + 1 < bytes.len() && bytes[i] == q && bytes[i + 1] == q;
    if triple {
        i += 2;
        while i + 2 < bytes.len() {
            if bytes[i] == q && bytes[i + 1] == q && bytes[i + 2] == q {
                return i + 3;
            }
            i += 1;
        }
        return bytes.len();
    }
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == q {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

// ── Drawing helper ──────────────────────────────────────────────────────────

/// Draw `line[start..end]` at `(start_x, y)`, breaking the chunk at syntax
/// token boundaries so each portion is colored according to `theme`. Returns
/// the total width drawn. If `tokens` is empty (no language detected) the
/// chunk renders as one piece in `default_color`.
#[allow(clippy::too_many_arguments)]
pub fn draw_chunk_with_syntax(
    text: &mut TextRenderer,
    line: &str,
    start: usize,
    end: usize,
    tokens: &[Token],
    default_color: Color,
    theme: Theme,
    fs: f32,
    start_x: f32,
    y: f32,
    weight: FontWeight,
    style: FontStyle,
    max_w: f32,
    sw: u32,
    sh: u32,
) -> f32 {
    if start >= end {
        return 0.0;
    }
    if tokens.is_empty() {
        let chunk = &line[start..end];
        text.queue_styled(chunk, fs, start_x, y, default_color, max_w, weight, style, sw, sh);
        return text.measure_width_styled(chunk, fs, weight, style);
    }
    let mut x = start_x;
    let mut pos = start;
    for token in tokens {
        if token.end <= pos {
            continue;
        }
        if token.start >= end {
            break;
        }
        let tk_start = token.start.max(pos);
        let tk_end = token.end.min(end);
        if pos < tk_start {
            let plain = &line[pos..tk_start];
            text.queue_styled(plain, fs, x, y, default_color, max_w, weight, style, sw, sh);
            x += text.measure_width_styled(plain, fs, weight, style);
        }
        let tk_text = &line[tk_start..tk_end];
        let color = theme.syntax_color(token.kind);
        text.queue_styled(tk_text, fs, x, y, color, max_w, weight, style, sw, sh);
        x += text.measure_width_styled(tk_text, fs, weight, style);
        pos = tk_end;
    }
    if pos < end {
        let plain = &line[pos..end];
        text.queue_styled(plain, fs, x, y, default_color, max_w, weight, style, sw, sh);
        x += text.measure_width_styled(plain, fs, weight, style);
    }
    x - start_x
}
