//! The engine against a compositor played by hand: its side of the
//! conversation is written with the same [`Request`] the engine writes
//! its own with (an event is laid out like a request), and what the
//! engine says back is read with the [`Reader`].

use super::*;
use crate::outputs::wire::Reader;

/// Ids the compositor gives its own objects start up here.
const HEAD_A: u32 = 0xFF00_0000;
const HEAD_B: u32 = 0xFF00_0100;
const REGISTRY: u32 = 2;
const SYNC: u32 = 3;
const MANAGER: u32 = 4;

/// Feed `events` in, and hand back the notes that weren't `Nothing` and
/// the messages the engine wrote.
fn feed(e: &mut Engine, events: Vec<Request>) -> (Vec<Note>, Vec<Message>) {
    let mut bytes = Vec::new();
    for ev in events {
        ev.finish(&mut bytes);
    }
    let mut r = Reader::default();
    r.push(&bytes);
    let (mut notes, mut out) = (Vec::new(), Vec::new());
    while let Some(m) = r.next().unwrap() {
        let note = e.event(&m, &mut out);
        if note != Note::Nothing {
            notes.push(note);
        }
    }
    (notes, said(&out))
}

fn said(bytes: &[u8]) -> Vec<Message> {
    let mut r = Reader::default();
    r.push(bytes);
    std::iter::from_fn(|| r.next().unwrap()).collect()
}

fn words(m: &Message) -> Vec<u32> {
    m.body.chunks(4).map(|c| u32::from_ne_bytes(c.try_into().unwrap())).collect()
}

/// A head called `name` with two modes (the second current), as the
/// compositor announces one.
fn head(id: u32, name: &str, enabled: bool, x: i32, scale: f64) -> Vec<Request> {
    let (m0, m1) = (id + 1, id + 2);
    let mut evs = vec![
        Request::new(MANAGER, op::MANAGER_HEAD).uint(id),
        Request::new(id, op::HEAD_NAME).string(name),
        Request::new(id, op::HEAD_ENABLED).int(enabled as i32),
        Request::new(id, op::HEAD_PHYSICAL_SIZE).int(600).int(340),
        Request::new(id, op::HEAD_MODE).uint(m0),
        Request::new(m0, op::MODE_SIZE).int(1920).int(1080),
        Request::new(m0, op::MODE_REFRESH).int(60_000),
        Request::new(id, op::HEAD_MODE).uint(m1),
        Request::new(m1, op::MODE_SIZE).int(3840).int(2160),
        Request::new(m1, op::MODE_REFRESH).int(240_000),
        Request::new(m1, op::MODE_PREFERRED),
        Request::new(id, op::HEAD_CURRENT_MODE).uint(m1),
    ];
    if enabled {
        evs.push(Request::new(id, op::HEAD_POSITION).int(x).int(0));
        evs.push(Request::new(id, op::HEAD_SCALE).fixed(scale));
    }
    evs
}

/// An engine that has bound the manager and been told of two monitors,
/// `HDMI-A-1` (announced first) and `DP-1`, settled at serial 7.
fn two_monitors(hdmi_on: bool) -> Engine {
    let mut hello = Vec::new();
    let mut e = Engine::new(&mut hello);
    let hello = said(&hello);
    assert_eq!((hello[0].object, hello[0].opcode, words(&hello[0])), (DISPLAY, op::DISPLAY_GET_REGISTRY, vec![REGISTRY]));
    assert_eq!((hello[1].object, hello[1].opcode, words(&hello[1])), (DISPLAY, op::DISPLAY_SYNC, vec![SYNC]));

    let (notes, out) = feed(&mut e, vec![
        Request::new(REGISTRY, op::REGISTRY_GLOBAL).uint(1).string("wl_compositor").uint(6),
        // Offered newer than we know: bound at the version we do.
        Request::new(REGISTRY, op::REGISTRY_GLOBAL).uint(31).string("zwlr_output_manager_v1").uint(9),
        Request::new(SYNC, 0).uint(0),
    ]);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!((out.len(), out[0].object, out[0].opcode), (1, REGISTRY, op::REGISTRY_BIND));
    let mut bind = Args::new(&out[0].body);
    assert_eq!((bind.uint(), bind.string().as_deref(), bind.uint(), bind.uint()), (Some(31), Some("zwlr_output_manager_v1"), Some(MANAGER_VERSION), Some(MANAGER)));
    assert!(!e.ready(), "nothing can be asked before the compositor has settled");

    let mut evs = head(HEAD_A, "HDMI-A-1", hdmi_on, 2743, 1.0);
    evs.extend(head(HEAD_B, "DP-1", true, 0, 1.4));
    evs.push(Request::new(MANAGER, op::MANAGER_DONE).uint(7));
    let (notes, out) = feed(&mut e, evs);
    assert_eq!((notes, out.len()), (vec![Note::Settled], 0));
    assert!(e.ready());
    e
}

