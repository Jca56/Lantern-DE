use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fox-settings-file-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("file-manager.json")
}

fn on_disk(path: &Path) -> store::Map {
    match store::read(path) {
        store::OnDisk::Map(map) => map,
        other => panic!("not a settings file: {other:?}"),
    }
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_first_save_writes_every_key_and_a_reload_reads_it_back() {
    let path = scratch("first");
    let mut s = Settings::load_from(&path);
    s.icon_zoom = 0.3;
    s.split_ratio = 0.537;
    s.favorites = vec!["/a".into()];
    s.save_to(&path);
    let disk = on_disk(&path);
    assert_eq!(disk.len(), Settings::default().to_map().len());
    assert_eq!(disk["sort_by"], "name");
    // Fractions are written the way they were set.
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"icon_zoom\": 0.3,"), "{text}");
    assert!(text.contains("\"split_ratio\": 0.537,"), "{text}");

    let back = Settings::load_from(&path);
    assert_eq!(back.icon_zoom, 0.3);
    assert_eq!(back.split_ratio, 0.537);
    assert_eq!(back.favorites, ["/a"]);
    // Loading and saving again changes nothing: no key counts as
    // changed just for having made the trip through the file.
    let before = std::fs::read(&path).unwrap();
    let mut back = back;
    back.save_to(&path);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    cleanup(&path);
}

#[test]
fn a_window_does_not_undo_what_another_one_saved() {
    let path = scratch("two");
    let mut first = Settings::load_from(&path);
    first.favorites = vec!["/a".into()];
    first.save_to(&path);

    // Two windows open on that file.
    let mut one = Settings::load_from(&path);
    let mut two = Settings::load_from(&path);
    // The second gets a favourite and a pinned tab, then closes.
    two.favorites.push("/two".into());
    two.pinned_tabs = vec!["/pin".into()];
    two.set_sort_by(crate::fs::SortBy::Date);
    two.save_to(&path);

    // The first only ever changed its view; its exit save writes its
    // whole state as before.
    one.view_mode = "list".into();
    one.pinned_tabs = Vec::new();
    one.save_to(&path);
    let disk = Settings::load_from(&path);
    assert_eq!(disk.view_mode, "list");
    assert_eq!(disk.sort_by, "date", "the other window's sort stays");
    assert_eq!(disk.favorites, ["/a", "/two"]);
    assert_eq!(disk.pinned_tabs, ["/pin"]);
    // And it did not quietly take the other window's list for its own.
    assert_eq!(one.favorites, ["/a"]);

    // Now the first adds a favourite of its own: both survive, and it
    // is handed the merged list to show.
    one.favorites.push("/one".into());
    one.save_to(&path);
    assert_eq!(one.favorites, ["/a", "/one", "/two"]);
    assert_eq!(Settings::load_from(&path).favorites, ["/a", "/one", "/two"]);
    // Removing one here removes exactly that one.
    one.favorites.retain(|f| f != "/a");
    one.save_to(&path);
    assert_eq!(Settings::load_from(&path).favorites, ["/one", "/two"]);
    cleanup(&path);
}

/// Two windows each pin a tab. Each rebuilds its pin list from its own tabs
/// before every save; neither may take the other's pin for removed.
#[test]
fn pins_made_in_two_windows_all_survive_both_closing() {
    let path = scratch("pins");
    Settings::load_from(&path).save_to(&path);
    let mut a = Settings::load_from(&path);
    let mut b = Settings::load_from(&path);

    b.pinned_tabs = vec!["/x".into()];
    b.save_to(&path);
    a.pinned_tabs = vec!["/y".into()];
    a.save_to(&path);
    let mut on_file = Settings::load_from(&path).pinned_tabs;
    on_file.sort();
    assert_eq!(on_file, ["/x", "/y"]);
    // A's list is still its own tabs, not the file's.
    assert_eq!(a.pinned_tabs, ["/y"]);

    // A closes: its exit save hands in its own tabs again.
    a.pinned_tabs = vec!["/y".into()];
    a.view_mode = "list".into();
    a.save_to(&path);
    // B closes with nothing changed.
    b.pinned_tabs = vec!["/x".into()];
    b.save_to(&path);
    let mut on_file = Settings::load_from(&path).pinned_tabs;
    on_file.sort();
    assert_eq!(on_file, ["/x", "/y"], "a pin made in the other window was dropped");

    // A window still removes the pins it had.
    let mut c = Settings::load_from(&path);
    c.pinned_tabs.retain(|p| p != "/x");
    c.save_to(&path);
    assert_eq!(Settings::load_from(&path).pinned_tabs, ["/y"]);
    cleanup(&path);
}

