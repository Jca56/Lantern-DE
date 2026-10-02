//! What the pill says for each report, when the held-deletions question
//! opens by itself, and what its dialog offers.

use std::time::{Duration, Instant};

use super::pill::*;
use super::view::*;
use super::*;
use crate::cloud::failures::{StuckFile, StuckKind};
use crate::cloud::guard::HoldReason;
use crate::cloud::sync::SyncStatus;

fn report(status: SyncStatus, problem: Option<SyncProblem>) -> SyncReport {
    SyncReport {
        status,
        problem,
        held: None,
        unreadable: 0,
        failed: 0,
        stuck: 0,
        stuck_sample: Vec::new(),
        uid: "alice".to_string(),
    }
}

fn held(id: &str, count: usize, total: usize, reason: HoldReason) -> HeldDeletions {
    HeldDeletions {
        held: count,
        total,
        reason,
        sample: (0..count.min(40))
            .map(|i| format!("Photos/img{i:03}.jpg"))
            .collect(),
        id: id.to_string(),
    }
}

fn holding(h: HeldDeletions) -> SyncReport {
    let mut r = report(SyncStatus::Error, Some(SyncProblem::DeletionsHeld));
    r.held = Some(h);
    r
}

fn pill_of(report: &SyncReport) -> Pill {
    pill_for(&PillInput {
        report: Some(report),
        sign_in: None,
        signed_in: true,
        answered: false,
    })
    .expect("a window with an engine always has a pill")
}

// ── The pill ───────────────────────────────────────────────────────────────

#[test]
fn held_deletions_make_the_pill_a_loud_paused_one_that_opens_the_question() {
    let r = holding(held("a", 132, 4370, HoldReason::TooMany));
    let pill = pill_of(&r);
    assert_eq!(pill.label, "Sync paused: 132 files missing");
    assert_eq!(pill.short, "Sync paused");
    assert_eq!(pill.tone, Tone::Warning);
    assert_eq!(pill.action, PillAction::Held);
    assert!(pill.loud, "shown in every folder, not only inside ~/Cloud");

    // Held deletions win over whatever else the report says.
    let mut quota = holding(held("a", 1, 12, HoldReason::ScanFailed));
    quota.status = SyncStatus::RateLimited;
    assert_eq!(pill_of(&quota).label, "Sync paused: 1 file missing");
    assert_eq!(pill_of(&quota).action, PillAction::Held);
}

#[test]
fn an_answer_on_its_way_shows_as_work_not_as_a_question() {
    let r = holding(held("a", 20, 40, HoldReason::TooMany));
    let pill = pill_for(&PillInput {
        report: Some(&r),
        sign_in: None,
        signed_in: true,
        answered: true,
    })
    .unwrap();
    assert_eq!(pill.tone, Tone::Busy);
    // Still opens the question: the answer can be changed until it is taken.
    assert_eq!(pill.action, PillAction::Held);
}

#[test]
fn the_quiet_states_stay_inside_the_cloud_folder() {
    for (status, label) in [
        (SyncStatus::Idle, "Synced"),
        (SyncStatus::Syncing, "Syncing\u{2026}"),
        (SyncStatus::RateLimited, "Sync paused (quota)"),
    ] {
        let pill = pill_of(&report(status, None));
        assert_eq!(pill.label, label);
        assert!(!pill.loud);
        assert_eq!(pill.action, PillAction::Details);
    }
}

#[test]
fn a_missing_or_unreadable_folder_is_said_everywhere() {
    let pill = pill_of(&report(SyncStatus::Error, Some(SyncProblem::FolderMissing)));
    assert_eq!(pill.label, "Cloud folder is missing, sync paused");
    assert!(pill.loud);
    assert_eq!(pill.tone, Tone::Warning);
    let pill = pill_of(&report(
        SyncStatus::Error,
        Some(SyncProblem::FolderUnreadable),
    ));
    assert!(pill.loud);
    assert_eq!(pill.action, PillAction::Details);
}

