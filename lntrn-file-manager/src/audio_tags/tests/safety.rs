//! Saves that must not happen, or must not take more than was asked:
//! a file another program is writing, tags another program changed while
//! the dialog was open, a file whose other properties a rewrite would drop.

use super::super::{is_refusal, mp3, read, wav, write, write_from};
use super::{sample_tags, scratch, synth_mp3, synth_wav};

#[test]
fn a_file_someone_is_still_writing_is_refused_untouched() {
    for (name, bytes) in [
        ("busy-tail.wav", synth_wav(false)),
        ("busy-lead.wav", synth_wav(true)),
        ("busy.mp3", synth_mp3(40, true)),
    ] {
        let p = scratch(name);
        std::fs::write(&p, &bytes).unwrap();
        // A recorder, a copy, a download: it has the file open for writing.
        let writer = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        let err = write(&p, &sample_tags()).unwrap_err();
        assert!(is_refusal(&err), "{name}: {err}");
        assert!(err.contains("still writing"), "{name}: {err}");
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "{name} was touched");
        drop(writer);
        // Once it has let go, the save goes through.
        write(&p, &sample_tags()).unwrap();
        assert_eq!(read(&p).unwrap().tags, sample_tags(), "{name}");
    }
}

/// The dialog loads a file; another program retags it; the user changes one
/// field and saves. Only that field may change: everything the other
/// program wrote stays, cover included.
#[test]
fn a_save_writes_only_what_the_user_changed_since_the_dialog_loaded() {
    for (name, bytes) in [
        ("baseline-tail.wav", synth_wav(false)),
        ("baseline-lead.wav", synth_wav(true)),
        ("baseline.mp3", synth_mp3(40, false)),
    ] {
        let p = scratch(name);
        std::fs::write(&p, &bytes).unwrap();
        let mut first = sample_tags();
        first.artwork = None;
        first.key = String::new();
        write(&p, &first).unwrap();

        // The dialog opens here.
        let shown = read(&p).unwrap().tags;
        assert_eq!(shown, first);

        // Meanwhile: a new artist, a key and a cover from somewhere else.
        let mut theirs = first.clone();
        theirs.artist = "Someone Else".into();
        theirs.key = "8A".into();
        theirs.artwork = sample_tags().artwork;
        write(&p, &theirs).unwrap();

        // The user edits the title only.
        let mut edited = shown.clone();
        edited.title = "RETITLED".into();
        write_from(&p, &shown, &edited).unwrap();

        let now = read(&p).unwrap().tags;
        assert_eq!(now.title, "RETITLED", "{name}");
        assert_eq!(now.artist, "Someone Else", "{name}: their artist was reverted");
        assert_eq!(now.key, "8A", "{name}: their key was wiped");
        assert_eq!(now.artwork, theirs.artwork, "{name}: their cover was removed");

        // Nothing changed in the dialog: nothing is written at all.
        let before = std::fs::read(&p).unwrap();
        write_from(&p, &shown, &shown).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), before, "{name}");
    }
}

#[test]
fn a_read_only_wav_is_refused_on_both_paths() {
    use std::os::unix::fs::PermissionsExt;
    for (name, lead) in [("ro-tail.wav", false), ("ro-lead.wav", true)] {
        let p = scratch(name);
        let _ = std::fs::remove_file(&p);
        let bytes = synth_wav(lead);
        std::fs::write(&p, &bytes).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();
        if std::fs::OpenOptions::new().write(true).open(&p).is_ok() {
            // Running as root: nothing is read-only.
            continue;
        }
        assert!(wav::write(&p, &sample_tags()).is_err(), "{name}");
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "{name} was replaced");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
}

/// The rewrite path makes a new file. It must be the old one in everything
/// but its bytes, and it must not quietly split a second name off.
#[test]
fn a_rewrite_keeps_attributes_and_refuses_to_split_hard_links() {
    use std::os::unix::ffi::OsStrExt;
    let p = scratch("attrs-lead.wav");
    let _ = std::fs::remove_file(&p);
    std::fs::write(&p, synth_wav(true)).unwrap();
    let c_path = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
    let name = c"user.xdg.origin.url";
    let value = b"https://example.org/set.wav";
    let set = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    wav::write(&p, &sample_tags()).unwrap();
    assert_eq!(read(&p).unwrap().tags, sample_tags());
    if set == 0 {
        let mut buf = [0u8; 64];
        let got = unsafe {
            libc::getxattr(c_path.as_ptr(), name.as_ptr(), buf.as_mut_ptr().cast(), buf.len())
        };
        assert_eq!(&buf[..got.max(0) as usize], value, "the attribute was dropped");
    } else {
        eprintln!("this filesystem has no user attributes: that half is skipped");
    }

    // A second name for the same file: the rewrite is refused, both names
    // keep what they had. (A fresh file: the one above has its tags behind
    // the audio now and would be saved in place.)
    let p = scratch("links-lead.wav");
    let twin = scratch("links-twin.wav");
    let _ = (std::fs::remove_file(&p), std::fs::remove_file(&twin));
    std::fs::write(&p, synth_wav(true)).unwrap();
    std::fs::hard_link(&p, &twin).unwrap();
    let before = std::fs::read(&p).unwrap();
    let mut other = sample_tags();
    other.title = "SPLIT".into();
    let err = wav::write(&p, &other).unwrap_err();
    assert!(is_refusal(&err) && err.contains("hard links"), "{err}");
    assert_eq!(std::fs::read(&p).unwrap(), before);
    assert_eq!(std::fs::read(&twin).unwrap(), before);
    let _ = std::fs::remove_file(&twin);
    // The in-place path changes the one file both names are: no refusal.
    let q = scratch("attrs-tail.wav");
    let twin = scratch("attrs-tail-twin.wav");
    let _ = (std::fs::remove_file(&q), std::fs::remove_file(&twin));
    std::fs::write(&q, synth_wav(false)).unwrap();
    std::fs::hard_link(&q, &twin).unwrap();
    wav::write(&q, &sample_tags()).unwrap();
    assert_eq!(read(&twin).unwrap().tags, sample_tags());
    let _ = std::fs::remove_file(&twin);
    let _ = mp3::write; // (the MP3 rewrite goes through the same replace_file)
}
