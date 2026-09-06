//! UDS control server.
//!
//! The listener thread never touches app state: requests cross an mpsc
//! channel as [`Command`]s and are serviced on the UI thread by [`drain`],
//! which sends the response back over the per-request reply channel. No
//! context wakeup is needed (a detached `egui::Context` cannot wake the
//! real UI anyway): render/screen.rs repaints unconditionally every 50 ms,
//! so `drain` runs at >=20 Hz and the 5 s reply timeout is ample.

use std::io::{BufRead, BufReader, Read, Take, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::{info, warn};

/// Longest request line accepted (1 MiB): effectively a write-payload cap.
const MAX_LINE: usize = 1024 * 1024;
/// How long the listener waits for the UI thread to answer a request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Accept-poll cadence; the stop flag is checked at the same rate.
const POLL: Duration = Duration::from_millis(50);

/// One client request plus the channel its answer goes back on.
pub(crate) struct Command {
    pub req: ipc_proto::Request,
    pub reply: Sender<ipc_proto::Response>,
}

/// Running control socket: the UI-side inbox plus the listener thread.
pub struct Ipc {
    rx: Receiver<Command>,
    sock: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Socket path: `$XDG_RUNTIME_DIR/terminator-rust/ipc.sock`, falling back
/// to `~/.config/terminator-rust/ipc.sock`. The directory is created
/// best-effort (ignored on failure).
pub fn socket_path() -> PathBuf {
    let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("terminator-rust"),
        _ => match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h).join(".config").join("terminator-rust"),
            _ => PathBuf::from(".config").join("terminator-rust"),
        },
    };
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ipc.sock")
}

/// Bind the socket locked to the owner (0600): capture/send over it is
/// full remote control of the terminal, and the `~/.config` fallback dir
/// is group/world-traversable on common distros. Fail-closed: a chmod
/// failure leaves no socket behind.
fn bind_private(path: &Path) -> std::io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        Ok(()) => Ok(listener),
        Err(e) => {
            drop(listener);
            let _ = std::fs::remove_file(path);
            Err(e)
        }
    }
}

/// Bind the control socket and start the listener thread. Best-effort:
/// returns `None` (and logs) when another instance already owns the socket
/// or anything else fails; the app runs fine without IPC.
pub fn start() -> Option<Ipc> {
    let path = socket_path();
    // Probe: a live listener answers, a dead one leaves a stale file.
    if UnixStream::connect(&path).is_ok() {
        warn!("ipc: socket in use, control disabled");
        return None;
    }
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = match bind_private(&path) {
        Ok(l) => l,
        Err(e) => {
            warn!("ipc: bind {}: {e}, control disabled", path.display());
            return None;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        warn!("ipc: nonblocking: {e}, control disabled");
        return None;
    }
    let (tx, rx) = mpsc::channel::<Command>();
    let stop = Arc::new(AtomicBool::new(false));
    // Later-spawned local panes inherit the env (pty.rs passes the parent
    // environment through), so shell helpers can find the control socket.
    // Set it before anything else spawns so no pane can race past it.
    std::env::set_var(ipc_proto::ENV_SOCKET, &path);
    let spawned = thread::Builder::new()
        .name("ipc-socket".to_string())
        .spawn({
            let stop = Arc::clone(&stop);
            move || listen(listener, tx, stop)
        })
        .map_err(|e| warn!("ipc: spawn listener: {e}, control disabled"));
    let thread = match spawned {
        Ok(t) => t,
        Err(()) => return None,
    };
    info!("ipc: control socket at {}", path.display());
    Some(Ipc {
        rx,
        sock: path,
        stop,
        thread: Some(thread),
    })
}

/// Accept loop: parks on `WouldBlock` in 50 ms slices so `Drop` can stop it.
fn listen(listener: UnixListener, tx: Sender<Command>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => serve(stream, &tx),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(e) => {
                warn!("ipc: accept: {e}");
                thread::sleep(POLL);
            }
        }
    }
}

/// Read one capped JSON request line, hand it to the UI thread and write
/// the response back as one JSON line.
fn serve(stream: UnixStream, tx: &Sender<Command>) {
    // Accepted sockets may inherit non-blocking mode; blocking + timeouts
    // is what a request/response exchange wants.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let resp = match read_request(&stream) {
        Some(req) => forward(req, tx),
        None => ipc_proto::Response::err("bad request"),
    };
    if let Ok(mut line) = serde_json::to_string(&resp) {
        line.push('\n');
        let _ = (&stream).write_all(line.as_bytes());
    }
}

/// One trimmed request line, parsed; `None` on read error / oversize /
/// empty / malformed JSON.
fn read_request(stream: &UnixStream) -> Option<ipc_proto::Request> {
    let mut line = String::new();
    // +1 makes an exactly-at-cap line legal and an over-cap one fail below.
    let capped: Take<&UnixStream> = stream.take((MAX_LINE + 1) as u64);
    let mut reader = BufReader::new(capped);
    reader.read_line(&mut line).ok()?;
    if line.len() > MAX_LINE {
        return None;
    }
    serde_json::from_str(line.trim_end()).ok()
}

/// Send the request to the UI thread and wait for the answer (the UI
/// loop drains continuously; see the module doc).
fn forward(req: ipc_proto::Request, tx: &Sender<Command>) -> ipc_proto::Response {
    let (reply, rx) = mpsc::channel();
    if tx.send(Command { req, reply }).is_err() {
        return ipc_proto::Response::err("app shutting down");
    }
    match rx.recv_timeout(REPLY_TIMEOUT) {
        Ok(resp) => resp,
        Err(_) => ipc_proto::Response::err("app not responding"),
    }
}

/// Service pending requests on the UI thread; called once per frame.
pub fn drain(ipc: &mut Ipc, data: &mut crate::state::Data) {
    while let Ok(cmd) = ipc.rx.try_recv() {
        let resp = crate::ipc::handle::execute(cmd.req, data);
        let _ = cmd.reply.send(resp);
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The accept loop re-checks the flag every 50 ms, so join returns.
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.sock);
    }
}
