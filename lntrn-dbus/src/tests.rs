use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

use super::*;

/// Write `bytes` with `fd` attached as SCM_RIGHTS, the way the bus daemon
/// delivers a reply that carries a descriptor.
fn send_with_fd(sock: &UnixStream, bytes: &[u8], fd: RawFd) {
    let mut control = [0u64; 4];
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut _,
        iov_len: bytes.len(),
    };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast();
    let fd_size = std::mem::size_of::<RawFd>() as u32;
    unsafe {
        msg.msg_controllen = libc::CMSG_SPACE(fd_size) as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_size) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        let sent = libc::sendmsg(sock.as_raw_fd(), &msg, 0);
        assert_eq!(sent, bytes.len() as isize, "sendmsg: {}", io::Error::last_os_error());
    }
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut ends = [0 as RawFd; 2];
    assert_eq!(unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) }
}

#[test]
fn a_passed_descriptor_arrives_with_its_reply() {
    let (bus, mut client) = UnixStream::pair().unwrap();
    let (pipe_read, pipe_write) = pipe();

    // A method return whose body is `h`: index 0 into the passed fds.
    let mut body = Vec::new();
    encode_u32(&mut body, 0);
    send_with_fd(&bus, &encode_reply(7, 3, ":1.5", "h", &body), pipe_write.as_raw_fd());
    drop(pipe_write); // only the passed copy keeps the write end open now

    let msg = read_message(&mut client, true).unwrap();
    assert_eq!(msg.reply_serial, 3);
    assert_eq!(msg.signature, "h");
    assert_eq!(msg.fds.len(), 1);

    // The descriptor is live: write through it, read it back off the pipe.
    let mut passed = File::from(msg.fds.into_iter().next().unwrap());
    passed.write_all(b"lantern").unwrap();
    drop(passed);
    let mut got = String::new();
    File::from(pipe_read).read_to_string(&mut got).unwrap();
    assert_eq!(got, "lantern");
}

#[test]
fn a_message_split_across_reads_keeps_its_descriptor() {
    let (mut bus, mut client) = UnixStream::pair().unwrap();
    let (_pipe_read, pipe_write) = pipe();

    // The fd rides on the first byte; the rest of the message follows later.
    let mut body = Vec::new();
    encode_u32(&mut body, 0);
    let wire = encode_reply(2, 1, ":1.5", "h", &body);
    send_with_fd(&bus, &wire[..1], pipe_write.as_raw_fd());
    bus.write_all(&wire[1..]).unwrap();

    let msg = read_message(&mut client, true).unwrap();
    assert_eq!(msg.reply_serial, 1);
    assert_eq!(msg.fds.len(), 1);
}

#[test]
fn connections_without_fd_passing_read_as_before() {
    let (mut bus, mut client) = UnixStream::pair().unwrap();
    bus.write_all(&encode_signal(1, "/org/lantern", "org.lantern.Test", "Ping", "", &[]))
        .unwrap();

    let msg = read_message(&mut client, false).unwrap();
    assert!(msg.is_signal());
    assert_eq!(msg.member, "Ping");
    assert!(msg.fds.is_empty());
}

/// End to end against the real logind: take a sleep delay-inhibitor and check
/// the bus hands its descriptor over. Dropping the fd releases the inhibitor.
#[test]
#[ignore = "talks to the system bus and logind"]
fn logind_hands_over_a_sleep_inhibitor() {
    let mut conn = Connection::connect_system_with_fds().unwrap();
    let mut body = Vec::new();
    for arg in ["sleep", "lntrn-dbus self-test", "checking fd passing", "delay"] {
        encode_string(&mut body, arg);
    }
    let serial = conn.method_call(
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        "Inhibit",
        "ssss",
        &body,
    );
    let reply = loop {
        let msg = conn.read_next().unwrap();
        if msg.reply_serial == serial {
            break msg;
        }
    };
    assert!(!reply.is_error(), "logind refused the inhibitor");
    assert_eq!(reply.signature, "h");
    assert_eq!(reply.fds.len(), 1);
}

/// An `a{sv}` entry whose variant is a 16-bit integer: key, signature,
/// then the two bytes, as BlueZ sends a device's RSSI.
fn entry_i16(body: &mut Vec<u8>, key: &str, v: i16) {
    align_to(body, 8);
    encode_string(body, key);
    encode_signature(body, "n");
    align_to(body, 2);
    body.extend_from_slice(&v.to_le_bytes());
}

