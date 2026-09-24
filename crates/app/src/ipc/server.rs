//! UDS control server.
//!
//! The listener thread never touches app state: requests cross an mpsc
//! channel as [`Command`]s and are serviced on the UI thread by [`drain`],
//! which sends the response back over the per-request reply channel. No
//! context wakeup is needed (a detached `egui::Context` cannot wake the
//! real UI anyway): render/screen.rs repaints unconditionally every 50 ms,
//! so `drain` runs at >=20 Hz and the 5 s reply timeout is ample.
//!
//! Migration requests carry more than a line: the sender attaches one
//! SCM_RIGHTS fd per pane leaf to the header message and follows it with
//! length-prefixed snapshot blocks; [`read_request`] collects all three
//! parts before the [`Command`] crosses to the UI thread.

use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::{info, warn};

use super::fd;

/// Longest request line accepted (1 MiB): effectively a write-payload cap.
const MAX_LINE: usize = 1024 * 1024;
/// First-chunk read size for the fd-carrying message.
const RECV_CHUNK: usize = 64 * 1024;
/// Per-request read timeout (header line + migration payload): a 32 MiB
/// snapshot still streams through a local socket well inside it.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the listener waits for the UI thread to answer a request.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Accept-poll cadence; the stop flag is checked at the same rate.
const POLL: Duration = Duration::from_millis(50);

/// One client request plus the channel its answer goes back on. Migration
/// attachments ride along: `fds` (one adopted PTY per depth-first leaf)
/// and `payload` (the per-leaf snapshot blocks); both empty otherwise.
pub(crate) struct Command {
    pub req: ipc_proto::Request,
    pub reply: Sender<ipc_proto::Response>,
    pub fds: Vec<OwnedFd>,
    pub payload: Vec<u8>,
}

/// Running control socket: the UI-side inbox plus the listener thread.
pub struct Ipc {
    rx: Receiver<Command>,
    sock: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Socket path policy: `$TERMINATOR_SOCK` when set (bind exactly there),
/// else `ipc.sock` under the runtime dir (`$XDG_RUNTIME_DIR/terminator-rust`,
/// falling back to the config root; created best-effort by
/// [`paths::runtime_dir`]).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os(ipc_proto::ENV_SOCKET) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    paths::runtime_dir().join("ipc.sock")
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

/// Probe-and-bind at one path: a connect that gets answered means a live
/// owner (this path is taken), a leftover file is a stale socket from a
/// SIGTERM'd instance and is reclaimed. The listener is returned in
/// non-blocking mode for the accept-poll loop.
fn bind_at(path: &Path) -> Option<UnixListener> {
    if UnixStream::connect(path).is_ok() {
        warn!("ipc: {} in use by a live instance", path.display());
        return None;
    }
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let listener = bind_private(path)
        .map_err(|e| {
            warn!("ipc: bind {}: {e}", path.display());
        })
        .ok()?;
    if let Err(e) = listener.set_nonblocking(true) {
        warn!("ipc: nonblocking: {e}");
        return None;
    }
    Some(listener)
}

/// Bind the control socket and start the listener thread. Candidate
/// paths, in order: an explicit `$TERMINATOR_SOCK` (bind exactly there),
/// the shared `ipc.sock` under the runtime dir, then a per-instance
/// `ipc-<pid>.sock` — pane children inherit `TERMINATOR_SOCK`, so a
/// nested launch must not lose IPC just because the parent owns the
/// shared name. Best-effort throughout: `None` (and logs) when nothing
/// binds; the app runs fine without IPC.
pub fn start() -> Option<Ipc> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os(ipc_proto::ENV_SOCKET) {
        if !p.is_empty() {
            candidates.push(PathBuf::from(p));
        }
    }
    candidates.push(socket_path());
    candidates.push(paths::runtime_dir().join(format!("ipc-{}.sock", std::process::id())));
    candidates.dedup();
    for path in candidates {
        if let Some(ipc) = start_at(path) {
            return Some(ipc);
        }
    }
    warn!("ipc: no bindable control socket, control disabled");
    None
}

/// Core of [`start`] at one fixed path; split out so tests can pick their
/// own socket location. `None` when the path is owned by a live instance
/// or anything else fails on the way to a running listener.
pub(crate) fn start_at(path: PathBuf) -> Option<Ipc> {
    let listener = bind_at(&path)?;
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

/// Read one request exchange, hand it to the UI thread and write the
/// response back as one JSON line.
fn serve(stream: UnixStream, tx: &Sender<Command>) {
    // Accepted sockets may inherit non-blocking mode; blocking + timeouts
    // is what a request/response exchange wants.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let resp = match read_request(&stream) {
        Some((req, fds, payload)) => forward(req, fds, payload, tx),
        None => ipc_proto::Response::err("bad request"),
    };
    if let Ok(mut line) = serde_json::to_string(&resp) {
        line.push('\n');
        let _ = (&stream).write_all(line.as_bytes());
    }
}

