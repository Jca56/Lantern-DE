use super::*;
use std::os::unix::fs::symlink;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fox-links-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn nothing_slow(_: &Path) -> bool {
    false
}

#[test]
fn links_are_followed_to_the_real_path() {
    let top = scratch("follow");
    std::fs::create_dir_all(top.join("real/inner")).unwrap();
    std::fs::write(top.join("real/inner/file"), "x").unwrap();
    symlink(top.join("real"), top.join("abs")).unwrap();
    symlink("real/inner", top.join("rel")).unwrap();
    symlink("../abs/inner/file", top.join("real/chain")).unwrap();
    symlink("nowhere", top.join("dangling")).unwrap();
    symlink("loop-b", top.join("loop-a")).unwrap();
    symlink("loop-a", top.join("loop-b")).unwrap();

    let local = |p: &str| Resolved::Local(top.join(p));
    assert_eq!(resolve(&top.join("abs"), &nothing_slow), local("real"));
    assert_eq!(
        resolve(&top.join("rel"), &nothing_slow),
        local("real/inner")
    );
    // Through a link in the middle of the path, and `..` after one.
    assert_eq!(
        resolve(&top.join("abs/inner/file"), &nothing_slow),
        local("real/inner/file")
    );
    assert_eq!(
        resolve(&top.join("real/chain"), &nothing_slow),
        local("real/inner/file")
    );
    assert_eq!(
        resolve(&top.join("dangling"), &nothing_slow),
        Resolved::Broken
    );
    assert_eq!(
        resolve(&top.join("loop-a"), &nothing_slow),
        Resolved::Broken
    );
    assert_eq!(
        resolve(Path::new("relative"), &nothing_slow),
        Resolved::Broken
    );
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn nothing_on_a_slow_mount_is_touched() {
    let top = scratch("slow");
    // "phone" stands in for a mount point. It does not even exist: if
    // the resolver looked at it, the answer would be Broken.
    let phone = top.join("phone");
    let on_phone = |p: &Path| p.starts_with(&phone);
    symlink(phone.join("DCIM/Camera"), top.join("camera")).unwrap();
    symlink(&phone, top.join("whole")).unwrap();
    symlink("camera/2026", top.join("hop")).unwrap();
    std::fs::create_dir(top.join("local")).unwrap();

    assert_eq!(
        resolve(&top.join("camera"), &on_phone),
        Resolved::Slow(phone.join("DCIM/Camera"))
    );
    assert_eq!(
        resolve(&top.join("whole"), &on_phone),
        Resolved::Slow(phone.clone())
    );
    // A local link to a local link that leads there.
    assert_eq!(
        resolve(&top.join("hop"), &on_phone),
        Resolved::Slow(phone.join("DCIM/Camera/2026"))
    );
    // A path below such a link.
    assert_eq!(
        resolve(&top.join("camera/IMG_1.jpg"), &on_phone),
        Resolved::Slow(phone.join("DCIM/Camera/IMG_1.jpg"))
    );
    assert_eq!(
        resolve(&top.join("local"), &on_phone),
        Resolved::Local(top.join("local"))
    );
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn a_link_onto_a_slow_mount_is_remembered_and_its_folder_treated_as_slow() {
    let top = scratch("registry");
    let phone = top.join("phone");
    let roots = vec![phone.clone()];
    let link = top.join("to-phone");
    let mut looker = Looker::with_roots(roots.clone());
    symlink(&phone, &link).unwrap();
    // The mount point itself is a folder, with no question asked.
    let seen = looker.look(&link).expect("known at once");
    assert!(seen.is_dir);
    assert!(leads_to_slow(&link, &roots));
    assert!(leads_to_slow(&link.join("DCIM/x.jpg"), &roots));
    assert!(!leads_to_slow(&top.join("other"), &roots));
    // The phone is unplugged: the same link leads nowhere slow.
    assert!(!leads_to_slow(&link, &[]));

    // The next listing of the folder no longer finds the link.
    std::fs::remove_file(&link).unwrap();
    looker.finish(&top);
    let relisted = Looker::with_roots(roots.clone());
    relisted.finish(&top);
    assert!(!leads_to_slow(&link, &roots));
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn a_target_inside_a_slow_mount_is_asked_about_on_another_thread() {
    let top = scratch("probe");
    // A real folder standing in for the phone, so the probe gets an
    // answer; to the looker it is a slow mount.
    let phone = top.join("phone");
    std::fs::create_dir_all(phone.join("DCIM")).unwrap();
    std::fs::write(phone.join("note.txt"), "abc").unwrap();
    symlink(phone.join("DCIM"), top.join("camera")).unwrap();
    symlink(phone.join("note.txt"), top.join("note")).unwrap();
    symlink(phone.join("gone"), top.join("gone")).unwrap();
    let roots = vec![phone.clone()];

    // The first listing does not wait: all three show as plain links.
    let mut first = Looker::with_roots(roots.clone());
    for name in ["camera", "note", "gone"] {
        assert_eq!(first.look(&top.join(name)), None, "{name}");
    }
    first.finish(&top);

    // The answers arrive, and the listing is asked for again.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let camera = loop {
        let mut again = Looker::with_roots(roots.clone());
        let camera = again.look(&top.join("camera"));
        let note = again.look(&top.join("note"));
        again.look(&top.join("gone"));
        again.finish(&top);
        let gone_answered = links()
            .iter()
            .any(|l| l.link == top.join("gone") && l.probe == Probe::Known(None));
        if let (Some(camera), Some(note), true) = (camera, note, gone_answered) {
            assert_eq!(note.size, 3);
            break camera;
        }
        assert!(std::time::Instant::now() < deadline, "never probed");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert!(camera.is_dir);
    assert!(take_changed(), "the folder showing them is listed again");
    // A link to nothing stays a link to nothing, and is not asked
    // about over and over.
    let mut again = Looker::with_roots(roots.clone());
    assert_eq!(again.look(&top.join("gone")), None);
    assert!(links()
        .iter()
        .any(|l| l.link == top.join("gone") && l.probe == Probe::Known(None)));
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn without_slow_mounts_a_link_is_simply_followed() {
    let top = scratch("plain");
    std::fs::create_dir(top.join("dir")).unwrap();
    std::fs::write(top.join("file"), "12345").unwrap();
    symlink("dir", top.join("to-dir")).unwrap();
    symlink("file", top.join("to-file")).unwrap();
    symlink("gone", top.join("to-nothing")).unwrap();
    let mut looker = Looker::with_roots(Vec::new());
    assert!(looker.look(&top.join("to-dir")).unwrap().is_dir);
    let file = looker.look(&top.join("to-file")).unwrap();
    assert!(!file.is_dir);
    assert_eq!(file.size, 5);
    assert_eq!(looker.look(&top.join("to-nothing")), None);
    let _ = std::fs::remove_dir_all(&top);
}

#[test]
fn a_listing_shows_a_link_as_what_it_points_at() {
    use crate::fs::{read_directory, sort_entries, SortBy, SortDir};
    let top = scratch("listing");
    let dir = top.join("shown");
    std::fs::create_dir_all(dir.join("folder")).unwrap();
    std::fs::create_dir_all(top.join("elsewhere/games")).unwrap();
    std::fs::write(top.join("elsewhere/chart.png"), vec![7u8; 4321]).unwrap();
    std::fs::write(dir.join("plain.txt"), "hello").unwrap();
    symlink(top.join("elsewhere/games"), dir.join("Games")).unwrap();
    symlink("../elsewhere/chart.png", dir.join("chart.png")).unwrap();
    symlink("missing", dir.join("broken")).unwrap();

    let mut entries = read_directory(&dir, false).expect("listed");
    sort_entries(&mut entries, SortBy::Name, SortDir::Asc);
    let by_name = |name: &str| entries.iter().find(|e| e.name == name).unwrap();

    // The link to a folder is a folder: entered, sorted with folders,
    // kept by a picker's filter. And still known to be a link.
    let games = by_name("Games");
    assert!(games.is_dir && games.is_symlink);
    assert_eq!(games.path, dir.join("Games"), "its path stays the link's");
    assert_eq!(games.extension(), "");
    let order: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        order,
        ["folder", "Games", "broken", "chart.png", "plain.txt"]
    );

    // The link to a file carries the file's size and date, which is
    // what its thumbnail and its info are keyed on.
    let chart = by_name("chart.png");
    assert!(chart.is_symlink && !chart.is_dir);
    assert_eq!(chart.size, 4321);
    let real = std::fs::metadata(top.join("elsewhere/chart.png")).unwrap();
    assert_eq!(chart.modified, real.modified().ok());

    // A dangling link is a file, described by the link itself.
    let broken = by_name("broken");
    assert!(broken.is_symlink && !broken.is_dir);
    assert_eq!(broken.size, "missing".len() as u64);

    assert!(!by_name("folder").is_symlink);
    assert!(!by_name("plain.txt").is_symlink);

    // The target is rewritten: the next listing has the new stamp, so
    // nothing cached under the old one is served for it.
    std::fs::write(top.join("elsewhere/chart.png"), vec![7u8; 99]).unwrap();
    let again = read_directory(&dir, false).unwrap();
    let chart = again.iter().find(|e| e.name == "chart.png").unwrap();
    assert_eq!(chart.size, 99);
    let _ = std::fs::remove_dir_all(&top);
}
