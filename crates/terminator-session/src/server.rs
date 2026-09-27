use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::protocol::{self, Summary};

struct SocketGuard(PathBuf);
impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct Attached {
    stream: UnixStream,
    input: Vec<u8>,
    output: Vec<u8>,
    output_at: usize,
}

fn pending_output(conn: &mut Attached, bytes: &[u8]) -> bool {
    if conn.output_at > 0 {
        conn.output.drain(..conn.output_at);
        conn.output_at = 0;
    }
    if conn.output.len() + bytes.len() > 8 * 1024 * 1024 {
        return false;
    }
    conn.output.extend_from_slice(bytes);
    true
}

fn flush_output(conn: &mut Attached) -> io::Result<()> {
    while conn.output_at < conn.output.len() {
        match conn.stream.write(&conn.output[conn.output_at..]) {
            Ok(0) => return Err(io::ErrorKind::BrokenPipe.into()),
            Ok(n) => conn.output_at += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    if conn.output_at == conn.output.len() {
        conn.output.clear();
        conn.output_at = 0;
    }
    Ok(())
}

fn take_commands(
    input: &mut Vec<u8>,
    master: &std::fs::File,
    pending: &mut Vec<u8>,
) -> io::Result<()> {
    let mut at = 0;
    while at < input.len() {
        match input[at] {
            protocol::RESIZE if input.len() - at >= 5 => {
                let cols = u16::from_be_bytes([input[at + 1], input[at + 2]]);
                let rows = u16::from_be_bytes([input[at + 3], input[at + 4]]);
                crate::pty::resize(master, cols, rows)?;
                at += 5;
            }
            protocol::INPUT if input.len() - at >= 5 => {
                let n =
                    u32::from_be_bytes(input[at + 1..at + 5].try_into().unwrap_or([0; 4])) as usize;
                if n > protocol::MAX_INPUT {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                if input.len() - at < 5 + n {
                    break;
                }
                if pending.len() + n > 1024 * 1024 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                pending.extend_from_slice(&input[at + 5..at + 5 + n]);
                at += 5 + n;
            }
            protocol::INPUT | protocol::RESIZE => break,
            _ => return Err(io::ErrorKind::InvalidData.into()),
        }
    }
    input.drain(..at);
    Ok(())
}

fn accept(
    listener: &UnixListener,
    summary: &Summary,
    history: &VecDeque<u8>,
    active: &mut Vec<Attached>,
) -> Result<bool> {
    let (mut stream, _) = listener.accept()?;
    stream.set_read_timeout(Some(std::time::Duration::from_millis(250)))?;
    let mut command = [0u8; 1];
    if stream.read_exact(&mut command).is_err() {
        return Ok(false);
    }
    match command[0] {
        protocol::QUERY => {
            let mut info = Summary {
                name: summary.name.clone(),
                title: summary.title.clone(),
                cwd: summary.cwd.clone(),
                attached: !active.is_empty(),
            };
            info.title = info.title.replace(['\n', '\r'], " ");
            serde_json::to_writer(&mut stream, &info)?;
            stream.write_all(b"\n")?;
            Ok(false)
        }
        protocol::ATTACH => {
            stream.set_read_timeout(None)?;
            stream.set_nonblocking(true)?;
            active.push(Attached {
                stream,
                input: Vec::new(),
                output: history.iter().copied().collect(),
                output_at: 0,
            });
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn drain_client(conn: &mut Attached, master: &std::fs::File, pending: &mut Vec<u8>) -> bool {
    loop {
        let mut buf = [0u8; 8192];
        match conn.stream.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => {
                conn.input.extend_from_slice(&buf[..n]);
                if conn.input.len() > 2 * protocol::MAX_INPUT + 5
                    || take_commands(&mut conn.input, master, pending).is_err()
                {
                    return false;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return true,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

fn flush_input(master: &mut std::fs::File, pending: &mut Vec<u8>) -> io::Result<()> {
    while !pending.is_empty() {
        match master.write(pending) {
            Ok(0) => return Err(io::ErrorKind::BrokenPipe.into()),
            Ok(n) => {
                pending.drain(..n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn serve(name: &str, title: &str, cwd: &str) -> Result<()> {
    // A disconnected SSH client must never take down the PTY owner.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let socket = crate::path::socket_path(name)?;
    let listener =
        UnixListener::bind(&socket).with_context(|| format!("bind {}", socket.display()))?;
    let _guard = SocketGuard(socket);
    listener.set_nonblocking(true)?;
    let mut shell = crate::pty::spawn(cwd)?;
    let summary = Summary {
        name: name.into(),
        title: title.into(),
        cwd: cwd.into(),
        attached: false,
    };
    let mut history = VecDeque::new();
    let mut active: Vec<Attached> = Vec::new();
    let mut pending = Vec::new();
    loop {
        let mut fds = vec![
            libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: shell.master.as_raw_fd(),
                events: libc::POLLIN | if pending.is_empty() { 0 } else { libc::POLLOUT },
                revents: 0,
            },
        ];
        fds.extend(active.iter().map(|c| libc::pollfd {
            fd: c.stream.as_raw_fd(),
            events: libc::POLLIN
                | if c.output_at < c.output.len() {
                    libc::POLLOUT
                } else {
                    0
                },
            revents: 0,
        }));
        let polled = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 200) };
        if polled < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e.into());
            }
            continue;
        }
        if fds[0].revents & libc::POLLIN != 0
            && matches!(accept(&listener, &summary, &history, &mut active), Ok(true))
        {
            if let Some(last) = active.last_mut() {
                if !drain_client(last, &shell.master, &mut pending) {
                    active.pop();
                }
            }
        }
        if fds[1].revents & libc::POLLOUT != 0 {
            flush_input(&mut shell.master, &mut pending)?;
        }
        let mut gone = Vec::new();
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match shell.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    protocol::push_history(&mut history, &buf[..n]);
                    for (i, conn) in active.iter_mut().enumerate() {
                        if !pending_output(conn, &buf[..n]) {
                            gone.push(i);
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                Err(e) => return Err(e.into()),
            }
        }
        for (index, event) in fds.iter().enumerate().skip(2) {
            if event.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                let conn = &mut active[index - 2];
                if !drain_client(conn, &shell.master, &mut pending) {
                    gone.push(index - 2);
                }
            }
        }
        gone.sort_unstable();
        gone.dedup();
        for index in gone.into_iter().rev() {
            active.remove(index);
        }
        active.retain_mut(|c| flush_output(c).is_ok());
    }
    let _ = shell.child.try_wait();
    Ok(())
}