/// One full request read: the fd-carrying first chunk (SCM_RIGHTS rides
/// the first message only), the newline-terminated JSON header, and — for
/// `tab_offer` — `leaf_count(tab.root)` length-prefixed snapshot blocks
/// following the header line (leftover first-chunk bytes count first).
///
/// `None` on read error / timeout / oversize / truncation / bad JSON; any
/// fds already received are closed by the drop on those paths, and stray
/// fds on a non-migration request are dropped with a warning.
fn read_request(stream: &UnixStream) -> Option<(ipc_proto::Request, Vec<OwnedFd>, Vec<u8>)> {
    let mut first = [0u8; RECV_CHUNK];
    let (n, fds) =
        fd::recv_with_fds(stream, &mut first, ipc_proto::migrate::MAX_MIGRATE_PANES).ok()?;
    let mut data = first[..n].to_vec();
    let mut pos = 0usize;
    // Header line: scan for '\n', plain-reading more as needed. Ancillary
    // data only ever arrives with the first message.
    let nl = loop {
        if let Some(i) = data[pos..].iter().position(|&b| b == b'\n') {
            break pos + i;
        }
        if data.len() - pos > MAX_LINE {
            return None;
        }
        let want = data.len() - pos + 1;
        let mut scan = pos;
        if !fill_exact(stream, &mut data, &mut scan, want) {
            return None;
        }
    };
    let line = std::str::from_utf8(&data[pos..nl]).ok()?;
    pos = nl + 1;
    let req: ipc_proto::Request = serde_json::from_str(line.trim_end()).ok()?;
    let leaves = match &req {
        ipc_proto::Request::TabOffer { tab } => ipc_proto::migrate::leaf_count(&tab.root),
        _ => {
            if !fds.is_empty() {
                warn!(
                    "ipc: {} fds attached to a non-migration request, dropped",
                    fds.len()
                );
            }
            return Some((req, Vec::new(), Vec::new()));
        }
    };
    // Snapshot blocks in wire form (4-byte LE length + bytes per leaf),
    // forwarded verbatim for `migrate::decode_payload`; capped overall so
    // a bogus length cannot drive a multi-GiB buffer.
    let payload_start = pos;
    for _ in 0..leaves {
        if !fill_exact(stream, &mut data, &mut pos, 4) {
            return None;
        }
        let len =
            u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos - payload_start + len > ipc_proto::migrate::MAX_MIGRATE_PAYLOAD {
            return None;
        }
        if !fill_exact(stream, &mut data, &mut pos, len) {
            return None;
        }
        pos += len;
    }
    Some((req, fds, data[payload_start..pos].to_vec()))
}

