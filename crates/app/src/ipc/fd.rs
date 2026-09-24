//! SCM_RIGHTS file-descriptor passing over a `UnixStream`.
//!
//! Cross-instance tab migration hands whole PTYs between
//! terminator-rust processes: the sender attaches one fd per pane leaf
//! to the request line inside a single `SCM_RIGHTS` control message,
//! and the receiver wraps the raw ints into [`OwnedFd`] exactly once so
//! an ordinary drop closes them. Linux + macOS, via the `libc` crate's
//! portable cmsg helpers (no glibc-only APIs anywhere); cmsg alignment
//! comes from a `#[repr(align(8))]` scratch buffer.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

/// cmsg scratch capacity (fds per message): one SCM_RIGHTS cmsg carrying
/// up to `MAX_MIGRATE_PANES` descriptors is all the protocol ever sends.
const MAX_FDS: usize = ipc_proto::migrate::MAX_MIGRATE_PANES;
/// Bytes reserved for one cmsg: aligned header + `MAX_FDS` ints + slack
/// for the two `CMSG_ALIGN` roundings. A const expression because
/// `CMSG_SPACE` is not const.
const CMSG_BUF_LEN: usize =
    std::mem::size_of::<libc::cmsghdr>() + MAX_FDS * std::mem::size_of::<libc::c_int>() + 16;

/// cmsghdr-aligned scratch for the control message. `cmsghdr` starts
/// with a length word (align >= 4 on every target we build; 8 covers the
/// 64-bit ones), so align(8) satisfies the kernel's alignment contract.
#[repr(align(8))]
struct CmsgBuf([u8; CMSG_BUF_LEN]);

/// CMSG_SPACE for `n` fd bytes, asserted to fit the scratch buffer.
fn cmsg_space(fds: usize) -> usize {
    let bytes = fds * std::mem::size_of::<libc::c_int>();
    let space = unsafe { libc::CMSG_SPACE(bytes as libc::c_uint) } as usize;
    assert!(
        space <= CMSG_BUF_LEN,
        "cmsg scratch too small for {fds} fds"
    );
    space
}

/// Send `data` with `fds` attached (single SCM_RIGHTS cmsg, one sendmsg).
///
/// `data` must fit in ONE message (one socket-buffer write): the fds ride
/// the first byte of this sendmsg only, so a partial write cannot be
/// retried — callers keep header+payload below the socket buffer
/// (~200 KiB minimum on Linux) and a short write surfaces as
/// [`io::ErrorKind::WriteZero`] rather than a silently wrong retry.
/// `EINTR` before any transfer is retried internally.
// Sender half of the migration handshake: the source instance (and the
// tests) drive it; nothing in the receiver-only app path calls it yet.
#[allow(dead_code)]
pub fn send_with_fds(stream: &UnixStream, data: &[u8], fds: &[OwnedFd]) -> io::Result<()> {
    if fds.len() > MAX_FDS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "send_with_fds: {} fds exceed the {} cap",
                fds.len(),
                MAX_FDS
            ),
        ));
    }
    let raw: Vec<libc::c_int> = fds.iter().map(|f| f.as_raw_fd()).collect();
    let mut iov = libc::iovec {
        iov_base: data.as_ptr().cast::<libc::c_void>().cast_mut(),
        iov_len: data.len(),
    };
    let mut control = CmsgBuf([0u8; CMSG_BUF_LEN]);
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if !raw.is_empty() {
        let bytes = raw.len() * std::mem::size_of::<libc::c_int>();
        let space = cmsg_space(raw.len());
        msg.msg_control = control.0.as_mut_ptr().cast::<libc::c_void>();
        msg.msg_controllen = space as _;
        let cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
        if cmsg.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "send_with_fds: no room for the cmsg",
            ));
        }
        unsafe {
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(bytes as libc::c_uint) as _;
            std::ptr::copy_nonoverlapping(raw.as_ptr().cast::<u8>(), libc::CMSG_DATA(cmsg), bytes);
        }
    }
    loop {
        // SAFETY: `msg`/`iov`/`control` outlive the call; the kernel only
        // reads them. EINTR means nothing was transferred yet.
        let n = unsafe { libc::sendmsg(stream.as_raw_fd(), &msg, 0) };
        if n >= 0 {
            return if n as usize == data.len() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "send_with_fds: partial sendmsg; fds attach once so the message cannot be retried",
                ))
            };
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

