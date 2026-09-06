//! Syntax highlighting: a language is a table of keywords and delimiters
//! ([`langs`]) fed to one generic line lexer ([`lexer`]); Markdown has its
//! own. Lines are lexed on demand from the last one known good, each
//! carrying the state (inside a block comment, a long string) into the
//! next, so a document costs only what is on screen.

pub mod langs;
mod lexer;
mod markdown;

use std::path::Path;

use crate::buffer::Buffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Text,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    Punct,
    Operator,
    Attribute,
    Constant,
    Heading,
    Emphasis,
    Code,
    Link,
}

/// A colored span of a line, in byte offsets. Gaps are plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub kind: TokenKind,
}

/// Which long string a line is inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StrDelim {
    #[default]
    Double,
    Single,
    Backtick,
    TripleDouble,
    TripleSingle,
    /// A Rust raw string closed by `"` and this many `#`.
    Raw(u8),
}

/// The state of the language inside a Markdown code fence, between lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Inner {
    #[default]
    Normal,
    Comment(u8),
    Str(StrDelim),
}

impl From<LexState> for Inner {
    fn from(s: LexState) -> Self {
        match s {
            LexState::Comment { depth } => Inner::Comment(depth),
            LexState::Str { delim } => Inner::Str(delim),
            _ => Inner::Normal,
        }
    }
}

impl From<Inner> for LexState {
    fn from(i: Inner) -> Self {
        match i {
            Inner::Normal => LexState::Normal,
            Inner::Comment(depth) => LexState::Comment { depth },
            Inner::Str(delim) => LexState::Str { delim },
        }
    }
}

/// What a line's end leaves for the next line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LexState {
    #[default]
    Normal,
    Comment {
        depth: u8,
    },
    Str {
        delim: StrDelim,
    },
    /// Inside a Markdown code fence, lexed as `lang` (`Plain`: no colors).
    Fenced {
        lang: Language,
        inner: Inner,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Plain,
    Rust,
    Toml,
    Markdown,
    Json,
    Python,
    JavaScript,
    C,
    Shell,
    Yaml,
}

impl Language {
    pub const ALL: [Language; 10] = [Language::Plain, Language::Rust, Language::Toml, Language::Markdown, Language::Json, Language::Python, Language::JavaScript, Language::C, Language::Shell, Language::Yaml];

    /// By file name and extension, then by a shebang line.
    pub fn detect(path: &Path, first_line: &str) -> Language {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        match name {
            "Makefile" | "makefile" | "GNUmakefile" | "Dockerfile" | ".gitignore" | ".env" | ".bashrc" | ".zshrc" | ".profile" | "PKGBUILD" => return Language::Shell,
            "Cargo.lock" => return Language::Toml,
            _ => {}
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        let by_ext = match ext.as_str() {
            "rs" => Language::Rust,
            "toml" => Language::Toml,
            "md" | "markdown" | "mdx" => Language::Markdown,
            "json" | "jsonc" | "json5" => Language::Json,
            "py" | "pyi" | "pyw" => Language::Python,
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => Language::JavaScript,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "cu" | "wgsl" | "glsl" | "frag" | "vert" | "m" | "mm" => Language::C,
            "sh" | "bash" | "zsh" | "fish" | "ebuild" | "eclass" => Language::Shell,
            "yaml" | "yml" => Language::Yaml,
            _ => Language::Plain,
        };
        if by_ext != Language::Plain || !first_line.starts_with("#!") {
            return by_ext;
        }
        let l = first_line.to_ascii_lowercase();
        if l.contains("python") {
            Language::Python
        } else if l.contains("node") || l.contains("deno") || l.contains("bun") {
            Language::JavaScript
        } else if l.contains("sh") {
            Language::Shell
        } else {
            Language::Plain
        }
    }

    /// The language a code fence's info string names (```rust, ```py …).
    pub fn from_fence(info: &str) -> Language {
        let word: String = info.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '#').collect::<String>().to_ascii_lowercase();
        match word.as_str() {
            "rust" | "rs" => Language::Rust,
            "toml" => Language::Toml,
            "json" | "jsonc" | "json5" => Language::Json,
            "python" | "py" | "python3" => Language::Python,
            "js" | "javascript" | "ts" | "typescript" | "jsx" | "tsx" => Language::JavaScript,
            "c" | "cpp" | "c++" | "h" | "hpp" | "cxx" | "wgsl" | "glsl" | "objc" => Language::C,
            "sh" | "bash" | "zsh" | "shell" | "console" | "fish" | "ebuild" => Language::Shell,
            "yaml" | "yml" => Language::Yaml,
            _ => Language::Plain,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Plain => "Plain Text",
            Language::Rust => "Rust",
            Language::Toml => "TOML",
            Language::Markdown => "Markdown",
            Language::Json => "JSON",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::C => "C / C++",
            Language::Shell => "Shell",
            Language::Yaml => "YAML",
        }
    }

