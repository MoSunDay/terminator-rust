use std::ffi::c_int;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};

pub struct Shell {
    pub master: File,
    pub child: Child,
}

fn duplicate(fd: c_int) -> io::Result<File> {
    let copy = unsafe { libc::dup(fd) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(copy) })
}

pub fn spawn(cwd: &str, session: &str) -> Result<Shell> {
    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            // libc 0.2.189 declares `winp` as *const winsize on linux
            // but *mut winsize on macOS: a raw *mut satisfies both
            // (implicit *mut -> *const weakening) and stays
            // clippy-clean (`&mut` trips unnecessary_mut_passed on
            // linux, `&` fails to compile for the macOS target).
            &raw mut size,
        )
    } < 0
    {
        return Err(io::Error::last_os_error()).context("openpty");
    }
    let master = unsafe { File::from_raw_fd(master) };
    let slave = unsafe { File::from_raw_fd(slave) };
    for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error()).context("set pty close-on-exec");
        }
    }
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut cmd = Command::new(shell);
    cmd.arg("-i")
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        // Identifies this keeper session to any child running in the
        // shell (documented environment contract, not a command route).
        .env(ipc_proto::ENV_SESSION, session)
        .env_remove("NO_COLOR")
        .stdin(Stdio::from(duplicate(slave.as_raw_fd())?))
        .stdout(Stdio::from(duplicate(slave.as_raw_fd())?))
        .stderr(Stdio::from(duplicate(slave.as_raw_fd())?));
    let slave_fd = slave.as_raw_fd();
    unsafe {
        cmd.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
            Ok(())
        });
    }
    let child = cmd.spawn().context("spawn shell")?;
    drop(slave);
    set_nonblocking(master.as_raw_fd())?;
    Ok(Shell { master, child })
}

fn set_nonblocking(fd: c_int) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn resize(master: &File, cols: u16, rows: u16) -> io::Result<()> {
    let size = libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
