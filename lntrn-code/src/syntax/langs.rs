//! What makes each language: its word lists and delimiters, as tables the
//! generic lexer reads.

use super::{Language, StrDelim};

/// A string opener and how far it may run.
pub struct StrSpec {
    pub open: &'static str,
    pub delim: StrDelim,
    /// An unclosed string carries into the next line.
    pub multiline: bool,
}

const fn s(open: &'static str, delim: StrDelim, multiline: bool) -> StrSpec {
    StrSpec { open, delim, multiline }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Attr {
    None,
    /// `#[...]` and `#![...]`.
    RustHash,
    /// `@name` decorators.
    Decorator,
    /// `#directive` at the start of a line.
    Preprocessor,
    /// `$name`, `${...}`, `$@`.
    ShellVar,
}

pub struct Spec {
    pub keywords: &'static [&'static str],
    pub types: &'static [&'static str],
    pub constants: &'static [&'static str],
    /// Colored as functions wherever they appear.
    pub builtins: &'static [&'static str],
    /// The identifier after one of these is a function name.
    pub fn_keywords: &'static [&'static str],
    pub line_comment: &'static [&'static str],
    pub block_comment: Option<(&'static str, &'static str)>,
    pub nested_comments: bool,
    /// Openers tried in order (longer ones first).
    pub strings: &'static [StrSpec],
    /// Letters that may precede a string (`b`, `r`, `f`), lower-case.
    pub string_prefixes: &'static [u8],
    /// `r#"..."#`.
    pub raw_hash_strings: bool,
    /// Backslash escapes inside `'...'`.
    pub single_escapes: bool,
    /// `'x'` is a character, not a string.
    pub char_literal: bool,
    /// `'a` is a lifetime.
    pub lifetimes: bool,
    pub attribute: Attr,
    /// `name!` is a macro.
    pub macros: bool,
    /// A capitalised identifier is a type.
    pub type_case: bool,
    /// An identifier ending in `_t` is a type.
    pub type_suffix_t: bool,
    /// An identifier before `(` is a function.
    pub calls: bool,
    /// A string before `:` is a key.
    pub json_keys: bool,
    /// A line-leading identifier before `:` is a key.
    pub key_colon: bool,
    /// `[table]` lines.
    pub toml_tables: bool,
}

const BASE: Spec = Spec {
    keywords: &[],
    types: &[],
    constants: &[],
    builtins: &[],
    fn_keywords: &[],
    line_comment: &[],
    block_comment: None,
    nested_comments: false,
    strings: &[],
    string_prefixes: &[],
    raw_hash_strings: false,
    single_escapes: true,
    char_literal: false,
    lifetimes: false,
    attribute: Attr::None,
    macros: false,
    type_case: false,
    type_suffix_t: false,
    calls: false,
    json_keys: false,
    key_colon: false,
    toml_tables: false,
};

static RUST: Spec = Spec {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "union", "macro_rules", "yield",
    ],
    types: &["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char", "str"],
    constants: &["self", "true", "false", "None", "Some", "Ok", "Err"],
    fn_keywords: &["fn"],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    nested_comments: true,
    strings: &[s("\"", StrDelim::Double, true)],
    string_prefixes: b"brc",
    raw_hash_strings: true,
    char_literal: true,
    lifetimes: true,
    attribute: Attr::RustHash,
    macros: true,
    type_case: true,
    calls: true,
    ..BASE
};

static PYTHON: Spec = Spec {
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match", "case",
    ],
    types: &["int", "float", "str", "bytes", "bool", "list", "dict", "set", "tuple", "object", "complex", "frozenset", "bytearray"],
    constants: &["None", "True", "False", "self", "cls", "Ellipsis", "NotImplemented"],
    builtins: &[
        "print", "len", "range", "open", "enumerate", "zip", "map", "filter", "sorted", "reversed", "isinstance", "type", "super", "min", "max", "sum", "abs", "any", "all", "input", "repr", "hash", "id", "iter", "next", "getattr", "setattr", "hasattr", "round", "divmod", "format", "vars", "dir", "exec", "eval",
    ],
    fn_keywords: &["def"],
    line_comment: &["#"],
    strings: &[s("\"\"\"", StrDelim::TripleDouble, true), s("'''", StrDelim::TripleSingle, true), s("\"", StrDelim::Double, false), s("'", StrDelim::Single, false)],
    string_prefixes: b"rbfu",
    attribute: Attr::Decorator,
    type_case: true,
    calls: true,
    ..BASE
};

