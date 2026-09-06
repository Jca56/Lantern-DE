//! The whole app headless, through the shell harness.

use crate::app::*;
use std::path::PathBuf;
use lntrn_ui::{Axis, Shell};
use crate::session::Session;
use crate::settings::Settings;

use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, Modifiers};

/// The whole app headless: a narrow left area switched to Terminal at
/// runtime, then clicked, typed into, wheeled and resized.
#[test]
fn terminal_area_at_runtime() {
    let session = Session { root: Some(PathBuf::from("/home/alva/Projects/lntrnlabs")), ..Session::default() };
    let mut app = App::new(Settings::default(), session, Vec::new());
    let mut shell = Shell::new(Editor::Code);
    let right = shell.screen.split(0, Axis::Horizontal, 0.2, Editor::Code).unwrap();
    shell.screen.split(right, Axis::Vertical, 0.67, Editor::Code);
    let mut h = Harness::new(2240.0, 1400.0);
    for _ in 0..3 {
        h.shell_frame(&mut shell, &mut app);
        app.apply_pending(&mut shell);
    }
    // The user picks Terminal from the left area's header dropdown.
    shell.screen.area_mut(0).unwrap().set_editor(Editor::Terminal);
    for _ in 0..3 {
        h.shell_frame(&mut shell, &mut app);
        app.apply_pending(&mut shell);
        app.reap_terminals(&shell);
    }
    assert_eq!(app.terminals.len(), 1, "one terminal came up");
    let rect = shell.screen.layout_of(0).unwrap().body;
    h.click_at(rect.center(), |_| {});
    for _ in 0..2 {
        h.shell_frame(&mut shell, &mut app);
        app.apply_pending(&mut shell);
    }
    h.type_text("ls");
    h.key(Key::Enter);
    h.key(Key::ArrowUp);
    h.key_with(Key::Char('c'), Modifiers::CTRL);
    h.key(Key::F(5));
    h.wheel(3.0);
    h.move_to(rect.center() + Vec2::new(10.0, 10.0));
    for _ in 0..30 {
        h.advance(0.02);
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.shell_frame(&mut shell, &mut app);
        app.apply_pending(&mut shell);
        app.reap_terminals(&shell);
    }
    // Resize to something tiny and back.
    let mut small = Harness::new(300.0, 200.0);
    for _ in 0..3 {
        small.shell_frame(&mut shell, &mut app);
    }
    for _ in 0..3 {
        h.shell_frame(&mut shell, &mut app);
    }
    // Switched back to Code the tab keeps its shell (it comes back on
    // the next switch); closing the area lets it go.
    shell.screen.area_mut(0).unwrap().set_editor(Editor::Code);
    h.shell_frame(&mut shell, &mut app);
    app.reap_terminals(&shell);
    assert_eq!(app.terminals.len(), 1);
    shell.screen.join(0);
    app.reap_terminals(&shell);
    assert!(app.terminals.is_empty());
}

/// A build's errors read off a terminal reach the problem count, the
/// IDE's `getDiagnostics` answer, and put the caret on the line when
/// the problem is opened.
#[test]
fn problems_from_terminal_output() {
    use crate::problems::Severity;
    let dir = std::env::temp_dir().join(format!("lntrn-code-problems-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {\n    let x: u32 = \"a\";\n}\n").unwrap();
    let session = Session { root: Some(dir.clone()), ..Session::default() };
    let mut app = App::new(Settings::default(), session, Vec::new());
    let mut shell = Shell::new(Editor::Code);
    let mut h = Harness::new(1600.0, 1000.0);
    h.shell_frame(&mut shell, &mut app);
    app.apply_pending(&mut shell);
    let tid = app.new_terminal(Some(dir.clone()));
    let t = app.terminals.iter_mut().find(|t| t.id == tid).unwrap();
    t.diags.line_done("   Compiling demo v0.1.0");
    t.diags.line_done("error[E0308]: mismatched types");
    t.diags.line_done("  --> src/main.rs:2:18");
    t.diags.line_done("warning: unused variable: `x`");
    t.diags.line_done("  --> src/main.rs:2:9");
    t.diags.resolve_pending(Some(&dir), &[]);
    let count = |s: Severity| app.problems().iter().filter(|p| p.severity == s).count();
    assert_eq!((count(Severity::Error), count(Severity::Warning)), (1, 1));
    let json = app.diagnostics_json(None).to_text();
    assert!(json.contains("src/main.rs") && json.contains("\"severity\":\"Error\"") && json.contains("\"line\":1") && json.contains("\"character\":17"), "{json}");
    assert_eq!(app.diagnostics_json(Some(&dir.join("nope.rs"))).to_text(), "[]");
    let target = app.problems().into_iter().find_map(|p| p.path).expect("the path resolved");
    app.pending_paths.push(target.clone());
    app.pending_goto = Some((target.clone(), Goto::Printed { line: Some(2), col: Some(18) }));
    app.apply_pending(&mut shell);
    let doc = app.focus_doc().expect("the file opened");
    assert_eq!((doc.cursor.line, doc.cursor.col), (1, 17), "the caret sits on the problem");
    // A search hit selects its span.
    app.pending_goto = Some((target, Goto::Span { line: 1, col: 8, len: 1 }));
    app.apply_pending(&mut shell);
    let doc = app.focus_doc().unwrap();
    assert_eq!(doc.selected_text(), "x");
    let _ = std::fs::remove_dir_all(&dir);
}