#[test]
fn a_broken_file_is_kept_aside_and_never_written_over() {
    let path = scratch("broken");
    let original = r#"{"favorites":["/precious"],"sort_by":"da"#;
    std::fs::write(&path, original).unwrap();
    let mut s = Settings::load_from(&path);
    assert!(s.favorites.is_empty(), "defaults for this session");
    assert!(!path.exists(), "moved aside at once");
    s.save_to(&path);
    s.show_hidden = true;
    s.save_to(&path);
    // The new file exists, and the old content is still there to be
    // recovered by hand.
    assert_eq!(on_disk(&path)["show_hidden"], true);
    let kept: Vec<PathBuf> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains(".broken-"))
        .collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), original);

    // Broken while a window is running (an older Fox died mid-write):
    // the same, and this window's good state is what is written.
    std::fs::write(&path, "").unwrap();
    s.favorites = vec!["/still-here".into()];
    s.save_to(&path);
    assert_eq!(Settings::load_from(&path).favorites, ["/still-here"]);
    cleanup(&path);
}

#[test]
fn a_file_that_cannot_be_read_at_all_is_left_alone() {
    let path = scratch("unreadable");
    // A folder under the file's name: every read fails, nothing can be
    // moved over it either.
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("inside"), "x").unwrap();
    let mut s = Settings::load_from(&path);
    s.show_hidden = true;
    s.save_to(&path);
    assert!(path.is_dir());
    assert!(path.join("inside").exists());
    cleanup(&path);
}

#[test]
fn one_bad_value_costs_one_setting() {
    let path = scratch("badvalue");
    std::fs::write(
        &path,
        r#"{"icon_zoom":"huge","favorites":["/a"],"sort_by":"size","from_the_future":[1]}"#,
    )
    .unwrap();
    let mut s = Settings::load_from(&path);
    assert_eq!(s.icon_zoom, Settings::default().icon_zoom);
    assert_eq!(s.favorites, ["/a"]);
    assert_eq!(s.sort_by, "size");
    // Saving something else leaves the odd value (and the key this
    // version does not know) exactly as it was.
    s.show_hidden = true;
    s.save_to(&path);
    let disk = on_disk(&path);
    assert_eq!(disk["icon_zoom"], "huge");
    assert_eq!(disk["from_the_future"], serde_json::json!([1]));
    assert_eq!(disk["show_hidden"], true);
    // Until this window sets it.
    s.icon_zoom = 0.75;
    s.save_to(&path);
    assert_eq!(on_disk(&path)["icon_zoom"], 0.75);
    cleanup(&path);
}

#[test]
fn a_file_with_keys_missing_loads_with_their_defaults() {
    let path = scratch("sparse");
    // What an older version wrote before these keys existed; the three
    // sizes used to be required and their absence reset everything.
    std::fs::write(&path, r#"{"favorites":["/a"],"show_hidden":true}"#).unwrap();
    let s = Settings::load_from(&path);
    assert!(s.show_hidden);
    assert_eq!(s.favorites, ["/a"]);
    assert_eq!(s.window_width, Settings::default().window_width);
    assert_eq!(s.split_ratio, 0.5);
    assert_eq!(s.view_mode, "grid");
    cleanup(&path);
}