/// One recvmsg collecting up to `max_fds` passed fds plus the first data
/// chunk. Returns `(bytes_read, fds)`.
///
/// Errors when MORE fds than `max_fds` (or than the internal cap) arrive
/// or the cmsg was truncated: every received fd has already been drained
/// into the returned-path `Vec` and is closed by the drop BEFORE the
/// error surfaces, so an unruly peer cannot leak descriptors. `EINTR` is
/// retried (nothing was consumed). Ancillary data attaches to the first
/// byte of the queued message: for a stream socket that means the fds
/// arrive with the FIRST recvmsg on that message — later reads are data
/// only.
pub fn recv_with_fds(
    stream: &UnixStream,
    buf: &mut [u8],
    max_fds: usize,
) -> io::Result<(usize, Vec<OwnedFd>)> {
    let capacity = max_fds.min(MAX_FDS);
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: buf.len(),
    };
    let mut control = CmsgBuf([0u8; CMSG_BUF_LEN]);
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if capacity > 0 {
        let space = cmsg_space(capacity);
        msg.msg_control = control.0.as_mut_ptr().cast::<libc::c_void>();
        msg.msg_controllen = space as _;
    }
    let n = loop {
        // SAFETY: plain recvmsg into our buffers; flags are 0 (no
        // MSG_ extras) so behavior matches a blocking read(2).
        let n = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, 0) };
        if n >= 0 {
            break n;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    };
    let truncated = msg.msg_flags & libc::MSG_CTRUNC != 0;
    let mut fds: Vec<OwnedFd> = Vec::new();
    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        unsafe {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let header = libc::CMSG_LEN(0) as usize;
                let len = (*cmsg).cmsg_len as usize;
                if len >= header {
                    let data = libc::CMSG_DATA(cmsg);
                    let count = (len - header) / std::mem::size_of::<libc::c_int>();
                    // The kernel aligns cmsg payload to the cmsghdr, which
                    // is at least c_int-aligned on every supported target.
                    let ints = std::slice::from_raw_parts(data.cast::<libc::c_int>(), count);
                    fds.extend(ints.iter().map(|&raw| OwnedFd::from_raw_fd(raw)));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    if truncated || fds.len() > max_fds {
        let seen = fds.len();
        // Close every drained fd before reporting (drop = close).
        drop(fds);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("recv_with_fds: {seen} fds exceed the max {max_fds}"),
        ));
    }
    Ok((n as usize, fds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn two_fds() -> Vec<OwnedFd> {
        let f = OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
        vec![f.try_clone().unwrap(), f]
    }

    #[test]
    fn roundtrips_data_and_fds_over_a_socketpair() {
        let (a, b) = UnixStream::pair().unwrap();
        let sent = two_fds();
        send_with_fds(&a, b"header\n\x01\x02\x03", &sent).unwrap();
        let mut buf = [0u8; 64];
        let (n, fds) = recv_with_fds(&b, &mut buf, MAX_FDS).unwrap();
        assert_eq!(&buf[..n], b"header\n\x01\x02\x03");
        assert_eq!(fds.len(), 2);
        // The received fds are real, distinct, open descriptors: dup'ing
        // each must succeed (a closed/leaked one would fail with EBADF),
        // and the dups close on drop without touching the originals.
        for fd in &fds {
            assert!(fd.try_clone().is_ok());
        }
        // Distinct descriptors (the sender's two were separate dups).
        assert_ne!(
            fds[0].as_raw_fd(),
            fds[1].as_raw_fd(),
            "SCM_RIGHTS must install fresh descriptors"
        );
    }

    #[test]
    fn plain_send_carries_no_fds() {
        let (a, b) = UnixStream::pair().unwrap();
        send_with_fds(&a, b"{}", &[]).unwrap();
        let mut buf = [0u8; 16];
        let (n, fds) = recv_with_fds(&b, &mut buf, MAX_FDS).unwrap();
        assert_eq!(&buf[..n], b"{}");
        assert!(fds.is_empty());
    }

    #[test]
    fn excess_fds_error_after_closing_them() {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        send_with_fds(&a, b"x", &two_fds()).unwrap();
        let mut buf = [0u8; 8];
        let err = recv_with_fds(&b, &mut buf, 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        // The stream still works for a follow-up data-only message.
        a.write_all(b"after").unwrap();
        drop(a); // EOF so read_to_string terminates
        let mut more = String::new();
        b.read_to_string(&mut more).unwrap();
        assert_eq!(more, "after");
    }

    #[test]
    fn oversize_fd_list_is_rejected_upfront() {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut many: Vec<OwnedFd> = Vec::new();
        let seed = OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
        for _ in 0..=MAX_FDS {
            many.push(seed.try_clone().unwrap());
        }
        drop(seed);
        let err = send_with_fds(&a, b"x", &many).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
    }
}