#[test]
fn sixteen_bit_integers_take_two_bytes_and_what_follows_still_reads() {
    // a{sv}: RSSI (n), then Name (s), then Appearance (q).
    let mut entries = Vec::new();
    entry_i16(&mut entries, "RSSI", -67);
    encode_dict_entry_sv(&mut entries, "Name", |b| encode_variant_string(b, "Buds"));
    align_to(&mut entries, 8);
    encode_string(&mut entries, "Appearance");
    encode_signature(&mut entries, "q");
    align_to(&mut entries, 2);
    entries.extend_from_slice(&0x0941u16.to_le_bytes());
    let mut body = Vec::new();
    encode_u32(&mut body, entries.len() as u32);
    align_to(&mut body, 8);
    body.extend_from_slice(&entries);

    let props = BodyReader::new(&body, "a{sv}").read_value("a{sv}").unwrap();
    let props = props.as_dict().unwrap();
    assert_eq!(props["RSSI"].as_i32(), Some(-67));
    assert_eq!(props["Name"].as_str(), Some("Buds"));
    assert_eq!(props["Appearance"].as_u32(), Some(0x0941));
}

#[test]
fn doubles_signatures_and_struct_arrays_read() {
    let mut body = Vec::new();
    encode_dict_entry_sv(&mut body, "Level", |b| encode_variant_double(b, 0.65));
    let mut wrapped = Vec::new();
    encode_u32(&mut wrapped, body.len() as u32);
    align_to(&mut wrapped, 8);
    wrapped.extend_from_slice(&body);
    let props = BodyReader::new(&wrapped, "a{sv}").read_value("a{sv}").unwrap();
    assert_eq!(props.as_dict().unwrap()["Level"].as_f64(), Some(0.65));

    // a(on): iwd's ordered networks, a path and a strength each.
    let mut items = Vec::new();
    for (path, strength) in [("/net/connman/iwd/0/4/a", -4500i16), ("/net/connman/iwd/0/4/b", -7000)] {
        align_to(&mut items, 8);
        encode_string(&mut items, path);
        align_to(&mut items, 2);
        items.extend_from_slice(&strength.to_le_bytes());
    }
    let mut body = Vec::new();
    encode_u32(&mut body, items.len() as u32);
    align_to(&mut body, 8);
    body.extend_from_slice(&items);
    let list = BodyReader::new(&body, "a(on)").read_value("a(on)").unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    let Value::Struct(second) = &list[1] else { panic!("a struct") };
    assert_eq!((second[0].as_str(), second[1].as_i32()), (Some("/net/connman/iwd/0/4/b"), Some(-7000)));

    let mut sig = Vec::new();
    encode_signature(&mut sig, "a{sv}");
    assert_eq!(BodyReader::new(&sig, "g").read_value("g").unwrap().as_str(), Some("a{sv}"));
}

#[test]
fn arrays_of_dicts_keep_their_elements() {
    // aa{sv} with two dicts of one entry each, built in one buffer so the
    // padding falls where it does on the wire. Lengths are filled in after.
    let mut body = Vec::new();
    encode_u32(&mut body, 0);
    let start = body.len();
    for name in ["one", "two"] {
        align_to(&mut body, 4);
        let len_at = body.len();
        encode_u32(&mut body, 0);
        align_to(&mut body, 8);
        let entries_at = body.len();
        encode_dict_entry_sv(&mut body, "Name", |b| encode_variant_string(b, name));
        let n = (body.len() - entries_at) as u32;
        body[len_at..len_at + 4].copy_from_slice(&n.to_le_bytes());
    }
    let n = (body.len() - start) as u32;
    body[..4].copy_from_slice(&n.to_le_bytes());
    let list = BodyReader::new(&body, "aa{sv}").read_value("aa{sv}").unwrap();
    let names: Vec<_> = list.as_array().unwrap().iter().map(|d| d.as_dict().unwrap()["Name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(names, ["one", "two"]);
}

#[test]
fn a_length_past_the_end_of_the_message_stops_at_its_end() {
    // Each array claims far more than the message holds; misaligned
    // padding too. Every one must come back, not spin.
    for sig in ["a{sv}", "aa{sv}", "a(iiay)", "as", "ao", "ay", "av", "a(on)", "ai"] {
        for cut in 0..24 {
            let mut body = Vec::new();
            encode_u32(&mut body, 100_000);
            body.extend(std::iter::repeat_n(0x41u8, cut));
            let _ = BodyReader::new(&body, sig).read_value(sig);
        }
    }
    // What is there before the cut still reads.
    let mut body = Vec::new();
    encode_u32(&mut body, 100_000);
    align_to(&mut body, 8);
    encode_dict_entry_sv(&mut body, "Name", |b| encode_variant_string(b, "Buds"));
    let props = BodyReader::new(&body, "a{sv}").read_value("a{sv}").unwrap();
    assert_eq!(props.as_dict().unwrap()["Name"].as_str(), Some("Buds"));
}

#[test]
fn a_call_carries_its_flags() {
    let plain = encode_method_call(1, 0, "net.connman.iwd", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", "", &[]);
    let flagged = encode_method_call(2, FLAG_NO_AUTO_START, "net.connman.iwd", "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", "", &[]);
    assert_eq!((plain[2], flagged[2]), (0, FLAG_NO_AUTO_START));
    assert_eq!(plain.len(), flagged.len());
}