    /// What starts a line comment, for Toggle Comment.
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::Rust | Language::JavaScript | Language::C => Some("//"),
            Language::Python | Language::Shell | Language::Toml | Language::Yaml => Some("#"),
            Language::Plain | Language::Markdown | Language::Json => None,
        }
    }
}

/// Lex one line of `lang` starting in `state`; returns the state at its end.
pub fn lex_line(lang: Language, line: &str, state: LexState, out: &mut Vec<Token>) -> LexState {
    match lang {
        Language::Plain => {
            out.clear();
            LexState::Normal
        }
        Language::Markdown => markdown::lex_line(line, state, out),
        other => lexer::lex_line(langs::spec(other), line, state, out),
    }
}

#[derive(Clone, Default)]
struct LineInfo {
    tokens: Vec<Token>,
    end: LexState,
}

/// The per-line token cache of one document.
pub struct Highlighter {
    lang: Language,
    lines: Vec<LineInfo>,
    /// Lines before this index are up to date.
    valid: usize,
}

impl Highlighter {
    pub fn new(lang: Language) -> Self {
        Self { lang, lines: Vec::new(), valid: 0 }
    }

    pub fn lang(&self) -> Language {
        self.lang
    }

    pub fn set_lang(&mut self, lang: Language) {
        self.lang = lang;
        self.valid = 0;
    }

    /// Line `line` changed (and maybe everything after it).
    pub fn invalidate_from(&mut self, line: usize) {
        self.valid = self.valid.min(line);
    }

    /// Bring lines up to and including `up_to` up to date.
    pub fn ensure(&mut self, buffer: &Buffer, up_to: usize) {
        let n = buffer.line_count();
        if self.lines.len() != n {
            self.lines.resize_with(n, LineInfo::default);
            self.valid = self.valid.min(n);
        }
        let up_to = up_to.min(n - 1);
        if up_to < self.valid {
            return;
        }
        let mut state = if self.valid == 0 { LexState::Normal } else { self.lines[self.valid - 1].end };
        let mut tokens = Vec::new();
        for i in self.valid..=up_to {
            state = lex_line(self.lang, buffer.line(i), state, &mut tokens);
            let li = &mut self.lines[i];
            std::mem::swap(&mut li.tokens, &mut tokens);
            li.end = state;
        }
        self.valid = up_to + 1;
    }

    /// The tokens of a line brought up to date by [`Self::ensure`].
    pub fn tokens(&self, line: usize) -> &[Token] {
        if line < self.valid { self.lines.get(line).map_or(&[], |l| l.tokens.as_slice()) } else { &[] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages() {
        assert_eq!(Language::detect(Path::new("a/b.rs"), ""), Language::Rust);
        assert_eq!(Language::detect(Path::new("Cargo.lock"), ""), Language::Toml);
        assert_eq!(Language::detect(Path::new("run"), "#!/usr/bin/env python3"), Language::Python);
        assert_eq!(Language::detect(Path::new("run"), "#!/bin/bash"), Language::Shell);
        assert_eq!(Language::detect(Path::new("x.unknown"), "text"), Language::Plain);
        assert_eq!(Language::detect(Path::new("Makefile"), ""), Language::Shell);
    }

    #[test]
    fn cache_follows_edits_and_block_state() {
        let mut b = Buffer::from_text("/* open\nstill\n*/ fn x() {}\nlet y = 1;");
        let mut h = Highlighter::new(Language::Rust);
        h.ensure(&b, 3);
        assert_eq!(h.tokens(1).len(), 1);
        assert_eq!(h.tokens(1)[0].kind, TokenKind::Comment);
        assert!(h.tokens(2).iter().any(|t| t.kind == TokenKind::Keyword), "fn after the comment closes");
        // Delete the opener: the whole file is code again.
        b.replace(crate::buffer::Range::new(crate::buffer::Pos::new(0, 0), crate::buffer::Pos::new(0, 2)), "");
        h.invalidate_from(0);
        h.ensure(&b, 3);
        assert!(h.tokens(1).is_empty(), "plain identifier, no token");
        assert_eq!(h.tokens(3).iter().filter(|t| t.kind == TokenKind::Number).count(), 1);
        // Lines are lexed only as far as asked.
        let mut h2 = Highlighter::new(Language::Rust);
        h2.ensure(&b, 1);
        assert!(h2.tokens(3).is_empty());
        h2.ensure(&b, 3);
        assert!(!h2.tokens(3).is_empty());
    }
}