#[test]
fn monitors_are_told_whole_and_in_the_order_of_their_names() {
    let e = two_monitors(false);
    let heads = e.heads();
    assert_eq!(heads.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["DP-1", "HDMI-A-1"]);
    let dp = &heads[0];
    assert_eq!((dp.enabled, dp.position, dp.physical_mm, dp.current), (true, (0, 0), (600, 340), Some(1)));
    // 1.4 comes over the wire as the nearest 256th.
    assert!((dp.scale - 1.4).abs() < 1.0 / 256.0 && dp.scale != 1.4);
    assert_eq!(dp.modes, vec![Mode { width: 1920, height: 1080, refresh: 60_000, preferred: false }, Mode { width: 3840, height: 2160, refresh: 240_000, preferred: true }]);
    // One that is off says neither where it is nor how it is scaled.
    assert_eq!((heads[1].enabled, heads[1].position, heads[1].scale), (false, (0, 0), 1.0));
}

#[test]
fn a_compositor_without_the_manager_is_found_out_by_the_sync() {
    let mut hello = Vec::new();
    let mut e = Engine::new(&mut hello);
    let (notes, out) = feed(&mut e, vec![Request::new(REGISTRY, op::REGISTRY_GLOBAL).uint(1).string("wl_compositor").uint(6), Request::new(SYNC, 0).uint(0)]);
    assert_eq!((notes, out.len()), (vec![Note::Missing], 0));
    // Its complaint about an object of ours ends the connection.
    let (notes, _) = feed(&mut e, vec![Request::new(DISPLAY, op::DISPLAY_ERROR).uint(4).uint(1).string("invalid new id")]);
    assert!(matches!(&notes[0], Note::Broken(why) if why.contains("invalid new id")));
}

#[test]
fn a_setup_names_every_head_and_only_what_changes() {
    let mut e = two_monitors(true);
    let mut out = Vec::new();
    // DP-1 goes to its first mode at 1.25; nothing is said of HDMI-A-1.
    e.configure(&[Change { name: "DP-1".into(), enabled: true, mode: Some(0), position: (0, 0), scale: Some(1.25) }], &mut out).unwrap();
    assert!(!e.ready(), "one at a time");
    let out = said(&out);
    let (config, row_hdmi, row_dp) = (MANAGER + 1, MANAGER + 2, MANAGER + 3);
    let got: Vec<(u32, u16, Vec<u32>)> = out.iter().map(|m| (m.object, m.opcode, words(m))).collect();
    assert_eq!(
        got,
        vec![
            (MANAGER, op::MANAGER_CREATE_CONFIGURATION, vec![config, 7]),
            // In the compositor's order, each new id the next one along.
            (config, op::CONFIG_ENABLE_HEAD, vec![row_hdmi, HEAD_A]),
            (config, op::CONFIG_ENABLE_HEAD, vec![row_dp, HEAD_B]),
            (row_dp, op::CONFIG_HEAD_SET_MODE, vec![HEAD_B + 1]),
            (row_dp, op::CONFIG_HEAD_SET_POSITION, vec![0, 0]),
            (row_dp, op::CONFIG_HEAD_SET_SCALE, vec![320]),
            (config, op::CONFIG_APPLY, vec![]),
        ]
    );
    // A second one waits for the first to be answered.
    assert!(e.configure(&[], &mut Vec::new()).is_err());

    let (notes, out) = feed(&mut e, vec![Request::new(config, op::CONFIG_SUCCEEDED)]);
    assert_eq!(notes, vec![Note::Outcome(Outcome::Succeeded)]);
    assert_eq!((out[0].object, out[0].opcode), (config, op::CONFIG_DESTROY));
    assert!(e.ready());
}

