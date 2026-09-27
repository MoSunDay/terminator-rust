use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::protocol::{self, Summary};

static RESIZED: AtomicBool = AtomicBool::new(true);

extern "C" fn winch(_: libc::c_int) {
    RESIZED.store(true, Ordering::Relaxed);
}

struct RawTerminal(Option<libc::termios>);
impl RawTerminal {
    fn enter(fd: RawFd) -> io::Result<Self> {
        if unsafe { libc::isatty(fd) } != 1 {
            return Ok(Self(None));
        }
        let mut old = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut old) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = old;
        unsafe {
            libc::cfmakeraw(&mut raw);
        }
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(Some(old)))
    }
}
impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Some(old) = &self.0 {
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, old);
            }
        }
    }
}

fn terminal_size(fd: RawFd) -> (u16, u16) {
    let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ as _, &mut size) } < 0 {
        return (80, 24);
    }
    (size.ws_col.max(1), size.ws_row.max(1))
}

fn connect_or_start(name: &str, title: &str) -> Result<UnixStream> {
    let path = crate::path::socket_path(name)?;
    let lock_path = crate::path::session_dir()?.join(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } < 0 {
        return Err(io::Error::last_os_error()).context("lock session");
    }
    match UnixStream::connect(&path) {
        Ok(stream) => return Ok(stream),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => return Err(e).with_context(|| format!("connect {}", path.display())),
    }
    if fs::symlink_metadata(&path).is_ok() {
        let kind = fs::symlink_metadata(&path)?.file_type();
        if !kind.is_socket() {
            bail!("session path is not a socket: {}", path.display());
        }
        fs::remove_file(&path)?;
    }
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.args(["__serve", name, title, &cwd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    cmd.spawn().context("start session keeper")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(stream) = UnixStream::connect(&path) {
            return Ok(stream);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    bail!("session keeper did not start: {}", path.display())
}

pub fn attach(name: &str, title: &str) -> Result<()> {
    let mut stream = connect_or_start(name, title)?;
    stream.write_all(&[protocol::ATTACH])?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let _raw = RawTerminal::enter(input.as_raw_fd())?;
    unsafe {
        libc::signal(libc::SIGWINCH, winch as *const () as libc::sighandler_t);
    }
    loop {
        if RESIZED.swap(false, Ordering::Relaxed) {
            let (cols, rows) = terminal_size(input.as_raw_fd());
            stream.write_all(&protocol::resize_packet(cols, rows))?;
        }
        let mut fds = [
            libc::pollfd {
                fd: input.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 200) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.into());
        }
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut bytes = [0u8; protocol::MAX_INPUT];
            match input.read(&mut bytes) {
                Ok(0) => return Ok(()),
                Ok(n) => stream.write_all(&protocol::input_packet(&bytes[..n]))?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            let mut bytes = [0u8; 8192];
            match stream.read(&mut bytes) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    output.write_all(&bytes[..n])?;
                    output.flush()?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
}

fn query(mut stream: UnixStream) -> Result<Summary> {
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(&[protocol::QUERY])?;
    let mut line = String::new();
    stream.read_to_string(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

pub fn list() -> Result<()> {
    let dir = crate::path::session_dir()?;
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().is_none_or(|e| e != "sock") {
            continue;
        }
        if let Ok(stream) = UnixStream::connect(entry.path()) {
            if let Ok(info) = query(stream) {
                found.push(info);
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    println!("{:<24} {:<10} {:<36} TITLE", "SESSION", "STATE", "CWD");
    for item in &found {
        println!("{}", protocol::list_line(item));
    }
    Ok(())
}
