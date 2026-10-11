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