#[test]
fn files_that_cannot_sync_are_counted_on_the_pill() {
    let mut r = report(SyncStatus::Error, Some(SyncProblem::CannotSync));
    r.stuck = 3;
    assert_eq!(pill_of(&r).label, "3 files cannot be synced");
    assert_eq!(pill_of(&r).action, PillAction::Details);

    let mut r = report(SyncStatus::Error, Some(SyncProblem::Failed));
    r.failed = 1;
    assert_eq!(pill_of(&r).label, "1 file failed to sync");
    assert_eq!(pill_of(&r).tone, Tone::Danger);
    r.failed = 0;
    assert_eq!(pill_of(&r).label, "Sync error");

    let mut r = report(SyncStatus::Error, Some(SyncProblem::Unreadable));
    r.unreadable = 2;
    assert_eq!(pill_of(&r).label, "2 items could not be read");
}

#[test]
fn a_lost_sign_in_offers_to_sign_in() {
    let revoked = pill_for(&PillInput {
        report: None,
        sign_in: Some(SignIn::Revoked),
        signed_in: false,
        answered: false,
    })
    .unwrap();
    assert_eq!(revoked.action, PillAction::SignIn);
    assert!(
        revoked.loud,
        "sync stopped behind the user's back: say it everywhere"
    );

    let signed_out = pill_for(&PillInput {
        report: None,
        sign_in: Some(SignIn::SignedOut),
        signed_in: false,
        answered: false,
    })
    .unwrap();
    assert_eq!(signed_out.action, PillAction::SignIn);
    assert!(!signed_out.loud, "signing out was the user's own doing");

    let never = pill_for(&PillInput {
        report: None,
        sign_in: None,
        signed_in: false,
        answered: false,
    })
    .unwrap();
    assert_eq!(never.label, "Sign in to sync");
}

#[test]
fn a_picker_has_no_pill() {
    // Signed in, no engine in this process.
    let pill = pill_for(&PillInput {
        report: None,
        sign_in: None,
        signed_in: true,
        answered: false,
    });
    assert_eq!(pill, None);
}

// ── When the question opens by itself ──────────────────────────────────────

#[test]
fn the_question_opens_once_per_hold() {
    let mut ask = AskState::default();
    let now = Instant::now();
    let first = held("a", 30, 60, HoldReason::TooMany);
    assert!(!ask.observe(None, now));
    assert!(ask.observe(Some(&first), now));
    // "Decide later": the same report, read four times a second.
    for _ in 0..10 {
        assert!(!ask.observe(Some(&first), now));
    }
    // A few files more in the same hold is not a new question...
    let more = held("b", 33, 60, HoldReason::TooMany);
    assert!(!ask.observe(Some(&more), now));
    // ...but the folder turning up empty is.
    let empty = held("c", 60, 60, HoldReason::EmptyFolder);
    assert!(ask.observe(Some(&empty), now));
}

#[test]
fn a_gap_in_the_reports_does_not_ask_about_the_same_set_again() {
    let mut ask = AskState::default();
    let now = Instant::now();
    let set = held("a", 30, 60, HoldReason::TooMany);
    assert!(ask.observe(Some(&set), now));
    // Another window took over as the sync owner: its first report holds
    // nothing, its first pass holds the same files.
    assert!(!ask.observe(None, now));
    assert!(!ask.observe(Some(&set), now));
    // After a gap, a different set is a new question.
    assert!(!ask.observe(None, now));
    let other = held("b", 31, 60, HoldReason::TooMany);
    assert!(ask.observe(Some(&other), now));
}

