//! Ending a process: what is asked first, and the signalling itself.

use lntrn_ui::{Action, Dialog, ShellRequest};

use crate::ffi::{self, SIGKILL};
use crate::rows::{self, Pick, View};
use crate::sample::procs::{self, Proc};

/// What a confirmed "End" or "Force Kill" goes to.
#[derive(Clone, Debug, PartialEq)]
pub struct Doomed {
    pub name: String,
    /// Each process's number and when it started.
    pub targets: Vec<(u32, u64)>,
    pub signal: i32,
}

/// Whether the desktop stands on `p`: ending it ends the session.
fn holds_the_desktop_up(p: &Proc) -> bool {
    p.pid == 1 || ["lntrn-compositor", "lntrn-session-manager"].contains(&&*p.name)
}

/// The question to ask before sending `signal` to what `pick` stands
/// for, and what to do if the answer is yes. `None` when it stands for
/// nothing any more.
pub fn ask(procs: &[Proc], view: &View, pick: &Pick, signal: i32) -> Option<(Doomed, ShellRequest)> {
    let targets = rows::targets(procs, view, pick);
    if targets.is_empty() {
        return None;
    }
    let name = rows::pick_name(procs, pick);
    let many = targets.len() > 1;
    let who = if many { format!("Its {} processes are", targets.len()) } else { "It is".to_owned() };
    let (title, what, button) = if signal == SIGKILL {
        (format!("Force {name} to quit?"), format!("{who} stopped at once. Anything {} saved is lost.", if many { "they haven't" } else { "it hasn't" }), "Force Quit")
    } else {
        (format!("End {name}?"), format!("{who} asked to quit, so {} save {} work first.", if many { "they can" } else { "it can" }, if many { "their" } else { "its" }), "End")
    };
    let vital = procs.iter().any(|p| targets.iter().any(|t| t.0 == p.pid) && holds_the_desktop_up(p));
    let body = if vital { format!("The desktop stands on this: ending it ends your session. {what}") } else { what };
    let dialog = Dialog::confirm(&title, &body, button, Action::new("procs.signal"));
    // Enter ends politely; what can lose work wants the button pressed.
    let dialog = if signal == SIGKILL || vital { dialog.default_button(0) } else { dialog };
    Some((Doomed { name, targets, signal }, ShellRequest::Dialog(dialog)))
}

/// Send the signal, and say in a few words how it went. A process that
/// has gone since it was listed, or whose number has been handed to a
/// newer one, is left alone.
pub fn carry_out(doomed: &Doomed) -> String {
    let (mut sent, mut refused) = (0, None);
    for (pid, started) in &doomed.targets {
        if procs::started(*pid) != Some(*started) {
            continue;
        }
        match ffi::signal(*pid, doomed.signal) {
            Ok(()) => sent += 1,
            Err(why) => refused = Some(why),
        }
    }
    let name = &doomed.name;
    match (sent, refused) {
        (0, None) => format!("{name} had already gone"),
        (0, Some(why)) => format!("Couldn't end {name}: {why}"),
        (_, Some(why)) => format!("Some of {name} wouldn't go: {why}"),
        (_, None) if doomed.signal == SIGKILL => format!("Stopped {name}"),
        (_, None) => format!("Asked {name} to quit"),
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;

    use super::*;
    use crate::ffi::SIGTERM;
    use crate::sample::fixture;

    fn dialog(request: ShellRequest) -> Dialog {
        match request {
            ShellRequest::Dialog(d) => d,
            other => panic!("a dialog, not {other:?}"),
        }
    }

    #[test]
    fn the_question_says_what_will_happen_and_to_how_many() {
        let procs = fixture::procs();
        let view = View { grouped: true, ..View::default() };
        let (doomed, request) = ask(&procs, &view, &Pick::Pid(1300), SIGTERM).unwrap();
        let d = dialog(request);
        assert_eq!((d.title.as_str(), d.body.as_str(), d.default), ("End zsh?", "It is asked to quit, so it can save its work first.", 1));
        assert_eq!(doomed, Doomed { name: "zsh".to_owned(), targets: vec![(1300, 13_000)], signal: SIGTERM });

        let (doomed, request) = ask(&procs, &view, &Pick::App(Arc::from("firefox")), SIGKILL).unwrap();
        let d = dialog(request);
        assert_eq!(d.title, "Force firefox to quit?");
        assert_eq!(d.body, "Its 4 processes are stopped at once. Anything they haven't saved is lost.");
        assert_eq!((d.default, d.buttons[1].0.as_str(), doomed.targets.len()), (0, "Force Quit", 4));

        // What the desktop stands on is warned about, and Enter cancels.
        let d = dialog(ask(&procs, &view, &Pick::Pid(1201), SIGTERM).unwrap().1);
        assert!(d.body.starts_with("The desktop stands on this"), "{}", d.body);
        assert_eq!(d.default, 0);
        assert!(ask(&procs, &view, &Pick::Pid(99_999), SIGTERM).is_none());
    }

    #[test]
    fn a_real_process_is_ended_and_a_stale_number_is_left_alone() {
        let mut child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id();
        let born = procs::started(pid).expect("the child is running");

        // The same number, but not the process that was listed: untouched.
        let stale = Doomed { name: "sleep".to_owned(), targets: vec![(pid, born + 1)], signal: SIGKILL };
        assert_eq!(carry_out(&stale), "sleep had already gone");
        assert!(child.try_wait().unwrap().is_none(), "still running");

        let real = Doomed { name: "sleep".to_owned(), targets: vec![(pid, born)], signal: SIGTERM };
        assert_eq!(carry_out(&real), "Asked sleep to quit");
        assert!(!child.wait().unwrap().success(), "ended by the signal");
    }
}
