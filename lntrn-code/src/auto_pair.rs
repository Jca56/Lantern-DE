//! Bracket / quote auto-pairing for code-ish files.
//!
//! Behaviors:
//! - Type `(`, `[`, `{` → inserts the open + close, cursor lands between
//! - Type `"`, `'`, `` ` `` → same, but only when not adjacent to a word char
//!   (so contractions like `don't` and identifiers like `let s = "x"` work)
//! - Type a closing char when the next char already matches → just advance
//!   the cursor (don't double up)
//! - Type an open char with text selected → wraps the selection with the pair
//! - Backspace inside an empty pair `(|)` → deletes both
//!
//! Only active when the current document looks like code, based on filename
//! extension. Plain `.txt` and `.md` files type literally.

use crate::editor::Editor;

/// Try to handle a typed character with auto-pair logic. Returns `true` if
/// the input was consumed (skip the normal `insert_char` call). Returns
/// `false` to fall through to default behavior.
pub fn handle_typed_char(ch: char, editor: &mut Editor) -> bool {
    if !is_code_file(&editor.filename) {
        return false;
    }

    // Wrap a selection with the pair when an opener is typed.
    if editor.has_selection() {
        if let Some((open, close)) = pair_for(ch) {
            wrap_selection(editor, open, close);
            return true;
        }
        return false;
    }

    // Skip a closing bracket if the next char already matches it.
    if is_closing(ch) {
        if next_char(editor) == Some(ch) {
            advance_cursor(editor);
            return true;
        }
        return false;
    }

    // Open + auto-close.
    if let Some((open, close)) = pair_for(ch) {
        if is_quote(ch) {
            let next = next_char(editor);
            // If we're typing the closing quote of an existing pair, skip it.
            if next == Some(ch) {
                advance_cursor(editor);
                return true;
            }
            // Don't auto-pair when adjacent to a word char — handles
            // contractions like "don't" and identifier suffixes.
            let prev = prev_char(editor);
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            if prev.map_or(false, is_word) || next.map_or(false, is_word) {
                return false;
            }
        }
        insert_pair(editor, open, close);
        return true;
    }

    false
}

/// Try to delete a matched pair on backspace. Returns true if a pair was
/// deleted. Otherwise returns false so the caller can run normal backspace.
pub fn handle_backspace(editor: &mut Editor) -> bool {
    if !is_code_file(&editor.filename) {
        return false;
    }
    if editor.has_selection() {
        return false;
    }
    let (Some(prev), Some(next)) = (prev_char(editor), next_char(editor)) else {
        return false;
    };
    if pair_for(prev) != Some((prev, next)) {
        return false;
    }
    // Cursor sits between an empty pair like `(|)` — delete both.
    editor.delete(); // removes the close char (forward delete)
    editor.backspace(); // removes the open char
    true
}

// ── Pair table ──────────────────────────────────────────────────────────────

fn pair_for(ch: char) -> Option<(char, char)> {
    match ch {
        '(' => Some(('(', ')')),
        '[' => Some(('[', ']')),
        '{' => Some(('{', '}')),
        '"' => Some(('"', '"')),
        '\'' => Some(('\'', '\'')),
        '`' => Some(('`', '`')),
        _ => None,
    }
}

fn is_closing(ch: char) -> bool {
    matches!(ch, ')' | ']' | '}')
}

fn is_quote(ch: char) -> bool {
    matches!(ch, '"' | '\'' | '`')
}

// ── Cursor helpers ──────────────────────────────────────────────────────────

fn prev_char(editor: &Editor) -> Option<char> {
    if editor.cursor_col == 0 {
        return None;
    }
    editor.lines[editor.cursor_line][..editor.cursor_col]
        .chars()
        .last()
}

fn next_char(editor: &Editor) -> Option<char> {
    let line = &editor.lines[editor.cursor_line];
    if editor.cursor_col >= line.len() {
        return None;
    }
    line[editor.cursor_col..].chars().next()
}

fn advance_cursor(editor: &mut Editor) {
    editor.move_right(false);
}

fn insert_pair(editor: &mut Editor, open: char, close: char) {
    editor.insert_char(open);
    editor.insert_char(close);
    editor.move_left(false);
}

fn wrap_selection(editor: &mut Editor, open: char, close: char) {
    let selected = editor.selected_text().unwrap_or_default();
    let wrapped = format!("{}{}{}", open, selected, close);
    editor.insert_str(&wrapped);
}

// ── File-type detection ─────────────────────────────────────────────────────

/// Return true if `filename` looks like a code or structured-data file
/// where bracket/quote pairing makes sense. Plain prose files (`.txt`,
/// `.md`, `.rst`) opt out so typing parentheses and quotes feels normal.
pub fn is_code_file(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();

    // Some files are recognized by name alone (no extension).
    let name_only = std::path::Path::new(&lower)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if matches!(
        name_only,
        "makefile" | "cmakelists.txt" | "dockerfile" | "rakefile" | "gemfile" | "build"
    ) {
        return true;
    }

    let Some(ext) = std::path::Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
    else {
        return false;
    };

    matches!(
        ext,
        // Compiled / systems
        "rs" | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hxx"
        | "go" | "zig" | "swift" | "kt" | "scala" | "java" | "cs"
        | "m" | "mm"
        // Scripting
        | "py" | "rb" | "pl" | "lua" | "tcl" | "r"
        | "sh" | "bash" | "zsh" | "fish" | "ps1"
        // Web
        | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx"
        | "html" | "htm" | "xml" | "svg"
        | "css" | "scss" | "sass" | "less"
        | "vue" | "svelte" | "astro"
        // Functional
        | "hs" | "elm" | "ex" | "exs" | "erl" | "clj" | "cljs"
        | "ml" | "mli" | "fs" | "fsx"
        // Data / config
        | "json" | "yaml" | "yml" | "toml" | "ini" | "conf"
        | "sql" | "graphql" | "gql"
        // Build / shell
        | "mk" | "cmake" | "nix"
    )
}
