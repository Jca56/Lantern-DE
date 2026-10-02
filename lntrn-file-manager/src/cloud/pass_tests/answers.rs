// The answer a dialog gives to held deletions names the set the dialog
// showed (`SyncHandle::answer_held`): an answer for a set that is no longer
// the held one must do nothing. (Harness: engine.rs.)

use super::cloud;
use super::engine::{fast, machine, start, synced, wait_for, MS};

#[test]
fn an_answer_counts_only_for_the_set_the_dialog_showed() {
    let cloud = cloud("alice");
    let (_dir, places) = machine("answer-held", &cloud, 12);
    let h = start(&cloud, &places, fast());
    wait_for("the first pass", || cloud.live().len() == 12 && synced(&h));

    std::fs::remove_dir_all(places.root.join("docs")).unwrap();
    wait_for("the hold", || h.held_deletions().is_some());
    let held = h.held_deletions().unwrap();

    // A dialog that still shows an older set: its "delete" is not recorded.
    assert!(!h.answer_held("not-the-held-set", true));
    assert!(!places.answer.exists());
    std::thread::sleep(300 * MS);
    assert_eq!(cloud.tombstones(), 0);
    assert_eq!(h.held_deletions().map(|now| now.id), Some(held.id.clone()));

    // The dialog that shows the held set: its answer goes through.
    assert!(h.answer_held(&held.id, false));
    wait_for("the files to return", || {
        (0..12).all(|i| places.root.join(format!("docs/f{i:02}.txt")).exists()) && synced(&h)
    });
    assert_eq!(cloud.tombstones(), 0);
    // Nothing is held any more, so there is nothing to answer.
    assert!(!h.answer_held(&held.id, true));
}
