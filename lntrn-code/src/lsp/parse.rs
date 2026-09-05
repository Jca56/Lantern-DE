//! The shapes a language server sends, read into ours: diagnostics,
//! hover text, definition locations, completion items, progress.

use std::path::PathBuf;

use super::pos::uri_to_path;
use super::{CompletionItem, LspDiag, TextEdit};
use crate::json::Json;
use crate::problems::{LspSpan, Severity};

fn position(j: &Json) -> Option<(usize, usize)> {
    Some((j.get("line")?.num()? as usize, j.get("character")?.num()? as usize))
}

/// `{start, end}` as `(line, col, end_line, end_col)`.
pub fn range(j: &Json) -> Option<(usize, usize, usize, usize)> {
    let (l, c) = position(j.get("start")?)?;
    let (el, ec) = position(j.get("end")?)?;
    Some((l, c, el, ec))
}

fn text_edit(j: &Json) -> Option<TextEdit> {
    let text = j.get("newText")?.str()?.to_owned();
    // An InsertReplaceEdit carries two ranges; replacing is what a pick does.
    let r = j.get("range").or_else(|| j.get("replace"))?;
    let (line, col, end_line, end_col) = range(r)?;
    Some(TextEdit { line, col, end_line, end_col, text })
}

/// `textDocument/publishDiagnostics`.
pub fn diagnostics(params: &Json, utf16: bool) -> Option<(PathBuf, Vec<LspDiag>)> {
    let path = uri_to_path(params.get("uri")?.str()?)?;
    let mut out = Vec::new();
    for d in params.get("diagnostics").and_then(Json::arr).unwrap_or(&[]) {
        let Some((line, col, end_line, end_col)) = d.get("range").and_then(range) else {
            continue;
        };
        let severity = match d.get("severity").and_then(Json::num).unwrap_or(1.0) as u8 {
            1 => Severity::Error,
            2 => Severity::Warning,
            3 => Severity::Info,
            _ => Severity::Hint,
        };
        let message = d.field_str("message").to_owned();
        let source = d.field_str("source").to_owned();
        out.push(LspDiag { span: LspSpan { line, col, end_line, end_col, utf16 }, severity, message, source });
    }
    Some((path, out))
}

