//! The page's state against a stand-in compositor: what is asked of it,
//! when, and what reaches the file.

use super::super::fake::{Fake, two_heads};
use super::*;

/// The page with a stand-in compositor holding `heads`, the file as
/// `cfg`, and the first look taken.
fn page(heads: Vec<Head>, cfg: &[Monitor]) -> (MonitorsState, Fake) {
    let fake = Fake::with(heads);
    let mut st = MonitorsState { source: Some(Box::new(fake.clone())), ..MonitorsState::default() };
    assert_eq!(st.tick(cfg, 0.0), None);
    (st, fake)
}

fn file() -> Vec<Monitor> {
    vec![Monitor { name: "DP-1".into(), x: 215, scale: Some(1.399999976158142), primary: true, ..Monitor::default() }, Monitor { name: "HDMI-A-1".into(), x: 2958, scale: Some(1.0), wallpaper: "/purple.png".into(), ..Monitor::default() }]
}

#[test]
fn the_first_look_is_what_is_live_and_nothing_to_apply() {
    let mut cfg = file();
    let (mut st, fake) = page(two_heads(), &cfg);
    assert_eq!((st.link.clone(), st.draft.len(), st.dirty(), st.idle()), (Link::Ready, 2, false, true));
    assert_eq!((st.draft[0].scale, st.draft[0].primary, st.draft[1].x), (1.4, true, 2743));
    assert_eq!((st.label(0), st.label(1)), ("1 · DP-1".to_owned(), "2 · HDMI-A-1".to_owned()));
    // Apply with nothing changed asks nothing and writes nothing.
    assert!(!st.apply(&mut cfg));
    assert!(fake.asked().is_empty() && cfg == file());
}

#[test]
fn a_change_goes_on_trial_and_is_kept_into_the_file() {
    let mut cfg = file();
    let (mut st, fake) = page(two_heads(), &cfg);
    st.draft[0].scale = 1.0;
    st.edited();
    assert!(st.dirty());
    assert_eq!(st.draft[1].x, 3840, "the neighbour moved out with the wider desktop");
    assert!(!st.apply(&mut cfg), "nothing is saved before it is kept");
    assert!(!st.idle() && cfg == file());
    let asked = fake.asked();
    assert_eq!((asked.len(), asked[0][0].scale, asked[0][1].position, asked[0][1].scale), (1, Some(1.0), (3840, 0), None));

    // Nothing happens until the compositor answers; then the clock runs.
    assert_eq!(st.tick(&cfg, 1.0), None);
    fake.answer(Outcome::Succeeded);
    assert!(st.tick(&cfg, 2.0).is_some());
    assert_eq!((st.seconds_left(2.0), st.seconds_left(16.2), st.dirty()), (Some(15), Some(1), false));
    assert_eq!((st.draft[0].scale, st.draft[1].x), (1.0, 3840), "the draft is what is live now");
    assert!(cfg == file(), "on trial is not saved");

    assert!(st.keep(&mut cfg));
    assert!(st.idle() && !st.dirty());
    assert_eq!((cfg[0].x, cfg[0].scale, cfg[0].resolution.as_str(), cfg[0].refresh_rate.as_str(), cfg[0].primary), (0, Some(1.0), "3840x2160", "240000", true));
    assert_eq!((cfg[1].x, cfg[1].wallpaper.as_str()), (3840, "/purple.png"));
    assert_eq!(fake.asked().len(), 1, "keeping asks nothing more of the compositor");
    assert!(st.notice.as_ref().is_some_and(|n| n.good));
}

#[test]
fn a_trial_nobody_keeps_puts_itself_back() {
    let mut cfg = file();
    let (mut st, fake) = page(two_heads(), &cfg);
    // Another mode and another scale on DP-1, and HDMI-A-1 off.
    (st.draft[0].mode, st.draft[0].scale, st.draft[1].enabled) = (Some(3), 1.0, false);
    st.edited();
    st.apply(&mut cfg);
    fake.answer(Outcome::Succeeded);
    st.tick(&cfg, 10.0);
    assert_eq!((fake.heads()[0].current, fake.heads()[1].enabled), (Some(3), false));

    // The page is not even looked at: the clock runs all the same.
    assert!(st.tick(&cfg, 24.9).is_some());
    assert_eq!(fake.asked().len(), 1);
    st.tick(&cfg, 25.0);
    let back = fake.asked()[1].clone();
    assert_eq!(back[0], Change { name: "DP-1".into(), enabled: true, mode: Some(0), position: (0, 0), scale: Some(1.4) });
    assert_eq!(back[1], Change { name: "HDMI-A-1".into(), enabled: true, mode: None, position: (2743, 0), scale: Some(1.0) });
    assert!(!st.idle() && st.seconds_left(25.0).is_none());

    fake.answer(Outcome::Succeeded);
    assert_eq!(st.tick(&cfg, 25.1), None);
    assert!(st.idle() && !st.dirty() && cfg == file(), "back as it was, and never written");
    assert_eq!((st.draft[0].mode, st.draft[0].scale, st.draft[1].enabled), (Some(0), 1.4, true));
    assert_eq!(st.notice.as_ref().map(|n| n.text.as_str()), Some("Back to how it was."));
}

