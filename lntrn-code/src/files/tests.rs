//! The tree's tests: listing, climbing, resolving typed paths.

use super::*;

#[test]
fn a_tree_lists_climbs_and_resolves() {
    let dir = std::env::temp_dir().join(format!("lntrn-code-tree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "").unwrap();
    std::fs::write(dir.join(".hidden"), "").unwrap();
    std::fs::write(dir.join("README.md"), "").unwrap();
    let mut t = Tree::new(dir.clone());
    let names: Vec<&str> = t.entries(&dir).iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["src", "README.md"], "folders first, no dotfiles");
    t.show_hidden = true;
    t.refresh();
    assert!(t.entries(&dir).iter().any(|e| e.name == ".hidden"));
    assert_eq!(t.target_dir(), dir, "nothing picked: the root");
    t.selected_dir = Some(dir.join("src"));
    assert_eq!(t.target_dir(), dir.join("src"));
    t.start_create(&dir.join("src"), false);
    assert!(matches!(t.editing, Some(Editing::Create { is_dir: false, .. })));
    assert_eq!(t.reveal, Some(dir.join("src")), "the folder opens for the new row");
    // Going somewhere drops what was picked; up climbs; a file is no root.
    t.go(dir.join("src/deep"));
    assert_eq!(t.root, dir.join("src/deep"));
    assert!(t.selected_dir.is_none() && t.editing.is_none());
    t.go(dir.join("src"));
    assert_eq!(t.root, dir.join("src"));
    t.go(dir.join("main.rs"));
    assert_eq!(t.root, dir.join("src"), "a file is not a root");
    // Typed paths: `~`, relative to the root, absolute.
    assert_eq!(t.resolve("~/x"), home().join("x"));
    assert_eq!(t.resolve("deep"), dir.join("src/deep"));
    assert_eq!(t.resolve("/usr"), PathBuf::from("/usr"));
    // A missing root falls back to home.
    assert_eq!(Tree::new(dir.join("nope")).root, home());
    let _ = std::fs::remove_dir_all(&dir);
}
