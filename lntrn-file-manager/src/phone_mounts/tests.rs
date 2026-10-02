use super::*;

#[test]
fn records_round_trip_and_tolerate_junk() {
    let record = Record {
        usb_address: Some((1, 27)),
        owner: Some(4242),
    };
    assert_eq!(parse_record(&format_record(&record)), record);
    assert_eq!(parse_record(""), Record::default());
    assert_eq!(
        parse_record("device=one,two\nowner=\nnoise\nowner=7\n"),
        Record {
            usb_address: None,
            owner: Some(7),
        }
    );
}

#[test]
fn a_replugged_phone_makes_its_old_mount_stale() {
    let made_at = |addr| {
        Some(Record {
            usb_address: addr,
            owner: Some(1),
        })
    };
    assert_eq!(
        mount_state(made_at(Some((1, 5))), Some((1, 5))),
        MountState::Fresh
    );
    // Same phone, same mount point, new device number after the re-plug.
    assert_eq!(
        mount_state(made_at(Some((1, 5))), Some((1, 6))),
        MountState::Stale
    );
    assert_eq!(
        mount_state(made_at(Some((1, 5))), Some((2, 5))),
        MountState::Stale
    );
    // Nothing to compare: never called stale on a guess.
    assert_eq!(mount_state(None, Some((1, 5))), MountState::Unknown);
    assert_eq!(
        mount_state(made_at(None), Some((1, 5))),
        MountState::Unknown
    );
    assert_eq!(
        mount_state(made_at(Some((1, 5))), None),
        MountState::Unknown
    );
}

#[test]
fn only_the_owner_unmounts_and_only_when_nobody_needs_the_mount() {
    let me = 100;
    assert!(exit_unmounts(Some(me), me, false, false));
    // A picker or second window that merely browsed it.
    assert!(!exit_unmounts(Some(7), me, false, false));
    assert!(!exit_unmounts(None, me, false, false));
    // Another Fox is inside.
    assert!(!exit_unmounts(Some(me), me, true, false));
    // A picker that mounted the phone and is returning a file on it.
    assert!(!exit_unmounts(Some(me), me, false, true));
}

#[test]
fn mounts_without_their_phone_are_the_ones_detached() {
    let phone = |mp: &str, addr| Phone {
        name: "P".into(),
        manufacturer: String::new(),
        product: String::new(),
        vendor_id: String::new(),
        product_id: String::new(),
        serial: String::new(),
        mount_point: PathBuf::from(mp),
        mounted: true,
        usb_address: addr,
    };
    let mounts: Vec<PathBuf> = ["/m/alive", "/m/unplugged", "/m/replugged", "/m/by-hand"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let phones = [
        phone("/m/alive", Some((1, 4))),
        phone("/m/replugged", Some((1, 9))),
        phone("/m/by-hand", Some((1, 3))),
    ];
    let record_of = |mp: &Path| match mp.to_str()? {
        "/m/alive" => Some(Record {
            usb_address: Some((1, 4)),
            owner: Some(1),
        }),
        "/m/replugged" => Some(Record {
            usb_address: Some((1, 8)),
            owner: Some(1),
        }),
        // Mounted outside Fox: no record, and so never guessed dead
        // while its phone is attached.
        _ => None,
    };
    assert_eq!(
        stale_mounts(&mounts, &phones, &record_of),
        [PathBuf::from("/m/unplugged"), PathBuf::from("/m/replugged")]
    );
    // With no phone attached at all, every mount is a leftover.
    assert_eq!(stale_mounts(&mounts, &[], &record_of).len(), 4);
}

#[test]
fn a_mount_is_detached_only_when_found_dead_twice_in_a_row() {
    let m = |p: &str| PathBuf::from(p);
    let mut suspects = Vec::new();
    // First sighting: suspected, not touched.
    assert!(confirm_dead(vec![m("/m/a")], &mut suspects).is_empty());
    assert_eq!(suspects, [m("/m/a")]);
    // Alive again on the next look (a glitch): forgotten.
    assert!(confirm_dead(vec![], &mut suspects).is_empty());
    assert!(suspects.is_empty());
    // Dead on two looks in a row: detached. A newcomer waits its turn.
    assert!(confirm_dead(vec![m("/m/a")], &mut suspects).is_empty());
    assert_eq!(
        confirm_dead(vec![m("/m/a"), m("/m/b")], &mut suspects),
        [m("/m/a")]
    );
    assert_eq!(suspects, [m("/m/b")]);
}
