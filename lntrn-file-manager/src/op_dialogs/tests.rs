//! The wording of the notices and questions: what they name, and that a
//! permanent delete is always called final.

use super::*;

fn fail(path: &str, reason: &str) -> OpFailure {
    OpFailure {
        path: PathBuf::from(path),
        reason: reason.to_string(),
    }
}

#[test]
fn failure_notice_names_items_and_counts_the_rest() {
    let (title, lines) = failure_notice("copied", &[fail("/mnt/stick/big.mkv", "too large")]);
    assert_eq!(title, "1 item could not be copied");
    assert_eq!(lines, vec!["/mnt/stick/big.mkv: too large"]);

    let many: Vec<OpFailure> = (0..11)
        .map(|i| fail(&format!("/home/a/Photos/Holidays in Portugal/2026/img{i}.jpg"), "no space left"))
        .collect();
    let (title, lines) = failure_notice("moved", &many);
    assert_eq!(title, "11 items could not be moved");
    assert_eq!(lines.len(), MAX_LISTED + 1);
    assert_eq!(lines[0], "\u{2026}/2026/img0.jpg: no space left");
    assert_eq!(lines[MAX_LISTED], "\u{2026}and 3 more.");
}

#[test]
fn permanent_delete_question_names_the_item_and_says_it_is_final() {
    let (title, lines) = permanent_delete_question(&[PathBuf::from("/t/files/a.jpg")]);
    assert_eq!(title, "Delete \u{201C}a.jpg\u{201D} permanently?");
    assert!(lines[0].contains("cannot be undone"));
    let (title, lines) =
        permanent_delete_question(&[PathBuf::from("/t/files/a"), PathBuf::from("/t/files/b")]);
    assert_eq!(title, "Delete 2 items permanently?");
    // Several items are named, so the question is about exactly these.
    assert!(lines.contains(&"a".to_string()) && lines.contains(&"b".to_string()));
}

#[test]
fn empty_trash_question_counts_and_names_what_goes() {
    let items: Vec<PathBuf> = (0..10)
        .map(|i| PathBuf::from(format!("/t/files/photo{i}.jpg")))
        .collect();
    let (title, lines) = empty_trash_question(&items);
    assert_eq!(title, "Empty the Trash?");
    assert!(lines[0].starts_with("10 items") && lines[0].contains("cannot be undone"));
    assert_eq!(lines[2], "photo0.jpg");
    assert_eq!(lines.last().unwrap(), "\u{2026}and 2 more.");
    let (_, lines) = empty_trash_question(&items[..1]);
    assert!(lines[0].starts_with("1 item "));
}

#[test]
fn delete_question_names_every_item_it_is_about() {
    let same = vec![
        (PathBuf::from("/srv/www/a.html"), "permission denied".to_string()),
        (PathBuf::from("/srv/www/b.html"), "permission denied".to_string()),
    ];
    let (title, lines) = delete_question(&same);
    assert!(title.starts_with("2 items"));
    assert_eq!(lines[0], "Reason: permission denied.");
    assert!(lines.contains(&"a.html".to_string()) && lines.contains(&"b.html".to_string()));
    assert!(lines.last().unwrap().contains("cannot be undone"));
}

#[test]
fn delete_question_says_why_and_that_it_is_final() {
    let one = vec![(PathBuf::from("/mnt/phone/DCIM/a.jpg"), "phones have no Trash".to_string())];
    let (title, lines) = delete_question(&one);
    assert!(title.contains("a.jpg") && title.contains("Trash"));
    assert_eq!(lines[0], "Reason: phones have no Trash.");
    assert!(lines.last().unwrap().contains("cannot be undone"));

    let mixed = vec![
        (PathBuf::from("/a/x"), "read-only".to_string()),
        (PathBuf::from("/a/y"), "permission denied".to_string()),
    ];
    let (title, lines) = delete_question(&mixed);
    assert!(title.starts_with("2 items"));
    assert_eq!(lines[0], "/a/x: read-only");
    assert_eq!(lines[1], "/a/y: permission denied");
}
