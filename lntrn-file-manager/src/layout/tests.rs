use super::*;

#[test]
fn list_zoom_moves_in_a_few_steps() {
    // Every position a slider can report lands on one of seventeen sizes.
    let mut seen: Vec<u32> = (0..=10_000)
        .map(|i| list_zoom_multiplier(i as f32 / 10_000.0).to_bits())
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 17);
    // The ends and the default are where they always were.
    assert_eq!(list_zoom_multiplier(0.0), 0.8);
    assert_eq!(list_zoom_multiplier(0.5), 1.5);
    assert_eq!(list_zoom_multiplier(1.0), 2.2);
    // Never further than half a step from the continuous value.
    for i in 0..=1000 {
        let zoom = i as f32 / 1000.0;
        let exact = 0.8 + zoom * 1.4;
        assert!((list_zoom_multiplier(zoom) - exact).abs() <= 0.7 / 16.0 + 1e-4);
    }
    // A value that is no slider position does not poison the layout.
    assert_eq!(list_zoom_multiplier(f32::NAN), 1.5);
    assert_eq!(list_zoom_multiplier(7.0), 2.2);
}

#[test]
fn row_geometry_follows_the_same_steps_as_the_fonts() {
    // Two slider positions inside one step: identical rows, so identical
    // font sizes (24, 20 and 16 times the multiplier).
    assert_eq!(list_row_h(1.25, 0.51), list_row_h(1.25, 0.52));
    assert_eq!(tree_row_h(1.25, 0.51), tree_row_h(1.25, 0.52));
    assert_eq!(list_header_h(1.25, 0.51), list_header_h(1.25, 0.52));
}

#[test]
fn each_pane_keeps_its_minimum_width() {
    let s = 1.0;
    // 1580px window: 1332px for the panes.
    for ratio in [0.0, 0.2, 0.5, 0.8, 1.0] {
        let (lx, lw, rx, rw) = split_pane_cols(1580.0, ratio, s);
        assert_eq!(lx, SIDEBAR_W);
        assert!(
            lw >= SPLIT_PANE_MIN_W && rw >= SPLIT_PANE_MIN_W,
            "ratio {ratio}: {lw} / {rw}"
        );
        assert_eq!(rx, lx + lw + SPLIT_HANDLE_W);
        assert!((rx + rw - 1580.0).abs() < 0.01);
    }
    // In the middle of the range the ratio is taken as it is.
    let (_, lw, _, _) = split_pane_cols(1580.0, 0.5, s);
    assert_eq!(lw, 666.0);
    // The minimum scales with the display.
    let (_, lw, _, rw) = split_pane_cols(2400.0, 0.0, 1.4);
    assert!(lw >= SPLIT_PANE_MIN_W * 1.4 - 0.01 && rw >= SPLIT_PANE_MIN_W * 1.4 - 0.01);
}

#[test]
fn a_window_too_narrow_for_two_minimum_panes_is_shared_evenly() {
    for ratio in [0.2, 0.5, 0.8] {
        let (_, lw, _, rw) = split_pane_cols(900.0, ratio, 1.0);
        assert!((lw - rw).abs() < 0.01, "ratio {ratio}: {lw} / {rw}");
    }
    // No room at all: empty panes, no panic.
    let (_, lw, _, rw) = split_pane_cols(200.0, 0.5, 1.0);
    assert_eq!((lw, rw), (0.0, 0.0));
}

#[test]
fn a_dragged_divider_stops_where_the_panes_do() {
    // Dragged against the sidebar: the left pane stops at its minimum.
    let ratio = split_ratio_at(0.0, 1580.0, 1.0).unwrap();
    let (_, lw, _, _) = split_pane_cols(1580.0, ratio, 1.0);
    assert!((lw - SPLIT_PANE_MIN_W).abs() < 0.01);
    // Dragged past the right edge: the right pane does.
    let ratio = split_ratio_at(5000.0, 1580.0, 1.0).unwrap();
    let (_, _, _, rw) = split_pane_cols(1580.0, ratio, 1.0);
    assert!((rw - SPLIT_PANE_MIN_W).abs() < 0.01);
    // In between, the divider is under the cursor.
    let ratio = split_ratio_at(900.0, 1580.0, 1.0).unwrap();
    let (lx, lw, _, _) = split_pane_cols(1580.0, ratio, 1.0);
    assert!((lx + lw - 900.0).abs() < 0.01);
    assert_eq!(split_ratio_at(100.0, 240.0, 1.0), None);
}