static JAVASCRIPT: Spec = Spec {
    keywords: &[
        "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "let", "new", "of", "return", "static", "super", "switch", "throw", "try", "typeof", "var", "void", "while", "with", "yield", "get", "set", "as", "interface", "type", "enum", "implements", "private", "public", "protected", "readonly", "declare", "namespace", "abstract", "satisfies", "keyof",
    ],
    types: &["string", "number", "boolean", "any", "void", "never", "unknown", "object", "symbol", "bigint"],
    constants: &["true", "false", "null", "undefined", "NaN", "Infinity", "this"],
    builtins: &["console", "require", "module", "exports", "document", "window", "process", "globalThis"],
    fn_keywords: &["function"],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[s("`", StrDelim::Backtick, true), s("\"", StrDelim::Double, false), s("'", StrDelim::Single, false)],
    attribute: Attr::Decorator,
    type_case: true,
    calls: true,
    ..BASE
};

static C: Spec = Spec {
    keywords: &[
        "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto", "if", "inline", "register", "restrict", "return", "sizeof", "static", "struct", "switch", "typedef", "union", "volatile", "while", "class", "namespace", "template", "typename", "using", "new", "delete", "this", "public", "private", "protected", "virtual", "override", "final", "try", "catch", "throw", "constexpr", "consteval", "operator", "explicit", "friend", "mutable", "noexcept", "static_assert", "alignas", "alignof", "decltype", "fn", "var", "let", "uniform", "storage", "workgroup", "in", "out", "layout",
    ],
    types: &[
        "int", "char", "short", "long", "float", "double", "void", "unsigned", "signed", "bool", "wchar_t", "size_t", "ssize_t", "ptrdiff_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t", "uintptr_t", "intptr_t", "FILE", "vec2", "vec3", "vec4", "mat3", "mat4", "f32", "f16", "u32", "i32", "vec2f", "vec3f", "vec4f", "vec2u", "vec3u", "vec4u", "mat4x4f", "mat3x3f", "sampler", "texture_2d",
    ],
    constants: &["true", "false", "NULL", "nullptr"],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[s("\"", StrDelim::Double, false)],
    char_literal: true,
    attribute: Attr::Preprocessor,
    type_case: true,
    type_suffix_t: true,
    calls: true,
    ..BASE
};

static SHELL: Spec = Spec {
    keywords: &["if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "select", "time", "coproc", "end", "switch", "begin"],
    builtins: &[
        "echo", "cd", "export", "local", "return", "exit", "source", "alias", "unalias", "set", "unset", "shift", "read", "printf", "test", "true", "false", "eval", "exec", "trap", "wait", "kill", "jobs", "fg", "bg", "pushd", "popd", "let", "declare", "typeset", "readonly", "getopts", "sudo", "doas", "emerge", "pacman", "cargo", "git", "make", "rm", "cp", "mv", "ls", "cat", "grep", "sed", "awk", "find", "xargs", "mkdir", "chmod", "chown", "curl", "wget", "tar", "ssh",
    ],
    line_comment: &["#"],
    strings: &[s("\"", StrDelim::Double, true), s("'", StrDelim::Single, true)],
    single_escapes: false,
    attribute: Attr::ShellVar,
    ..BASE
};

static TOML: Spec = Spec {
    constants: &["true", "false", "inf", "nan"],
    line_comment: &["#"],
    strings: &[s("\"\"\"", StrDelim::TripleDouble, true), s("'''", StrDelim::TripleSingle, true), s("\"", StrDelim::Double, false), s("'", StrDelim::Single, false)],
    single_escapes: false,
    toml_tables: true,
    ..BASE
};

static JSON: Spec = Spec {
    constants: &["true", "false", "null"],
    line_comment: &["//"],
    block_comment: Some(("/*", "*/")),
    strings: &[s("\"", StrDelim::Double, false)],
    json_keys: true,
    ..BASE
};

static YAML: Spec = Spec {
    constants: &["true", "false", "null", "yes", "no", "on", "off", "~"],
    line_comment: &["#"],
    strings: &[s("\"", StrDelim::Double, false), s("'", StrDelim::Single, false)],
    single_escapes: false,
    key_colon: true,
    ..BASE
};

/// The table of a language. Plain text and Markdown have none.
pub fn spec(lang: Language) -> &'static Spec {
    match lang {
        Language::Rust => &RUST,
        Language::Python => &PYTHON,
        Language::JavaScript => &JAVASCRIPT,
        Language::C => &C,
        Language::Shell => &SHELL,
        Language::Toml => &TOML,
        Language::Json => &JSON,
        Language::Yaml => &YAML,
        Language::Plain | Language::Markdown => &BASE,
    }
}