/// The text of a hover result: markup or marked strings, code fences
/// dropped, blank runs squeezed.
pub fn hover_text(result: &Json) -> Option<String> {
    let contents = result.get("contents")?;
    let mut parts: Vec<String> = Vec::new();
    let mut push = |j: &Json| {
        if let Some(s) = j.str() {
            parts.push(s.to_owned());
        } else if let Some(v) = j.get("value").and_then(Json::str) {
            parts.push(v.to_owned());
        }
    };
    match contents {
        Json::Arr(items) => items.iter().for_each(&mut push),
        other => push(other),
    }
    let mut lines: Vec<String> = Vec::new();
    for part in parts {
        for l in part.lines() {
            if l.trim_start().starts_with("```") {
                continue;
            }
            let l = l.trim_end();
            if l.is_empty() && lines.last().is_some_and(|p| p.is_empty()) {
                continue;
            }
            lines.push(l.to_owned());
        }
        if !lines.last().is_some_and(|p| p.is_empty()) {
            lines.push(String::new());
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The first location of a definition result: `Location`,
/// `Location[]` or `LocationLink[]`.
pub fn definition(result: &Json) -> Option<(PathBuf, usize, usize, usize, usize)> {
    let first = match result {
        Json::Arr(items) => items.first()?,
        other => other,
    };
    if let Some(uri) = first.get("targetUri").and_then(Json::str) {
        let r = first.get("targetSelectionRange").or_else(|| first.get("targetRange"))?;
        let (l, c, el, ec) = range(r)?;
        return Some((uri_to_path(uri)?, l, c, el, ec));
    }
    let uri = first.get("uri")?.str()?;
    let (l, c, el, ec) = range(first.get("range")?)?;
    Some((uri_to_path(uri)?, l, c, el, ec))
}

/// Snippet placeholders (`$0`, `${1:name}`) as plain text.
fn unsnippet(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('{') => {
                chars.next();
                let mut inner = String::new();
                let mut depth = 1;
                for c in chars.by_ref() {
                    if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    inner.push(c);
                }
                if let Some((_, name)) = inner.split_once(':') {
                    out.push_str(&unsnippet(name));
                }
            }
            Some(d) if d.is_ascii_digit() => {
                while chars.peek().is_some_and(|d| d.is_ascii_digit()) {
                    chars.next();
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

/// `textDocument/completion`: a list, or `{items, isIncomplete}`.
pub fn completions(result: &Json) -> Vec<CompletionItem> {
    let items = match result {
        Json::Arr(a) => a.as_slice(),
        other => other.get("items").and_then(Json::arr).unwrap_or(&[]),
    };
    let mut out = Vec::new();
    for it in items {
        let label = it.field_str("label").to_owned();
        if label.is_empty() {
            continue;
        }
        let snippet = it.get("insertTextFormat").and_then(Json::num) == Some(2.0);
        let raw = it.get("insertText").and_then(Json::str).unwrap_or(&label).to_owned();
        let insert = if snippet { unsnippet(&raw) } else { raw };
        let mut edit = it.get("textEdit").and_then(text_edit);
        if let (true, Some(e)) = (snippet, edit.as_mut()) {
            e.text = unsnippet(&e.text);
        }
        let extra = it.get("additionalTextEdits").and_then(Json::arr).map(|a| a.iter().filter_map(text_edit).collect()).unwrap_or_default();
        let detail = it.get("detail").and_then(Json::str).unwrap_or("").to_owned();
        let kind = it.get("kind").and_then(Json::num).unwrap_or(0.0) as u32;
        let filter = it.get("filterText").and_then(Json::str).unwrap_or(&label).to_owned();
        let sort = it.get("sortText").and_then(Json::str).unwrap_or(&label).to_owned();
        out.push(CompletionItem { label, detail, kind, insert, edit, extra, filter, sort });
    }
    out.sort_by(|a, b| a.sort.cmp(&b.sort).then_with(|| a.label.cmp(&b.label)));
    out
}

/// `$/progress`: the token, what to show, and whether it ended.
pub fn progress(params: &Json) -> Option<(String, String, bool)> {
    let token = match params.get("token")? {
        Json::Str(s) => s.clone(),
        Json::Num(n) => n.to_string(),
        _ => return None,
    };
    let value = params.get("value")?;
    let kind = value.field_str("kind");
    let mut text = value.get("title").and_then(Json::str).unwrap_or("").to_owned();
    if let Some(m) = value.get("message").and_then(Json::str) {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(m);
    }
    if let Some(p) = value.get("percentage").and_then(Json::num) {
        text.push_str(&format!(" {}%", p.round() as u32));
    }
    Some((token, text.trim().to_owned(), kind == "end"))
}

/// Whether the server wants UTF-16 columns (the default) after
/// `initialize`.
pub fn wants_utf16(init_result: &Json) -> bool {
    init_result.get("capabilities").and_then(|c| c.get("positionEncoding")).and_then(Json::str) != Some("utf-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_results() {
        let d = Json::parse(r#"{"uri":"file:///p/a.rs","diagnostics":[{"range":{"start":{"line":1,"character":4},"end":{"line":1,"character":9}},"severity":2,"message":"unused","source":"rustc"}]}"#).unwrap();
        let (path, diags) = diagnostics(&d, false).unwrap();
        assert_eq!(path, PathBuf::from("/p/a.rs"));
        assert_eq!((diags[0].severity, diags[0].span.col, diags[0].span.end_col, diags[0].source.as_str()), (Severity::Warning, 4, 9, "rustc"));
        let h = Json::parse(r#"{"contents":{"kind":"markdown","value":"```rust\nfn main()\n```\n\n\nDocs here\n"}}"#).unwrap();
        assert_eq!(hover_text(&h).unwrap(), "fn main()\n\nDocs here");
        let h2 = Json::parse(r#"{"contents":["a",{"language":"rust","value":"b"}]}"#).unwrap();
        assert_eq!(hover_text(&h2).unwrap(), "a\n\nb");
        let def = Json::parse(r#"[{"targetUri":"file:///p/b.rs","targetRange":{"start":{"line":0,"character":0},"end":{"line":9,"character":0}},"targetSelectionRange":{"start":{"line":2,"character":3},"end":{"line":2,"character":7}}}]"#).unwrap();
        assert_eq!(definition(&def).unwrap(), (PathBuf::from("/p/b.rs"), 2, 3, 2, 7));
        let c = Json::parse(r#"{"isIncomplete":false,"items":[{"label":"push(…)","sortText":"b","insertText":"push(${1:value})$0","insertTextFormat":2,"textEdit":{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":4}},"newText":"push(${1:value})$0"},"detail":"fn push(&mut self, value: T)","kind":2},{"label":"len()","sortText":"a","insertText":"len()"}]}"#).unwrap();
        let items = completions(&c);
        assert_eq!(items[0].label, "len()", "sorted by sortText");
        assert_eq!(items[1].insert, "push(value)");
        assert_eq!(items[1].edit.as_ref().unwrap().text, "push(value)");
        assert_eq!(unsnippet("a$0b${2}c$"), "abc$");
        let p = Json::parse(r#"{"token":"rustAnalyzer/Indexing","value":{"kind":"report","message":"3/10 crates","percentage":30}}"#).unwrap();
        assert_eq!(progress(&p).unwrap(), ("rustAnalyzer/Indexing".into(), "3/10 crates 30%".into(), false));
        assert!(!wants_utf16(&Json::parse(r#"{"capabilities":{"positionEncoding":"utf-8"}}"#).unwrap()));
        assert!(wants_utf16(&Json::parse(r#"{"capabilities":{}}"#).unwrap()));
    }
}
