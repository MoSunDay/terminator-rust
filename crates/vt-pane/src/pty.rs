//! PTY creation and I/O on top of `libc` (Linux + macOS).

use std::ffi::CString;
use std::io;
use std::os::fd::{FromRawFd, RawFd};

use anyhow::{bail, Context, Result};

/// macOS has no C.UTF-8 locale; en_US.UTF-8 ships with every install.
#[cfg(target_os = "macos")]
const LOCALE: &str = "en_US.UTF-8";
#[cfg(not(target_os = "macos"))]
const LOCALE: &str = "C.UTF-8";

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
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // Pre-fork: build every string/pointer table and resolve the program
    // BEFORE opening the pty pair - a malformed entry (interior NUL) must
    // fail without leaking the just-opened fds. And once other sessions
    // exist their reader threads are running, and the forked child must
    // not malloc (a sibling thread can hold the arena lock at fork time
    // and wedge the child before exec). After the fork the child only
    // runs async-signal-safe calls on these pre-built tables.
    let prog = CString::new(argv[0]).context("argv[0] contains NUL")?;
    let cargv: Vec<CString> = argv
        .iter()
        .map(|a| CString::new(*a).context("argv contains NUL"))
        .collect::<Result<_>>()?;
    let mut env: Vec<CString> = vec![
        CString::new("TERM=xterm-256color")?,
        CString::new("TERM_PROGRAM=terminator-rust")?,
        CString::new(format!("LANG={LOCALE}"))?,
        CString::new(format!("LC_ALL={LOCALE}"))?,
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
    // execve has no PATH lookup; resolve the program in the parent.
    let exec_prog = if prog.to_bytes().contains(&b'/') {
        prog.clone()
    } else {
        resolve_on_path(&prog, &env).unwrap_or_else(|| prog.clone())
    };

    // CLOEXEC on BOTH descriptors (openpty has no flags): a sibling
    // thread forking while our slave is open would otherwise leak it into
    // a foreign child, and that holder keeps our master from ever seeing
    // EOF (exit detection waits for it). The child's dup2(2) clears
    // FD_CLOEXEC on stdio, so stdio survives exec.
    let (master, slave) = open_pty_pair(&winsize)?;

    // SAFETY: the child branch below only calls async-signal-safe
    // functions on pre-built tables.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            let err = io::Error::last_os_error();
            unsafe {
                libc::close(master);
                libc::close(slave);
            }
            Err(err).context("fork")
        }
        0 => {
            // Child: new session, slave becomes ctty + stdio, then exec.
            unsafe {
                libc::setsid();
                libc::ioctl(slave, TIOCSCTTY_IOCTL, 0);
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);
                if slave > 2 {
                    libc::close(slave);
                }
                libc::close(master);
                // SIG_IGN survives fork+execve: a background/desktop
                // launch leaves INT/QUIT/TERM/HUP ignored, which would make
                // ^C/^\ and kill useless inside the pane. Reset to default.
                libc::signal(libc::SIGHUP, libc::SIG_DFL);
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGQUIT, libc::SIG_DFL);
                libc::signal(libc::SIGTERM, libc::SIG_DFL);
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                // execProg was resolved pre-fork; a failed lookup still
                // execs the bare name (ENOENT) to reach the same 127.
                libc::execve(exec_prog.as_ptr(), argvp.as_ptr(), envp.as_ptr());
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

/// Duplicate an fd (`F_DUPFD_CLOEXEC`, portable across the supported
/// unices). The duplicate is independent of any later close of the
/// original descriptor.
pub fn dup_fd(fd: RawFd) -> io::Result<std::os::fd::OwnedFd> {
    // SAFETY: plain C fcntl with a dummy third arg (ignored for
    // F_DUPFD_CLOEXEC-style requests).
    let n = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl returned this fresh, previously-unowned descriptor.
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(n) })
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
        let n = n as usize;
        if n == 0 {
            // POSIX never returns 0 for a nonzero blocking write; do not
            // spin forever if some exotic fd disagrees.
            anyhow::bail!("pty write returned 0");
        }
        data = &data[n..];
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

