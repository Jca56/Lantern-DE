//! Lightweight syntax highlighter for fenced code blocks. Handles the
//! languages most likely to appear in chat replies: rust, python, js/ts,
//! shell/bash, json, toml, html, c/cpp. Falls back to plain text for
//! anything unknown.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    Plain,
    Keyword,
    Type,
    Builtin,
    String,
    Number,
    Comment,
    Punct,
}

#[derive(Debug, Clone)]
pub struct Tok {
    pub kind: TokKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Shell,
    Json,
    Toml,
    Html,
    C,
    Plain,
}

pub fn lang_from_tag(tag: &str) -> Lang {
    match tag.to_ascii_lowercase().as_str() {
        "rust" | "rs" => Lang::Rust,
        "python" | "py" => Lang::Python,
        "js" | "javascript" => Lang::JavaScript,
        "ts" | "typescript" | "tsx" => Lang::TypeScript,
        "sh" | "bash" | "zsh" | "shell" | "console" => Lang::Shell,
        "json" => Lang::Json,
        "toml" => Lang::Toml,
        "html" | "xml" | "svg" => Lang::Html,
        "c" | "cpp" | "c++" | "h" | "hpp" => Lang::C,
        _ => Lang::Plain,
    }
}

pub fn tokenize(src: &str, lang: Lang) -> Vec<Tok> {
    match lang {
        Lang::Rust => tokenize_curly(src, &RUST_KW, &RUST_TY, "//", Some(("/*", "*/")), true),
        Lang::Python => tokenize_curly(src, &PY_KW, &PY_BUILTIN, "#", None, false),
        Lang::JavaScript | Lang::TypeScript =>
            tokenize_curly(src, &JS_KW, &JS_BUILTIN, "//", Some(("/*", "*/")), true),
        Lang::Shell => tokenize_curly(src, &SH_KW, &[], "#", None, false),
        Lang::Json => tokenize_json(src),
        Lang::Toml => tokenize_toml(src),
        Lang::Html => tokenize_html(src),
        Lang::C => tokenize_curly(src, &C_KW, &C_TY, "//", Some(("/*", "*/")), true),
        Lang::Plain => vec![Tok { kind: TokKind::Plain, text: src.into() }],
    }
}

fn tokenize_curly(
    src: &str,
    keywords: &[&str],
    types: &[&str],
    line_comment: &str,
    block_comment: Option<(&str, &str)>,
    rust_lifetimes: bool,
) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        // line comment
        if src[i..].starts_with(line_comment) {
            let end = src[i..].find('\n').map(|n| i + n).unwrap_or(bytes.len());
            out.push(Tok { kind: TokKind::Comment, text: src[i..end].to_string() });
            i = end;
            continue;
        }
        // block comment
        if let Some((open, close)) = block_comment {
            if src[i..].starts_with(open) {
                let after = i + open.len();
                let end = src[after..].find(close)
                    .map(|n| after + n + close.len())
                    .unwrap_or(bytes.len());
                out.push(Tok { kind: TokKind::Comment, text: src[i..end].to_string() });
                i = end;
                continue;
            }
        }
        // string literals
        if b == b'"' || b == b'\'' {
            let quote = b;
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() { j += 2; continue; }
                if bytes[j] == quote { j += 1; break; }
                j += 1;
            }
            out.push(Tok { kind: TokKind::String, text: src[i..j].to_string() });
            i = j;
            continue;
        }
        // number literal
        if b.is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_') {
                j += 1;
            }
            out.push(Tok { kind: TokKind::Number, text: src[i..j].to_string() });
            i = j;
            continue;
        }
        // rust lifetime: 'a — guard against 'a' char literal (already handled above as string).
        if rust_lifetimes && b == b'\'' {
            // unreachable in practice; handled by string branch
        }
        // identifier
        if b.is_ascii_alphabetic() || b == b'_' {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let ident = &src[i..j];
            let kind = if keywords.contains(&ident) {
                TokKind::Keyword
            } else if types.contains(&ident) {
                TokKind::Type
            } else if ident.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
                && ident.len() > 1
            {
                TokKind::Type
            } else {
                TokKind::Plain
            };
            out.push(Tok { kind, text: ident.to_string() });
            i = j;
            continue;
        }
        // punctuation cluster — split each, runs of whitespace stay plain.
        if b.is_ascii_whitespace() {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            out.push(Tok { kind: TokKind::Plain, text: src[i..j].to_string() });
            i = j;
            continue;
        }
        out.push(Tok { kind: TokKind::Punct, text: src[i..i + 1].to_string() });
        i += 1;
    }
    out
}