/// Plain-read continuation (no fds past the first chunk) until at least
/// `want` bytes are buffered past `pos`; `false` on EOF, error or timeout.
/// `EINTR` is retried; chunked reads keep memory bounded for big payloads.
fn fill_exact(stream: &UnixStream, data: &mut Vec<u8>, pos: &mut usize, want: usize) -> bool {
    // `Read` is implemented for `&UnixStream`, so a shared reborrow reads.
    let mut rd = stream;
    while data.len() - *pos < want {
        let mut chunk = [0u8; 8192];
        match rd.read(&mut chunk) {
            Ok(0) => return false,
            Ok(k) => data.extend_from_slice(&chunk[..k]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

/// Send the request to the UI thread and wait for the answer (the UI
/// loop drains continuously; see the module doc).
fn forward(
    req: ipc_proto::Request,
    fds: Vec<OwnedFd>,
    payload: Vec<u8>,
    tx: &Sender<Command>,
) -> ipc_proto::Response {
    let (reply, rx) = mpsc::channel();
    if tx
        .send(Command {
            req,
            reply,
            fds,
            payload,
        })
        .is_err()
    {
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
        let resp = crate::ipc::handle::execute(cmd.req, cmd.fds, cmd.payload, data);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Unique socket path per test (short: `sun_path` caps at ~104 bytes).
    fn tmpsock(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "trn-ipc-{}-{}-{tag}.sock",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    fn pane_leaf() -> ipc_proto::migrate::MigrateNode {
        ipc_proto::migrate::MigrateNode::Pane {
            pane: ipc_proto::migrate::MigratePane {
                manual_title: Some("src".into()),
                kind: "local".into(),
                degraded: false,
                pid: 4242,
                cols: 80,
                rows: 24,
            },
        }
    }

    /// Two distinct open fds (dups of /dev/null).
    fn two_fds() -> Vec<OwnedFd> {
        let f = OwnedFd::from(std::fs::File::open("/dev/null").unwrap());
        vec![f.try_clone().unwrap(), f]
    }

    /// Serialize `req` as the header line, no payload.
    fn line(req: &ipc_proto::Request) -> Vec<u8> {
        let mut bytes = serde_json::to_string(req).unwrap().into_bytes();
        bytes.push(b'\n');
        bytes
    }

    /// Take one forwarded command and answer it so `serve` unblocks
    /// promptly (the real UI thread would `drain` it within 50 ms).
    fn take(ipc: &Ipc) -> Command {
        ipc.rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request forwarded to the UI inbox")
    }

    #[test]
    fn start_at_reclaims_stale_and_rejects_live_double_bind() {
        let path = tmpsock("bind");
        // Stale file from a SIGTERM'd instance: reclaimed on bind.
        std::fs::write(&path, b"").unwrap();
        let first = start_at(path.clone()).expect("stale socket reclaimed");
        assert!(path.exists());
        // A live owner answers the probe: a second bind must fail.
        assert!(start_at(path.clone()).is_none(), "double bind refused");
        // Dropping the first removes its socket file; a rebind works.
        drop(first);
        let second = start_at(path.clone()).expect("rebind after drop");
        drop(second);
        assert!(!path.exists(), "Drop removes the socket file");
    }

    #[test]
    fn tab_offer_roundtrips_fds_and_payload() {
        let path = tmpsock("offer");
        let ipc = start_at(path.clone()).expect("start_at binds");
        let tab = ipc_proto::migrate::MigrateTab {
            title: "work".into(),
            focused: None,
            root: pane_leaf(),
        };
        let payload = ipc_proto::migrate::encode_payload(&[Some(vec![7u8, 8, 9])]);
        let mut msg = line(&ipc_proto::Request::TabOffer { tab });
        msg.extend_from_slice(&payload);
        let stream = UnixStream::connect(&path).unwrap();
        fd::send_with_fds(&stream, &msg, &two_fds()).unwrap();

        let cmd = take(&ipc);
        match cmd.req {
            ipc_proto::Request::TabOffer { tab } => assert_eq!(tab.title, "work"),
            other => panic!("expected tab_offer, got {other:?}"),
        }
        assert_eq!(cmd.fds.len(), 2, "both SCM_RIGHTS fds forwarded");
        assert_eq!(cmd.payload, payload, "snapshot blocks forwarded verbatim");
        // Distinct open descriptors reached the UI side of the channel.
        assert_ne!(cmd.fds[0].as_raw_fd(), cmd.fds[1].as_raw_fd());

        let _ = cmd.reply.send(ipc_proto::Response::Migrated { panes: 1 });
        let mut reply = String::new();
        let mut reader = std::io::BufReader::new(&stream);
        reader.read_line(&mut reply).unwrap();
        assert!(reply.contains("migrated"), "reply line: {reply}");
    }

    #[test]
    fn plain_request_carries_no_fds() {
        let path = tmpsock("plain");
        let ipc = start_at(path.clone()).expect("start_at binds");
        let mut stream = UnixStream::connect(&path).unwrap();
        stream.write_all(b"{\"cmd\":\"list\"}\n").unwrap();

        let cmd = take(&ipc);
        assert!(matches!(cmd.req, ipc_proto::Request::List));
        assert!(cmd.fds.is_empty());
        assert!(cmd.payload.is_empty());
        let _ = cmd.reply.send(ipc_proto::Response::List { panes: vec![] });
        let mut reply = String::new();
        let mut reader = std::io::BufReader::new(&stream);
        reader.read_line(&mut reply).unwrap();
        assert!(reply.contains("\"list\""), "reply line: {reply}");
    }

    #[test]
    fn truncated_tab_offer_does_not_kill_the_listener() {
        let path = tmpsock("trunc");
        let ipc = start_at(path.clone()).expect("start_at binds");
        let tab = ipc_proto::migrate::MigrateTab {
            title: "t".into(),
            focused: None,
            root: ipc_proto::migrate::MigrateNode::Split {
                axis: "h".into(),
                ratio: 0.5,
                first: Box::new(pane_leaf()),
                second: Box::new(pane_leaf()),
            },
        };
        // Header promises 2 leaves; only 1 snapshot block is sent, then
        // the client vanishes: serve must error out without panicking.
        let mut msg = line(&ipc_proto::Request::TabOffer { tab });
        msg.extend_from_slice(&ipc_proto::migrate::encode_payload(&[Some(vec![1u8])]));
        let mut stream = UnixStream::connect(&path).unwrap();
        stream.write_all(&msg).unwrap();
        drop(stream);

        // The listener survives and serves the next client.
        let mut next = UnixStream::connect(&path).unwrap();
        next.write_all(b"{\"cmd\":\"list\"}\n").unwrap();
        let cmd = take(&ipc);
        assert!(matches!(cmd.req, ipc_proto::Request::List));
        let _ = cmd.reply.send(ipc_proto::Response::err("test"));
    }
}