/// Resolve the slave device path of a pty master fd.
///
/// glibc exposes the thread-safe `ptsname_r`; Apple platforms instead use
/// the `TIOCPTYGNAME` ioctl, which the libc crate does not export.
// Apple's thread-safe ptsname equivalent. The encoding is
// `_IOC(IOC_OUT, 't', N, 128)`: the command number moved from 99 to 83 in
// the modern SDKs and the direction is IOC_OUT, not IOC_IN. The request
// previously hardcoded here (0x80807463) got BOTH wrong, so
// `ioctl(TIOCPTYGNAME)` failed with ENOTTY, `pty_slave_name` errored, and
// every pane spawn failed. Try the current SDK's request first, then the
// legacy encoding, so both old and new macOS work.
#[cfg(target_os = "macos")]
const TIOCPTYGNAME: libc::c_ulong = 0x4080_7453; // _IOC(IOC_OUT, 't', 83, 128)
#[cfg(target_os = "macos")]
const TIOCPTYGNAME_LEGACY: libc::c_ulong = 0x4080_7463; // _IOC(IOC_OUT, 't', 99, 128)

/// Darwin's libc exposes TIOCSCTTY as `c_uint` while `ioctl` takes
/// `c_ulong`; normalize per platform so call sites stay uniform.
#[cfg(target_os = "macos")]
const TIOCSCTTY_IOCTL: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;
#[cfg(not(target_os = "macos"))]
const TIOCSCTTY_IOCTL: libc::c_ulong = libc::TIOCSCTTY;

fn pty_slave_name(master: RawFd) -> io::Result<String> {
    #[cfg(target_os = "macos")]
    {
        let mut buf = [0u8; 128];
        let mut last_err = None;
        for req in [TIOCPTYGNAME, TIOCPTYGNAME_LEGACY] {
            // SAFETY: plain ioctl writing into a valid buffer.
            if unsafe { libc::ioctl(master, req, buf.as_mut_ptr(), buf.len()) } == 0 {
                let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                return String::from_utf8(buf[..end].to_vec()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "pty slave name not UTF-8")
                });
            }
            last_err = Some(io::Error::last_os_error());
        }
        // Last resort: libc's own ptsname(3), which issues the same ioctl
        // with whatever encoding this SDK ships.
        // SAFETY: plain C call; the returned pointer is a static buffer we
        // immediately copy out of. Only pane spawn (one UI-thread caller)
        // reaches here, so there is no concurrent writer to race with.
        let name = unsafe { libc::ptsname(master) };
        if !name.is_null() {
            // SAFETY: non-null, NUL-terminated C string owned by libc.
            return Ok(unsafe { std::ffi::CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned());
        }
        Err(last_err
            .unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "ptsname returned NULL")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut name = [0 as libc::c_char; 64];
        // SAFETY: plain C call with a valid pointer/len.
        if unsafe { libc::ptsname_r(master, name.as_mut_ptr(), name.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        String::from_utf8(name[..end].iter().map(|c| *c as u8).collect())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "pty slave name not UTF-8"))
    }
}

/// Allocate a pty pair with O_CLOEXEC on both ends.
fn open_pty_pair(winsize: &libc::winsize) -> Result<(RawFd, RawFd)> {
    // SAFETY: plain C pty calls, all before fork.
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
        if master < 0 {
            return Err(io::Error::last_os_error()).context("posix_openpt");
        }
        let fail = |m: RawFd| {
            libc::close(m);
            io::Error::last_os_error()
        };
        if libc::grantpt(master) != 0 {
            return Err(fail(master)).context("grantpt");
        }
        if libc::unlockpt(master) != 0 {
            return Err(fail(master)).context("unlockpt");
        }
        let slave_name = match pty_slave_name(master) {
            Ok(n) => n,
            Err(e) => {
                libc::close(master);
                return Err(e).context("pty_slave_name");
            }
        };
        let slave_path = match CString::new(slave_name) {
            Ok(p) => p,
            Err(_) => return Err(fail(master)).context("pty slave path contains NUL"),
        };
        let slave = libc::open(
            slave_path.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        );
        if slave < 0 {
            return Err(fail(master)).context("open pty slave");
        }
        if libc::ioctl(slave, libc::TIOCSWINSZ, winsize) != 0 {
            libc::close(slave);
            return Err(fail(master)).context("TIOCSWINSZ");
        }
        Ok((master, slave))
    }
}

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