fn tokenize_json(src: &str) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() { j += 2; continue; }
                if bytes[j] == b'"' { j += 1; break; }
                j += 1;
            }
            out.push(Tok { kind: TokKind::String, text: src[i..j].to_string() });
            i = j;
            continue;
        }
        if b.is_ascii_digit() || b == b'-' && i + 1 < bytes.len() && bytes[i+1].is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.' || bytes[j] == b'e' || bytes[j] == b'E' || bytes[j] == b'+' || bytes[j] == b'-') {
                j += 1;
            }
            out.push(Tok { kind: TokKind::Number, text: src[i..j].to_string() });
            i = j;
            continue;
        }
        if b.is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_alphabetic() { j += 1; }
            let id = &src[i..j];
            let kind = if matches!(id, "true" | "false" | "null") {
                TokKind::Keyword
            } else { TokKind::Plain };
            out.push(Tok { kind, text: id.into() });
            i = j;
            continue;
        }
        out.push(Tok { kind: TokKind::Punct, text: src[i..i+1].into() });
        i += 1;
    }
    out
}

fn tokenize_toml(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            out.push(Tok { kind: TokKind::Comment, text: line.into() });
            continue;
        }
        if trimmed.starts_with('[') {
            out.push(Tok { kind: TokKind::Type, text: line.into() });
            continue;
        }
        if let Some(eq) = line.find('=') {
            out.push(Tok { kind: TokKind::Keyword, text: line[..eq].into() });
            out.push(Tok { kind: TokKind::Punct, text: "=".into() });
            let rest = &line[eq + 1..];
            // basic string / number / plain split
            out.push(Tok {
                kind: if rest.trim_start().starts_with('"') { TokKind::String } else { TokKind::Plain },
                text: rest.into(),
            });
        } else {
            out.push(Tok { kind: TokKind::Plain, text: line.into() });
        }
    }
    out
}

fn tokenize_html(src: &str) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = src[i..].find('>').map(|n| i + n + 1).unwrap_or(bytes.len());
            out.push(Tok { kind: TokKind::Type, text: src[i..end].into() });
            i = end;
            continue;
        }
        let next = src[i..].find('<').map(|n| i + n).unwrap_or(bytes.len());
        out.push(Tok { kind: TokKind::Plain, text: src[i..next].into() });
        i = next;
    }
    out
}

// ── Keyword tables ──────────────────────────────────────────────────────────

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn",
    "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];
const RUST_TY: &[&str] = &[
    "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
    "str", "String", "Vec", "Option", "Result", "Box", "Rc", "Arc", "HashMap", "BTreeMap",
];

const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "False", "finally", "for", "from",
    "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not",
    "or", "pass", "raise", "return", "True", "try", "while", "with", "yield",
];
const PY_BUILTIN: &[&str] = &[
    "print", "len", "range", "list", "dict", "tuple", "set", "str", "int",
    "float", "bool", "bytes", "type", "isinstance", "open", "self",
];

const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue",
    "default", "delete", "do", "else", "export", "extends", "false", "finally",
    "for", "function", "if", "import", "in", "instanceof", "let", "new", "null",
    "of", "return", "super", "switch", "this", "throw", "true", "try", "typeof",
    "undefined", "var", "void", "while", "yield",
    "interface", "type", "enum", "namespace", "implements", "readonly",
];
const JS_BUILTIN: &[&str] = &[
    "console", "window", "document", "Math", "JSON", "Promise", "Array",
    "Object", "Number", "String", "Boolean",
];

const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "do",
    "done", "in", "function", "return", "exit", "export", "local", "echo",
];

const C_KW: &[&str] = &[
    "auto", "break", "case", "const", "continue", "default", "do", "else",
    "enum", "extern", "for", "goto", "if", "inline", "register", "restrict",
    "return", "sizeof", "static", "struct", "switch", "typedef", "union",
    "volatile", "while", "class", "namespace", "template", "public",
    "private", "protected", "this", "new", "delete", "operator", "virtual",
    "override", "nullptr", "true", "false",
];
const C_TY: &[&str] = &[
    "void", "int", "char", "short", "long", "float", "double", "signed",
    "unsigned", "bool", "size_t", "ssize_t", "uint8_t", "uint16_t",
    "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t",
];