#[test]
fn a_monitor_is_switched_off_but_never_the_last() {
    let mut e = two_monitors(true);
    let off = |name: &str| Change { name: name.into(), enabled: false, mode: None, position: (0, 0), scale: None };
    let mut out = Vec::new();
    e.configure(&[off("HDMI-A-1")], &mut out).unwrap();
    let out = said(&out);
    assert_eq!((out[1].opcode, words(&out[1])), (op::CONFIG_DISABLE_HEAD, vec![HEAD_A]));
    assert_eq!((out[2].opcode, words(&out[2])[1]), (op::CONFIG_ENABLE_HEAD, HEAD_B));
    let (notes, _) = feed(&mut e, vec![Request::new(MANAGER + 1, op::CONFIG_FAILED)]);
    assert_eq!(notes, vec![Note::Outcome(Outcome::Failed)]);

    // Both off is refused before anything is written.
    let mut out = Vec::new();
    assert!(e.configure(&[off("HDMI-A-1"), off("DP-1")], &mut out).unwrap_err().contains("every monitor"));
    // So is the only one that is on, with the other off already.
    let mut e = two_monitors(false);
    assert!(e.configure(&[off("DP-1")], &mut out).is_err());
    assert!(e.configure(&[Change { name: "DP-9".into(), ..off("x") }], &mut out).unwrap_err().contains("DP-9"));
    assert!(e.configure(&[Change { mode: Some(2), enabled: true, ..off("DP-1") }], &mut out).is_err(), "a mode it hasn't got");
    assert!(e.configure(&[Change { scale: Some(0.0), enabled: true, ..off("DP-1") }], &mut out).is_err());
    assert!(out.is_empty() && e.ready());
}

#[test]
fn a_monitor_that_goes_makes_the_rest_stale() {
    let mut e = two_monitors(true);
    // Its modes go first, then the head.
    let (notes, out) = feed(&mut e, vec![Request::new(HEAD_A + 1, op::MODE_FINISHED), Request::new(HEAD_A + 2, op::MODE_FINISHED), Request::new(HEAD_A, op::HEAD_FINISHED)]);
    assert_eq!(notes, vec![Note::Stale]);
    assert_eq!(out.iter().map(|m| (m.object, m.opcode)).collect::<Vec<_>>(), [(HEAD_A + 1, op::MODE_RELEASE), (HEAD_A + 2, op::MODE_RELEASE), (HEAD_A, op::HEAD_RELEASE)]);
    assert_eq!(e.heads().len(), 1);
    // Something said to what we let go of is nothing.
    let (notes, out) = feed(&mut e, vec![Request::new(HEAD_A, op::HEAD_SCALE).fixed(2.0), Request::new(HEAD_A + 1, op::MODE_SIZE).int(1).int(1)]);
    assert!(notes.is_empty() && out.is_empty());
}

#[test]
fn what_changes_later_shows_at_the_next_settle() {
    let mut e = two_monitors(true);
    let (notes, _) = feed(&mut e, vec![
        Request::new(HEAD_B, op::HEAD_SCALE).fixed(1.5),
        Request::new(HEAD_B, op::HEAD_POSITION).int(1920).int(40),
        Request::new(HEAD_A, op::HEAD_ENABLED).int(0),
        Request::new(HEAD_B, op::HEAD_CURRENT_MODE).uint(HEAD_B + 1),
        Request::new(MANAGER, op::MANAGER_DONE).uint(8),
    ]);
    assert_eq!(notes, vec![Note::Settled]);
    let heads = e.heads();
    assert_eq!((heads[0].scale, heads[0].position, heads[0].current, heads[1].enabled), (1.5, (1920, 40), Some(0), false));
    // The new serial is the one quoted from now on.
    let mut out = Vec::new();
    e.configure(&[], &mut out).unwrap();
    assert_eq!(words(&said(&out)[0])[1], 8);
}
