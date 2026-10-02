use super::*;
use std::path::Path;

#[test]
fn a_uri_list_becomes_local_paths() {
    let list = b"# a comment\r\nfile:///home/a/one.txt\r\nfile://localhost/home/a/two%20words.txt\r\n\r\nfile:///home/a/caf%C3%A9\n";
    assert_eq!(
        parse_uri_list(list),
        vec![
            PathBuf::from("/home/a/one.txt"),
            PathBuf::from("/home/a/two words.txt"),
            PathBuf::from("/home/a/caf\u{e9}"),
        ]
    );
}

#[test]
fn names_that_are_not_utf8_or_hold_odd_characters_survive() {
    use std::os::unix::ffi::OsStrExt;
    let list = b"file:///tmp/100%25%23%3F.txt\r\nfile:///tmp/raw%FFbyte\r\n";
    let paths = parse_uri_list(list);
    assert_eq!(paths[0], Path::new("/tmp/100%#?.txt"));
    assert_eq!(paths[1].as_os_str().as_bytes(), b"/tmp/raw\xFFbyte");
    // What Fox itself sends for a drag comes back as the same path.
    let odd = Path::new("/tmp/a b#c%d\u{e9}");
    let sent = format!("file://{}\r\n", crate::file_ops::percent_encode_path(odd));
    assert_eq!(parse_uri_list(sent.as_bytes()), vec![odd.to_path_buf()]);
}

#[test]
fn what_is_not_a_plain_local_path_is_left_out() {
    let list = b"https://example.com/x\r\n\
        file://otherhost/home/a/x\r\n\
        file:relative\r\n\
        file:///home/a/../../etc/passwd\r\n\
        file:///\r\n\
        file:///home/a/x\r\n\
        file:///home/a/x\r\n";
    assert_eq!(parse_uri_list(list), vec![PathBuf::from("/home/a/x")]);
    assert!(parse_uri_list(b"").is_empty());
    assert!(parse_uri_list(b"just some text").is_empty());
}

#[test]
fn a_broken_escape_is_kept_as_typed() {
    assert_eq!(percent_decode(b"a%2"), b"a%2");
    assert_eq!(percent_decode(b"a%zzb"), b"a%zzb");
    assert_eq!(percent_decode(b"%41%4a"), b"AJ");
}

#[test]
fn the_hint_names_where_the_drop_goes() {
    let folder = DropTarget::Folder {
        dir: PathBuf::from("/home/a/Pictures"),
        reload_tab: None,
        in_pane: false,
    };
    assert_eq!(describe(&folder), "Drop into \u{201C}Pictures\u{201D}");
    let root = DropTarget::Folder {
        dir: PathBuf::from("/"),
        reload_tab: None,
        in_pane: false,
    };
    assert_eq!(describe(&root), "Drop into \u{201C}/\u{201D}");
    assert_eq!(describe(&DropTarget::Favorites), "Drop to add to Favorites");
}

#[test]
fn the_whole_list_is_read_and_a_silent_source_does_not_hang_the_reader() {
    use std::io::Write;
    let (reader, mut writer) = std::io::pipe().unwrap();
    let big = vec![b'x'; 300_000];
    let expect = big.clone();
    let feeder = std::thread::spawn(move || {
        writer.write_all(&big).unwrap();
    });
    assert_eq!(read_offer(reader), Some(expect));
    feeder.join().unwrap();

    // More than any list of paths: given up on, not buffered forever.
    let (reader, mut writer) = std::io::pipe().unwrap();
    let feeder = std::thread::spawn(move || {
        let block = vec![0u8; 1 << 20];
        while writer.write_all(&block).is_ok() {}
    });
    assert_eq!(read_offer(reader), None);
    feeder.join().unwrap();
}
