//! PTY creation and I/O on top of `libc` (Linux + macOS).

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

use anyhow::{bail, Context, Result};

/// A live pseudo-terminal attached to a child process.
#[derive(Debug, Clone, Copy)]
pub struct PtyHandle {
    /// Master (parent) side of the pty.
    pub master_fd: RawFd,
    /// Child process id, 0 if unknown.
    pub child_pid: i32,
}

/// Open a pty of `cols` x `rows` cells running `argv` and return the master.
///
/// The child gets a fresh session with the pty as its controlling terminal,
/// `TERM=xterm-256color`, and UTF-8 `LANG`/`LC_ALL` unless the caller passes
/// overrides through `extra_env` (entries shaped `KEY=VALUE`).
pub fn open_pty(cols: u16, rows: u16, argv: &[&str], extra_env: &[String]) -> Result<PtyHandle> {
    if argv.is_empty() {
        bail!("argv must not be empty");
    }
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    // SAFETY: plain C calls with valid out-pointers.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error()).context("openpty");
    }

    let prog = CString::new(argv[0]).context("argv[0] contains NUL")?;
    let cargv: Vec<CString> = argv
        .iter()
        .map(|a| CString::new(*a).context("argv contains NUL"))
        .collect::<Result<_>>()?;
    let mut env: Vec<CString> = vec![
        CString::new("TERM=xterm-256color")?,
        CString::new("TERM_PROGRAM=terminator-rust")?,
        CString::new("LANG=C.UTF-8")?,
        CString::new("LC_ALL=C.UTF-8")?,
    ];
    for kv in extra_env {
        if kv.contains('=') {
            env.push(CString::new(kv.as_str())?);
        }
    }
    // Inherit the rest of the parent environment (PATH etc.).
    for (k, v) in std::env::vars_os() {
        let key = k.to_string_lossy();
        if key == "TERM" || key == "TERM_PROGRAM" || key == "LANG" || key == "LC_ALL" {
            continue;
        }
        let joined = format!("{}={}", key, v.to_string_lossy());
        if let Ok(c) = CString::new(joined) {
            env.push(c);
        }
    }

    // SAFETY: fork in a single-threaded context here (called before any
    // reader thread exists).
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            let err = io::Error::last_os_error();
            unsafe {
                libc::close(master);
                libc::close(slave);
            }
            return Err(err).context("fork");
        }
        0 => {
            // Child: new session, slave becomes ctty + stdio, then exec.
            unsafe {
                libc::setsid();
                libc::ioctl(slave, libc::TIOCSCTTY, 0);
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);
                if slave > 2 {
                    libc::close(slave);
                }
                libc::close(master);
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                let argvp: Vec<*const libc::c_char> = cargv
                    .iter()
                    .map(|c| c.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect();
                let envp: Vec<*const libc::c_char> = env
                    .iter()
                    .map(|c| c.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect();
                // execve has no PATH lookup; resolve the program ourselves.
                if prog.to_bytes().contains(&b'/') {
                    libc::execve(prog.as_ptr(), argvp.as_ptr(), envp.as_ptr());
                } else if let Some(path) = resolve_on_path(&prog, &env) {
                    libc::execve(path.as_ptr(), argvp.as_ptr(), envp.as_ptr());
                }
                // exec failed: nothing safe left to do.
                libc::_exit(127);
            }
        }
        child => {
            // Parent keeps only the master.
            unsafe { libc::close(slave) };
            Ok(PtyHandle {
                master_fd: master,
                child_pid: child,
            })
        }
    }
}

/// Write a slice to the pty master, retrying on partial writes.
pub fn pty_write(fd: RawFd, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        // SAFETY: plain C call on an int fd.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if err.raw_os_error() == Some(libc::EIO) {
                // Child side gone; treat as benign EOF for writers.
                return Ok(());
            }
            return Err(err).context("pty write");
        }
        data = &data[n as usize..];
    }
    Ok(())
}

/// Resize the pty window.
pub fn pty_resize(fd: RawFd, cols: u16, rows: u16) -> Result<()> {
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: plain C call.
    let rc = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &winsize) };
    if rc != 0 {
        return Err(io::Error::last_os_error()).context("TIOCSWINSZ");
    }
    Ok(())
}

/// Reap the child if it has exited; returns Some(exit status) then.
pub fn pty_wait(pid: i32, non_blocking: bool) -> Result<Option<i32>> {
    let mut status: libc::c_int = 0;
    let flags = if non_blocking { libc::WNOHANG } else { 0 };
    // SAFETY: plain C call.
    let rc = unsafe { libc::waitpid(pid, &mut status, flags) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ECHILD) {
            return Ok(Some(0));
        }
        return Err(err).context("waitpid");
    }
    if rc == 0 {
        return Ok(None);
    }
    if libc::WIFEXITED(status) {
        Ok(Some(libc::WEXITSTATUS(status)))
    } else if libc::WIFSIGNALED(status) {
        Ok(Some(128 + libc::WTERMSIG(status)))
    } else {
        Ok(Some(0))
    }
}

/// Find `prog` on the PATH encoded in `env` (`KEY=VALUE` CStrings).
fn resolve_on_path(prog: &CString, env: &[CString]) -> Option<CString> {
    let path_val = env
        .iter()
        .find(|c| c.as_bytes().starts_with(b"PATH="))?
        .to_bytes();
    let path_val = &path_val[b"PATH=".len()..];
    let prog_bytes = prog.to_bytes();
    for dir in path_val.split(|b| *b == b':') {
        let dir_s = String::from_utf8_lossy(dir);
        let prog_s = String::from_utf8_lossy(prog_bytes);
        let joined = if dir.is_empty() {
            format!("./{prog_s}")
        } else {
            format!("{dir_s}/{prog_s}")
        };
        if let Ok(candidate) = CString::new(joined) {
            // SAFETY: plain C call with a valid pointer.
            // SAFETY: zeroed stat buffer is valid for C to fill.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let rc = unsafe { libc::stat(candidate.as_ptr(), &raw mut st) };
            if rc != 0 {
                continue;
            }
            if st.st_mode & libc::S_IFMT == libc::S_IFREG && st.st_mode & 0o111 != 0 {
                return Some(candidate);
            }
        }
    }
    None
}
