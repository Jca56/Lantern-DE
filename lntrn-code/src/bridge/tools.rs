//! The MCP surface Claude Code expects of an IDE: the tool list with its
//! schemas, and the shapes of the answers.

use crate::json::Json;
use crate::obj;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn initialize_result(params: &Json) -> Json {
    let version = params.get("protocolVersion").and_then(Json::str).unwrap_or("2024-11-05");
    obj! {
        "protocolVersion" => version,
        "capabilities" => obj! { "tools" => obj! { "listChanged" => true } },
        "serverInfo" => obj! { "name" => "lntrn-code", "version" => VERSION },
    }
}

fn prop(name: &str, kind: &str, desc: &str) -> (String, Json) {
    (name.to_owned(), obj! { "type" => kind, "description" => desc })
}

fn tool(name: &str, description: &str, props: Vec<(String, Json)>, required: &[&str]) -> Json {
    let required: Vec<Json> = required.iter().map(|r| Json::from(*r)).collect();
    obj! {
        "name" => name,
        "description" => description,
        "inputSchema" => obj! { "type" => "object", "properties" => Json::Obj(props), "required" => required },
    }
}

pub fn list() -> Json {
    let s = |n: &str, d: &str| prop(n, "string", d);
    let b = |n: &str, d: &str| prop(n, "boolean", d);
    obj! {
        "tools" => vec![
            tool("openFile", "Open a file in the editor and optionally select a range of text", vec![s("filePath", "Path to the file"), b("preview", "Open in preview mode"), s("startText", "Text to find for the start of the selection"), s("endText", "Text to find for the end of the selection"), b("selectToEndOfLine", "Extend the selection to the end of the line"), b("makeFrontmost", "Bring the file to the front")], &["filePath"]),
            tool("openDiff", "Show a diff of a proposed change and wait for the user to accept or reject it", vec![s("old_file_path", "Path of the file as it is"), s("new_file_path", "Path the new contents go to"), s("new_file_contents", "The proposed contents"), s("tab_name", "Name of the diff tab")], &["old_file_path", "new_file_path", "new_file_contents", "tab_name"]),
            tool("getCurrentSelection", "The text selected in the focused editor", Vec::new(), &[]),
            tool("getLatestSelection", "The most recent selection, focused or not", Vec::new(), &[]),
            tool("getOpenEditors", "The files open in the editor", Vec::new(), &[]),
            tool("getWorkspaceFolders", "The project folders open in the editor", Vec::new(), &[]),
            tool("getDiagnostics", "Problems the editor knows about", vec![s("uri", "Only this file")], &[]),
            tool("checkDocumentDirty", "Whether a file has unsaved changes", vec![s("filePath", "Path to the file")], &["filePath"]),
            tool("saveDocument", "Save a file", vec![s("filePath", "Path to the file")], &["filePath"]),
            tool("close_tab", "Close a diff tab by name", vec![s("tab_name", "Name of the tab")], &["tab_name"]),
            tool("closeAllDiffTabs", "Close every diff tab", Vec::new(), &[]),
        ],
    }
}

/// A result of one text block.
pub fn text(t: &str) -> Json {
    obj! { "content" => vec![obj! { "type" => "text", "text" => t }] }
}

/// A result of several text blocks.
pub fn texts(ts: &[&str]) -> Json {
    let blocks: Vec<Json> = ts.iter().map(|t| obj! { "type" => "text", "text" => *t }).collect();
    obj! { "content" => blocks }
}

/// A JSON value, as the text block the CLI expects it in.
pub fn json_text(j: &Json) -> Json {
    text(&j.to_text())
}

pub fn error(message: &str) -> Json {
    obj! { "content" => vec![obj! { "type" => "text", "text" => message }], "isError" => true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let l = list();
        let names: Vec<&str> = l.get("tools").unwrap().arr().unwrap().iter().map(|t| t.field_str("name")).collect();
        assert!(names.contains(&"openDiff") && names.contains(&"getCurrentSelection") && names.len() == 11);
        let open = &l.get("tools").unwrap().arr().unwrap()[0];
        assert_eq!(open.get("inputSchema").unwrap().get("required").unwrap().arr().unwrap()[0], Json::from("filePath"));
        assert_eq!(text("hi").to_text(), r#"{"content":[{"type":"text","text":"hi"}]}"#);
        assert!(error("no").get("isError").unwrap().bool().unwrap());
        let init = initialize_result(&obj! { "protocolVersion" => "2025-03-26" });
        assert_eq!(init.field_str("protocolVersion"), "2025-03-26");
        assert_eq!(initialize_result(&Json::Null).field_str("protocolVersion"), "2024-11-05");
    }
}
