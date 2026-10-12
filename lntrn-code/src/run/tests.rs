//! Run through the whole app, headless: the status bar's button, the
//! dialog that asks for a command, real programs in the Run editor.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lntrn_math::Vec2;
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, Shell, WidgetId};

use crate::app::{App, Editor};
use crate::session::Session;
use crate::settings::Settings;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lntrn-code-run-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The app with `dir` as its project, a few frames in.
fn open(dir: &Path) -> (Harness, Shell<App>, App) {
    let session = Session { root: Some(dir.to_path_buf()), ..Session::default() };
    let mut app = App::new(Settings::default(), session, Vec::new());
    let mut shell = Shell::new(Editor::Code);
    let mut h = Harness::new(1600.0, 1000.0);
    for _ in 0..3 {
        frame(&mut h, &mut shell, &mut app);
    }
    assert!(app.project.is_some(), "the folder became the project");
    (h, shell, app)
}

fn frame(h: &mut Harness, shell: &mut Shell<App>, app: &mut App) {
    h.advance(0.02);
    h.shell_frame(shell, app);
    app.apply_pending(shell);
    app.reap_terminals(shell);
}

/// Frames until `done`, for a few seconds at most.
fn until(h: &mut Harness, shell: &mut Shell<App>, app: &mut App, done: impl Fn(&App) -> bool) -> bool {
    for _ in 0..500 {
        frame(h, shell, app);
        if done(app) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn press_button(h: &mut Harness, shell: &mut Shell<App>, app: &mut App) {
    let button = h.rect_of(WidgetId::ROOT.with("status").with("run")).expect("the Run button is in the status bar");
    let window = h.window();
    assert!(button.max.x <= window.max.x && button.min.y > window.max.y - 80.0, "at the right end of the bottom bar: {button:?}");
    h.move_to(button.center());
    h.press();
    frame(h, shell, app);
    h.release();
    frame(h, shell, app);
}

fn screen(app: &App) -> String {
    let Some(t) = app.run_terminal() else { return String::new() };
    (0..t.grid.rows).map(|y| t.grid.row(y).iter().map(|c| c.ch).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn exit_code(app: &App) -> Option<i32> {
    app.run_terminal().and_then(|t| t.exited)
}

/// A project with nothing to run asks for a command; the button then
/// runs it in the Run editor, stops one that goes on, and closing the
/// Run tab ends what is in it.
#[test]
fn the_button_asks_runs_and_stops() {
    let dir = scratch("custom");
    std::fs::write(dir.join("notes.md"), "nothing to run here\n").unwrap();
    let (mut h, mut shell, mut app) = open(&dir);
    assert!(app.run_plan_shown().is_none(), "the button found nothing to run");

    // Nothing to run: the button asks what Run should start here.
    press_button(&mut h, &mut shell, &mut app);
    assert!(shell.popup_open(), "the Run Command dialog");
    assert!(app.run.term.is_none());
    frame(&mut h, &mut shell, &mut app);
    h.type_text("echo lntrn-run-$((40 + 2))");
    frame(&mut h, &mut shell, &mut app);
    h.key(Key::Enter);
    for _ in 0..3 {
        frame(&mut h, &mut shell, &mut app);
    }
    assert!(!shell.popup_open());
    assert_eq!(app.run_custom(), Some("echo lntrn-run-$((40 + 2))"));

    // Now it runs, through the shell, in the project's folder.
    press_button(&mut h, &mut shell, &mut app);
    assert!(shell.screen.target(Editor::Run).is_some(), "the Run editor came on screen");
    assert!(until(&mut h, &mut shell, &mut app, |a| exit_code(a).is_some()), "the command ended");
    assert!(screen(&app).contains("lntrn-run-42"), "{}", screen(&app));
    assert_eq!(exit_code(&app), Some(0));
    assert_eq!(app.run_state(), "echo lntrn-run-$((40 + 2)) · finished");
    assert_eq!(app.terminals.len(), 1);

    // One that goes on: the button is Stop now, and stops it.
    let first = app.run.term;
    app.session.set_run_command(&app.project.as_ref().unwrap().root.clone(), Some("echo going; sleep 30"));
    app.run.plan = None;
    press_button(&mut h, &mut shell, &mut app);
    assert!(app.run.term != first && app.terminals.len() == 1, "a fresh terminal took the last run's place");
    assert!(until(&mut h, &mut shell, &mut app, |a| screen(a).contains("going")));
    assert!(app.run_active());
    assert!(app.run_state().ends_with("· running"), "{}", app.run_state());
    press_button(&mut h, &mut shell, &mut app);
    assert!(until(&mut h, &mut shell, &mut app, |a| !a.run_active()), "Stop interrupted it");
    assert_ne!(exit_code(&app), Some(0), "it did not sleep its thirty seconds out");

    // Closing the Run tab ends the program in it.
    press_button(&mut h, &mut shell, &mut app);
    assert!(until(&mut h, &mut shell, &mut app, |a| screen(a).contains("going")));
    let (area, _) = shell.screen.tab_hosting(Editor::Run).unwrap();
    shell.screen.area_mut(area).unwrap().set_editor(Editor::Code);
    frame(&mut h, &mut shell, &mut app);
    assert!(app.terminals.is_empty() && !app.run_active());
    let _ = std::fs::remove_dir_all(&dir);
}

/// F5 runs a Python project's `main.py` with nothing open, and again
/// from the run's own terminal once it has the keyboard.
#[test]
fn f5_runs_python() {
    let dir = scratch("python");
    std::fs::write(dir.join("main.py"), "import os\nprint('py-' + 'ok', os.path.basename(os.getcwd()) == os.path.basename(os.path.dirname(os.path.abspath(__file__))))\n").unwrap();
    let (mut h, mut shell, mut app) = open(&dir);
    let plan = app.run_plan_shown().cloned().expect("main.py was found");
    assert_eq!(plan.label, "main.py");
    if !plan.program_found() {
        eprintln!("no Python on this machine: not run");
        return;
    }
    h.key(Key::F(5));
    for _ in 0..3 {
        frame(&mut h, &mut shell, &mut app);
    }
    assert!(until(&mut h, &mut shell, &mut app, |a| exit_code(a).is_some()), "python ran");
    assert!(screen(&app).contains("py-ok True"), "{}", screen(&app));
    assert_eq!(exit_code(&app), Some(0));
    // The Run area is the active one and its terminal has the keys, F5 aside.
    let first = app.run.term;
    let body = shell.screen.layout_of(shell.screen.target(Editor::Run).unwrap()).unwrap().body;
    h.click_at(body.center(), |_| {});
    for _ in 0..2 {
        frame(&mut h, &mut shell, &mut app);
    }
    h.key(Key::F(5));
    for _ in 0..3 {
        frame(&mut h, &mut shell, &mut app);
    }
    assert!(app.run.term.is_some() && app.run.term != first, "F5 from the run's terminal ran it again");
    assert!(until(&mut h, &mut shell, &mut app, |a| exit_code(a) == Some(0)));
    // A right click on the button opens the Run menu.
    let button = h.rect_of(WidgetId::ROOT.with("status").with("run")).unwrap();
    h.move_to(button.center() + Vec2::new(1.0, 0.0));
    h.right_press();
    for _ in 0..2 {
        frame(&mut h, &mut shell, &mut app);
    }
    assert!(shell.popup_open(), "the Run menu");
    let _ = std::fs::remove_dir_all(&dir);
}