#[test]
fn an_answered_question_is_not_asked_again_while_the_answer_is_on_its_way() {
    let mut ask = AskState::default();
    let now = Instant::now();
    let set = held("a", 30, 60, HoldReason::TooMany);
    assert!(ask.observe(Some(&set), now));
    ask.answered("a", now);
    assert!(ask.is_answered("a"));
    // The engine is in the middle of a long upload and has not looked yet.
    assert!(!ask.observe(Some(&set), now + Duration::from_secs(120)));
    // It took the answer: nothing is held.
    assert!(!ask.observe(None, now + Duration::from_secs(121)));
    assert!(!ask.is_answered("a"));
    // The same files held again later is a new question.
    assert!(ask.observe(Some(&set), now + Duration::from_secs(300)));
}

#[test]
fn an_answer_nobody_took_is_asked_again() {
    let mut ask = AskState::default();
    let now = Instant::now();
    let set = held("a", 30, 60, HoldReason::TooMany);
    assert!(ask.observe(Some(&set), now));
    ask.answered("a", now);
    assert!(!ask.observe(Some(&set), now + Duration::from_secs(599)));
    assert!(ask.observe(Some(&set), now + Duration::from_secs(601)));
    assert!(!ask.is_answered("a"));
}

#[test]
fn an_answer_for_a_set_that_changed_does_not_silence_the_new_set() {
    let mut ask = AskState::default();
    let now = Instant::now();
    assert!(ask.observe(Some(&held("a", 30, 60, HoldReason::TooMany)), now));
    ask.answered("a", now);
    // A pass computed another set before the answer was taken.
    assert!(ask.observe(Some(&held("b", 35, 60, HoldReason::TooMany)), now));
}

// ── The dialogs ────────────────────────────────────────────────────────────

#[test]
fn the_question_says_what_is_missing_and_defaults_to_restore() {
    let all: Vec<String> = (0..132).map(|i| format!("Photos/img{i:03}.jpg")).collect();
    let view = held_view(&held("a", 132, 4370, HoldReason::EmptyFolder), Some(all));
    assert_eq!(view.title, "Cloud sync is paused");
    assert!(view.lines[0].starts_with("132 of 4370 synced files are missing"));
    assert!(view.lines[0].ends_with("is in the folder any more."));
    assert!(view
        .lines
        .iter()
        .any(|l| l.contains("Nothing is deleted from the cloud until you choose")));
    // Every file that would be deleted is in the list, not a sample.
    assert_eq!(view.list.len(), 132);
    assert_eq!(view.list_total, 132);
    assert!(view.note.is_none());

    // Enter restores; deleting is a deliberate click on a button of its own.
    assert_eq!(view.default_zone(), Some(crate::ZONE_CLOUD_DLG_RESTORE));
    let delete = view.aside.expect("the destructive button is kept apart");
    assert_eq!(delete.label, "Delete everywhere");
    assert_eq!(delete.role, Role::Danger);
    assert_eq!(delete.zone, crate::ZONE_CLOUD_DLG_DELETE);
    let labels: Vec<&str> = view.buttons.iter().map(|b| b.label).collect();
    assert_eq!(labels, ["Decide later", "Restore them"]);
    // "Decide later" is the queue's safe button: it closes, nothing else.
    assert_eq!(view.buttons[0].zone, crate::ZONE_OP_DIALOG_SAFE);

    // A small set is complete in the report itself.
    let one = held_view(&held("b", 1, 12, HoldReason::ScanFailed), None);
    assert!(one.lines[0].starts_with("1 of 12 synced files is missing"));
    assert_eq!(one.buttons[1].label, "Restore it");
    assert!(one.aside.is_some());
}

/// Nothing may be deleted on the other devices that the user could not see
/// listed: without the complete list there is no "Delete everywhere".
#[test]
fn without_the_complete_list_deleting_everywhere_is_not_offered() {
    let h = held("a", 132, 4370, HoldReason::TooMany);
    for full in [None, Some(h.sample.clone())] {
        let view = held_view(&h, full);
        assert!(view.aside.is_none());
        assert!(!view.has_button(crate::ZONE_CLOUD_DLG_DELETE));
        assert!(view.note.as_deref().is_some_and(|n| n.contains("complete list")));
        assert!(view.lines.iter().all(|l| !l.contains("Delete everywhere")));
        // Restoring deletes nothing and stays available, and the default.
        assert_eq!(view.default_zone(), Some(crate::ZONE_CLOUD_DLG_RESTORE));
        // The rows say how many are not named.
        let rows = list_rows(&view.list, view.list_total);
        assert_eq!(rows.len(), 41);
        assert_eq!(rows[40], "\u{2026}and 92 more.");
    }
}

