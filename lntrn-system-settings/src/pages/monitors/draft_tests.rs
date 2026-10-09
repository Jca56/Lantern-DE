use super::*;

fn mode(width: i32, height: i32, refresh: i32) -> Mode {
    Mode { width, height, refresh, preferred: false }
}

/// A 4K monitor the way the compositor lists one: the fast mode first,
/// the one the panel prefers further down, a twin of the same rate.
fn dp(enabled: bool, x: i32, scale: f64) -> Head {
    let modes = vec![mode(3840, 2160, 240_000), Mode { preferred: true, ..mode(3840, 2160, 60_000) }, mode(3840, 2160, 60_000), mode(2560, 1440, 144_000), mode(1920, 1080, 240_000), mode(1920, 1080, 60_000)];
    Head { name: "DP-1".into(), enabled, position: (x, 0), scale, physical_mm: (590, 330), modes, current: Some(0) }
}

fn hdmi(enabled: bool, x: i32) -> Head {
    Head { name: "HDMI-A-1".into(), enabled, position: (x, 0), scale: 1.0, physical_mm: (0, 0), modes: vec![mode(1920, 1080, 100_000), mode(1280, 720, 60_000)], current: Some(0) }
}

/// The file as it stands on the desk: both monitors, in coordinates
/// that start 215 pixels in.
fn file() -> Vec<Monitor> {
    vec![
        Monitor { name: "HDMI-A-1".into(), x: 2961, scale: Some(1.0), resolution: "1920x1080".into(), refresh_rate: "100000".into(), wallpaper: "/purple.png".into(), ..Monitor::default() },
        Monitor { name: "DP-1".into(), x: 215, scale: Some(1.399999976158142), resolution: "3840x2160".into(), refresh_rate: "240000".into(), primary: true, ..Monitor::default() },
    ]
}

#[test]
fn a_scale_is_the_step_it_looks_like_through_the_wire() {
    // 1.4 arrives as 358 256ths.
    assert_eq!(nominal(358.0 / 256.0), 1.4);
    assert_eq!(nominal(1.0), 1.0);
    assert_eq!(nominal(1.25), 1.25);
    assert_eq!(nominal((1.15f64 * 256.0).round() / 256.0), 1.15);
    // One set by hand to something else stays what it is.
    assert_eq!(nominal(340.0 / 256.0), 340.0 / 256.0);
}

#[test]
fn what_is_live_comes_from_the_compositor_and_the_file() {
    let heads = [dp(true, 0, 358.0 / 256.0), hdmi(true, 2746)];
    let live = live_with(&heads);
    assert_eq!(live[0], Draft { name: "DP-1".into(), enabled: true, mode: Some(0), scale: 1.4, x: 0, y: 0, primary: true, vrr: false });
    assert_eq!((live[1].x, live[1].scale, live[1].primary), (2746, 1.0, false));
    assert_eq!(tile(&heads[0], &live[0]), Tile { x: 0, y: 0, w: 2743, h: 1543 });

    // One that is off is where the file has it, in today's coordinates:
    // the file starts 215 in, the desktop at nothing.
    let heads = [dp(true, 0, 358.0 / 256.0), hdmi(false, 0)];
    let off = &live_with(&heads)[1];
    assert_eq!((off.enabled, off.x, off.scale, off.mode), (false, 2746, 1.0, Some(0)));
    // A monitor the file has never heard of is plain, and one of those
    // that is off waits off the right end, to come on there.
    let unknown = base(&heads, &[]);
    assert_eq!((unknown[0].scale, unknown[0].primary, unknown[0].vrr), (1.4, false, false));
    assert_eq!((unknown[1].x, unknown[1].y, unknown[1].scale), (2743, 0, 1.0));
    let mut draft = unknown.clone();
    draft[1].enabled = true;
    refit(&mut draft, &heads, &unknown);
    assert_eq!((draft[0].x, draft[1].x), (0, 2743));
}

/// What is live with `heads` plugged in and the file as it stands.
fn live_with(heads: &[Head]) -> Vec<Draft> {
    base(heads, &file())
}

#[test]
fn an_edit_asks_only_for_what_it_changes() {
    let heads = [dp(true, 0, 358.0 / 256.0), hdmi(true, 2743)];
    let base = live_with(&heads);
    let mut draft = base.clone();
    assert!(!moves_the_desktop(&draft, &base));
    // Only the file knows which is the main one.
    draft[1].primary = true;
    assert!(draft != base && !moves_the_desktop(&draft, &base));

    // A smaller scale makes DP-1 wider: its neighbour moves out with it.
    draft[0].scale = 1.0;
    refit(&mut draft, &heads, &base);
    assert_eq!((draft[1].x, draft[1].y), (3840, 0));
    assert!(moves_the_desktop(&draft, &base));
    let asked = changes(&draft, &base);
    assert_eq!(asked[0], Change { name: "DP-1".into(), enabled: true, mode: None, position: (0, 0), scale: Some(1.0) });
    assert_eq!(asked[1], Change { name: "HDMI-A-1".into(), enabled: true, mode: None, position: (3840, 0), scale: None });

    // A new mode is named; the same one is left alone, so nothing blinks.
    let mut draft = base.clone();
    draft[0].mode = Some(4);
    refit(&mut draft, &heads, &base);
    assert_eq!(tile(&heads[0], &draft[0]).w, 1371);
    assert_eq!(draft[1].x, 1371);
    assert_eq!(changes(&draft, &base)[0].mode, Some(4));
}