#[test]
fn going_back_is_a_press_too() {
    let mut cfg = file();
    let (mut st, fake) = page(two_heads(), &cfg);
    st.draft[1].x = -1920;
    st.edited();
    assert_eq!((st.draft[0].x, st.draft[1].x), (1920, 0), "dragged to the left: the corner is the origin again");
    st.apply(&mut cfg);
    fake.answer(Outcome::Succeeded);
    st.tick(&cfg, 1.0);
    st.go_back();
    assert_eq!(fake.asked()[1].iter().map(|c| c.position).collect::<Vec<_>>(), [(0, 0), (2743, 0)]);
    // Keep does nothing once it is on its way back.
    assert!(!st.keep(&mut cfg) && cfg == file());
}

#[test]
fn what_only_the_file_knows_is_saved_without_a_trial() {
    let mut cfg = file();
    let (mut st, fake) = page(two_heads(), &cfg);
    (st.draft[0].primary, st.draft[1].primary, st.draft[1].vrr) = (false, true, true);
    st.edited();
    assert!(st.apply(&mut cfg), "the file is to be saved");
    assert!(fake.asked().is_empty() && st.idle() && !st.dirty());
    assert_eq!((cfg[0].primary, cfg[1].primary, cfg[1].vrr), (false, true, true));
    // Nothing else of the entries was touched: the file's own numbers stay.
    assert_eq!((cfg[0].x, cfg[0].scale, cfg[1].x), (215, Some(1.399999976158142), 2958));
}

#[test]
fn a_refusal_keeps_the_edit_and_says_why() {
    let mut cfg = file();
    let (mut st, fake) = page(two_heads(), &cfg);
    st.draft[0].scale = 2.0;
    st.edited();
    fake.refuse("the last change is still being made");
    st.apply(&mut cfg);
    assert!(st.idle() && st.dirty());
    assert!(st.notice.as_ref().is_some_and(|n| !n.good && n.text.contains("still being made")));

    // Sent, and the compositor can't: the edit is still there to change.
    st.apply(&mut cfg);
    fake.answer(Outcome::Failed);
    st.tick(&cfg, 1.0);
    assert!(st.idle() && st.dirty() && st.seconds_left(1.0).is_none() && cfg == file());
    assert_eq!(st.draft[0].scale, 2.0);
    // The monitors changed under it: the page starts over from what is live.
    st.apply(&mut cfg);
    fake.answer(Outcome::Cancelled);
    st.tick(&cfg, 2.0);
    assert!(st.idle() && !st.dirty());
    st.reset();
    assert!(st.notice.is_none());
}

#[test]
fn a_monitor_that_goes_takes_its_edits_with_it() {
    let cfg = file();
    let fake = Fake::with(two_heads());
    let mut st = MonitorsState { source: Some(Box::new(fake.clone())), ..MonitorsState::default() };
    st.tick(&cfg, 0.0);
    st.selected = 1;
    st.draft[1].scale = 1.5;
    st.edited();
    // HDMI-A-1 is unplugged.
    fake.plug(two_heads()[..1].to_vec());
    st.tick(&cfg, 1.0);
    assert_eq!((st.draft.len(), st.selected, st.dirty()), (1, 0, false));
    // The compositor going away is told, and stops nothing else.
    fake.lose("the compositor hung up");
    st.tick(&cfg, 2.0);
    assert_eq!(st.link, Link::Lost("the compositor hung up".to_owned()));
}

#[test]
fn without_a_compositor_there_is_nothing_to_do() {
    let mut cfg = file();
    let mut st = MonitorsState::default();
    assert_eq!(st.tick(&cfg, 0.0), None);
    st.connect();
    assert!(st.source.is_none(), "no waker, no connection: a headless run reaches no compositor");
    assert!(!st.apply(&mut cfg) && !st.keep(&mut cfg) && !st.dirty());
    st.go_back();
    assert!(st.idle());
}