#[test]
fn the_details_name_the_files_that_cannot_sync() {
    let mut r = report(SyncStatus::Error, Some(SyncProblem::CannotSync));
    r.stuck = 45;
    r.stuck_sample = vec![
        StuckFile {
            path: "Videos/holiday.mkv".to_string(),
            kind: StuckKind::TooBig,
            detail: "1 GiB or larger".to_string(),
            attempts: 0,
        },
        StuckFile {
            path: "Notes/secret.txt".to_string(),
            kind: StuckKind::Unreadable,
            detail: "Permission denied".to_string(),
            attempts: 0,
        },
    ];
    let view = details_view(&r, "a@b.c");
    assert_eq!(view.lines[0], "Signed in as a@b.c.");
    assert!(view
        .lines
        .contains(&"45 files cannot be synced right now:".to_string()));
    assert_eq!(
        view.list,
        vec![
            "Videos/holiday.mkv: too big for the cloud (1 GiB or more)",
            "Notes/secret.txt: cannot be read (Permission denied)",
        ]
    );
    assert_eq!(view.list_total, 45);
    // Nothing here gets better by trying again.
    assert!(!view.has_button(crate::ZONE_CLOUD_DLG_RETRY));
    assert_eq!(view.default_zone(), Some(crate::ZONE_OP_DIALOG_SAFE));

    r.stuck_sample.push(StuckFile {
        path: "big.iso".to_string(),
        kind: StuckKind::Failing,
        detail: "status code 402".to_string(),
        attempts: 4,
    });
    let view = details_view(&r, "a@b.c");
    assert!(view.has_button(crate::ZONE_CLOUD_DLG_RETRY));
    assert_eq!(
        view.list[2],
        "big.iso: failed 4 times, waiting to be tried again (status code 402)"
    );
}

#[test]
fn a_missing_folder_is_explained_and_never_created_without_asking() {
    let view = details_view(
        &report(SyncStatus::Error, Some(SyncProblem::FolderMissing)),
        "a@b.c",
    );
    assert_eq!(view.title, "The Cloud folder is missing");
    assert!(view.lines[0].contains("Nothing has been deleted"));
    assert!(view.has_button(crate::ZONE_CLOUD_DLG_CREATE));
    // Enter closes; creating the folder is a click.
    assert_eq!(view.default_zone(), Some(crate::ZONE_OP_DIALOG_SAFE));
}

#[test]
fn the_rows_are_every_name_and_a_count_of_the_ones_not_known() {
    let names: Vec<String> = (0..40).map(|i| format!("f{i}")).collect();
    assert_eq!(list_rows(&names[..3], 3), names[..3].to_vec());
    let rows = list_rows(&names, 132);
    assert_eq!(rows.len(), 41);
    assert_eq!(rows[39], "f39");
    assert_eq!(rows[40], "\u{2026}and 92 more.");
    assert!(list_rows(&[], 0).is_empty());
}

#[test]
fn a_question_that_switches_to_another_set_is_deaf_again_and_starts_at_the_top() {
    let now = Instant::now();
    let first = held("a", 30, 60, HoldReason::TooMany);
    let mut dialog = CloudDialog::held(&first, None, now);
    dialog.scroll.set(300.0);
    assert!(Instant::now() >= dialog.armed_at);
    let second = held("b", 35, 60, HoldReason::TooMany);
    dialog.show_held(&second, None, "The list has changed since this question opened.");
    assert!(matches!(&dialog.kind, Kind::Held { id } if id == "b"));
    assert!(dialog.armed_at > Instant::now(), "a click on its way is not for this list");
    assert_eq!(dialog.scroll.get(), 0.0);
    assert!(dialog.view.note.as_deref().is_some_and(|n| n.starts_with("The list has changed")));
}