#[test]
fn a_monitor_coming_on_is_told_everything() {
    let heads = [dp(true, 0, 358.0 / 256.0), hdmi(false, 0)];
    let base = live_with(&heads);
    let mut draft = base.clone();
    draft[1].enabled = true;
    refit(&mut draft, &heads, &base);
    // Where the file had it is three pixels off DP-1's edge: fitted in.
    assert_eq!(draft[1].x, 2743);
    let asked = changes(&draft, &base);
    assert_eq!(asked[1], Change { name: "HDMI-A-1".into(), enabled: true, mode: None, position: (2743, 0), scale: Some(1.0) });
    assert!(moves_the_desktop(&draft, &base));

    // Going off, it is only named, and the one left has the desktop.
    let heads = [dp(true, 1920, 358.0 / 256.0), hdmi(true, 0)];
    let base = live_with(&heads);
    let mut draft = base.clone();
    draft[1].enabled = false;
    refit(&mut draft, &heads, &base);
    assert_eq!((draft[0].x, draft[0].y), (0, 0), "the corner is back at the origin");
    let asked = changes(&draft, &base);
    assert_eq!((asked[1].enabled, asked[1].scale, asked[1].mode), (false, None, None));
}

#[test]
fn modes_make_two_lists() {
    let modes = dp(true, 0, 1.0).modes;
    assert_eq!(resolutions(&modes), [(3840, 2160), (2560, 1440), (1920, 1080)]);
    // Twins of a rate show once, as the first of them.
    assert_eq!(rates(&modes, (3840, 2160)), [(240_000, 0), (60_000, 1)]);
    assert_eq!(rates(&modes, (1920, 1080)), [(240_000, 4), (60_000, 5)]);
    assert!(rates(&modes, (800, 600)).is_empty());
    // A new resolution keeps the rate when it can, else takes the fastest.
    assert_eq!(mode_at(&modes, (1920, 1080), 60_000), Some(5));
    assert_eq!(mode_at(&modes, (2560, 1440), 240_000), Some(3));
    assert_eq!(mode_at(&modes, (800, 600), 60_000), None);
    assert_eq!(resolution_label(&modes, (3840, 2160)), "3840 × 2160  (native)");
    assert_eq!(resolution_label(&modes, (1920, 1080)), "1920 × 1080");
    assert_eq!((rate_label(240_000), rate_label(59_940)), ("240 Hz".to_owned(), "59.94 Hz".to_owned()));
    assert_eq!((inches((590, 330)), inches((0, 0))), (Some(27), None));
}

#[test]
fn a_kept_setup_goes_into_the_file() {
    // DP-1 alone is plugged in, now at 1.25 and 1080p; HDMI-A-1 is at home.
    let heads = [Head { current: Some(4), ..dp(true, 0, 1.25) }];
    let mut cfg = file();
    let live = base(&heads, &cfg);
    let wanted = vec![Draft { vrr: true, ..live[0].clone() }];
    keep(&mut cfg, &heads, &live, &wanted);
    let kept = &cfg[1];
    assert_eq!((kept.x, kept.y, kept.scale, kept.resolution.as_str(), kept.refresh_rate.as_str()), (0, 0, Some(1.25), "1920x1080", "240000"));
    assert_eq!((kept.primary, kept.vrr, kept.enabled), (true, true, true));
    // The one that isn't here moved along with it and kept all else.
    let away = &cfg[0];
    assert_eq!((away.x, away.scale, away.wallpaper.as_str(), away.resolution.as_str()), (2746, Some(1.0), "/purple.png", "1920x1080"));

    // One switched off is marked off and stays where it was beside the
    // other; a monitor the file never had gets an entry.
    let heads = [dp(true, 0, 1.0), hdmi(false, 0), Head { name: "eDP-1".into(), ..hdmi(true, 3840) }];
    let live = base(&heads, &cfg);
    keep(&mut cfg, &heads, &live, &live.clone());
    assert_eq!((cfg[0].enabled, cfg[0].x), (false, 2746));
    assert_eq!((cfg[2].name.as_str(), cfg[2].x, cfg[2].scale, cfg[2].refresh_rate.as_str()), ("eDP-1", 3840, Some(1.0), "100000"));
}