// ── The window ─────────────────────────────────────────────────────────────

fn front_question(app: &App) -> Option<&CloudDialog> {
    match app.op_dialogs.front() {
        Some(OpDialog::Cloud(dialog)) if matches!(dialog.kind, Kind::Held { .. }) => Some(dialog),
        _ => None,
    }
}

/// A moment ago: a question opened then takes input by now.
fn a_moment_ago() -> Instant {
    Instant::now()
        .checked_sub(Duration::from_secs(2))
        .expect("the clock has run for two seconds")
}

#[test]
fn a_hold_opens_the_question_and_decide_later_keeps_it_closed() {
    let mut app = App::new();
    let now = a_moment_ago();
    let r = holding(held("a", 30, 60, HoldReason::TooMany));

    assert!(app.cloud_report_read(&r, now));
    assert!(front_question(&app).is_some());
    assert!(
        app.op_dialog_open(),
        "modal: the view behind it takes no clicks"
    );

    // Decide later (also Esc and a click outside).
    app.op_dialog_choose(false);
    assert!(app.op_dialogs.is_empty());
    // The same report keeps coming; the question does not.
    for _ in 0..5 {
        assert!(!app.cloud_report_read(&r, now));
    }
    assert!(app.op_dialogs.is_empty());
}

#[test]
fn a_question_that_only_just_appeared_takes_no_input() {
    let mut app = App::new();
    let now = Instant::now();
    app.cloud_report_read(&holding(held("a", 30, 60, HoldReason::TooMany)), now);
    // The key or click was on its way to whatever was on screen before:
    // it neither answers the question nor closes it unread.
    app.op_dialog_enter();
    app.cloud_dialog_button(crate::ZONE_CLOUD_DLG_DELETE);
    app.cloud_dialog_button(crate::ZONE_CLOUD_DLG_RESTORE);
    app.op_dialog_choose(false);
    assert!(front_question(&app).is_some());
}

#[test]
fn an_open_question_follows_the_report() {
    let mut app = App::new();
    let now = Instant::now();
    app.cloud_report_read(&holding(held("a", 30, 60, HoldReason::TooMany)), now);

    // The set changed: what is on screen is what an answer would be for.
    app.cloud_report_read(&holding(held("b", 31, 60, HoldReason::TooMany)), now);
    assert_eq!(app.op_dialogs.len(), 1);
    let dialog = front_question(&app).unwrap();
    assert_eq!(
        dialog.kind,
        Kind::Held {
            id: "b".to_string()
        }
    );
    assert!(dialog.view.lines[0].starts_with("31 of 60"));
    assert!(dialog.view.note.is_some());

    // Answered in another window: the question goes away here too.
    app.cloud_report_read(&report(SyncStatus::Syncing, None), now);
    assert!(app.op_dialogs.is_empty());
}

#[test]
fn only_cloud_buttons_reach_the_cloud_dialogs() {
    assert!(is_dialog_zone(crate::ZONE_CLOUD_DLG_RESTORE));
    assert!(is_dialog_zone(crate::ZONE_CLOUD_DLG_DELETE));
    assert!(!is_dialog_zone(crate::ZONE_OP_DIALOG_ACT));
    assert!(!is_dialog_zone(crate::ZONE_CLOUD_PILL));

    // A button the dialog on screen does not have does nothing.
    let mut app = App::new();
    app.cloud_report_read(
        &holding(held("a", 30, 60, HoldReason::TooMany)),
        a_moment_ago(),
    );
    app.cloud_dialog_button(crate::ZONE_CLOUD_DLG_SIGN_OUT);
    assert!(front_question(&app).is_some());
}
