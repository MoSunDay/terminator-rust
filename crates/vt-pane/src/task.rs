//! Session wiring: pty reader thread, terminal state, key encoding.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use libghostty_vt::key::{Action, Encoder as KeyEncoder, Event as KeyEvent, Mods};
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::terminal::Mode;
use libghostty_vt::Terminal;

use crate::pty::{self, PtyHandle};
use crate::effects::{self, CellPx};
use crate::term::{snapshot_frame, Frame};

/// Events delivered by the pty reader thread.
#[derive(Debug)]
pub enum PtyEvent {
    /// Bytes read from the pty master.
    Output(Vec<u8>),
    /// Child exited with this status (also set when the master hits EIO/EOF).
    Exit(i32),
}

/// How to spawn a pane's process.
#[derive(Debug, Clone)]
pub struct SessionOpts {
    pub cols: u16,
    pub rows: u16,
    pub argv: Vec<String>,
    /// Extra `KEY=VALUE` env entries layered over the inherited environment.
    pub env: Vec<String>,
    /// Scrollback size in lines.
    pub scrollback_lines: usize,
    /// Answers light/dark for terminal color-scheme queries
    /// (CSI ? 996 n); linked to the active theme.
    pub dark: bool,
}

impl SessionOpts {
    /// Options for the user's default shell (`$SHELL`, fallback `sh -i`).
    pub fn local_shell(cols: u16, rows: u16) -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        SessionOpts {
            cols,
            rows,
            argv: vec![shell, "-i".to_string()],
            env: Vec::new(),
            scrollback_lines: 10_000,
            dark: true,
        }
    }

    /// Options for a raw command (used by the remote/zellij integration).
    pub fn command(cols: u16, rows: u16, argv: Vec<String>) -> Self {
        SessionOpts {
            cols,
            rows,
            argv,
            env: Vec::new(),
            scrollback_lines: 10_000,
            dark: true,
        }
    }
}

/// One terminal pane: pty + terminal + render helpers.
///
/// The reader thread owns nothing but a pty fd clone; all state lives here
/// and is only touched from the UI thread. The reader thread is the sole
/// closer of the master fd and publishes that fact via `closed` before
/// closing, so late writers (the pty-write effect, `write`) can bail out
/// instead of touching a reused fd number.
pub struct Session {
    handle: PtyHandle,
    events: Receiver<PtyEvent>,
    /// Set by `terminate`; the reader thread polls it between reads.
    stop: Arc<AtomicBool>,
    /// Set by the reader thread right before it closes the master fd;
    /// guards the UI-thread write path against the reused fd number.
    closed: Arc<AtomicBool>,
    pub term: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
    key_encoder: KeyEncoder<'static>,
    key_event: KeyEvent<'static>,
    /// Non-empty once the child has exited / the master closed.
    pub exit: Option<i32>,
    /// Current cell pixel size, shared with the size-query effect.
    cell_px: CellPx,
    /// Mouse encoder + selection gesture objects (see `mouse`).
    pub(crate) pointer: crate::mouse::PointerState,
}

/// Spawn a session for the given options.
pub fn spawn_session(opts: &SessionOpts) -> Result<Session> {
    if opts.argv.is_empty() {
        bail!("session argv is empty");
    }
    let argv: Vec<&str> = opts.argv.iter().map(String::as_str).collect();
    let handle = pty::open_pty(opts.cols, opts.rows, &argv, &opts.env).context("spawning pty")?;

    let mut term = Terminal::new(opts.cols, opts.rows)?;
    term.set_scrollback_max_lines(Some(opts.scrollback_lines))?;

    // Closed-guard: the reader thread is the sole closer of the master fd.
    // It flips `closed` before closing, so query-response writes (issued
    // synchronously from vt_write effects) skip the fd instead of racing
    // with a possible fd-number reuse by a newer pane's pty.
    let closed = Arc::new(AtomicBool::new(false));
    let write_fd = handle.master_fd;
    let write_closed = Arc::clone(&closed);
    // Both captures are Copy/Arc, so 'static holds.
    term.on_pty_write(move |_t, data| {
        if write_closed.load(Ordering::Acquire) {
            return; // fd already closed by the reader thread
        }
        let _ = pty::pty_write(write_fd, data);
    })?;
    let cell_px = effects::new_cell_px();
    effects::install(&mut term, Arc::clone(&cell_px), opts.dark)?;

    let (tx, rx) = channel::<PtyEvent>();
    let read_fd = handle.master_fd;
    let pid = handle.child_pid;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&stop);
    let closed_reader = Arc::clone(&closed);
    let reader = thread::Builder::new()
        .name("pty-reader".to_string())
        .spawn(move || reader_loop(read_fd, pid, stop_reader, closed_reader, tx));
    match reader {
        Ok(_) => {}
        Err(e) => {
            // No reader thread will own or close the fd: clean up the
            // just-forked child and the master fd ourselves.
            signal_group(handle.child_pid, libc::SIGKILL);
            // SAFETY: plain C close.
            unsafe {
                libc::close(handle.master_fd);
            }
            return Err(e).context("spawning reader thread");
        }
    }

    Ok(Session {
        handle,
        events: rx,
        stop,
        closed,
        term,
        render_state: RenderState::new()?,
        row_it: RowIterator::new()?,
        cell_it: CellIterator::new()?,
        key_encoder: KeyEncoder::new()?,
        key_event: KeyEvent::new()?,
        exit: None,
        cell_px,
        pointer: crate::mouse::new_pointer_state().context("pointer state")?,
    })
}

/// Reader loop for the pty master; runs on its own thread.
///
/// Polls with a bounded timeout so a `terminate` request (stop flag) is
/// noticed within 200ms; a blocking read could never be woken. This thread
/// is the sole closer of the master fd: it stores into `closed` (Release)
/// immediately before the close so the pty-write guard sees it before the
/// fd number can be handed out again.
fn reader_loop(
    fd: i32,
    pid: i32,
    stop: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    tx: std::sync::mpsc::Sender<PtyEvent>,
) {
    let mut buf = [0u8; 8192];
    loop {
        if stop.load(Ordering::Relaxed) {
            break; // terminated from the UI side: no exit event needed
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll on one valid fd with a bounded timeout.
        let rc = unsafe { libc::poll(&mut pfd, 1, 200) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            let _ = tx.send(PtyEvent::Exit(-1));
            break;
        }
        if rc == 0 {
            continue; // poll timeout: re-check the stop flag
        }
        // SAFETY: plain C read on an int fd.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            if tx
                .send(PtyEvent::Output(buf[..n as usize].to_vec()))
                .is_err()
            {
                break; // receiver dropped: session is gone
            }
            continue;
        }
        if n == 0 || unsafe { *libc::__errno_location() } == libc::EIO {
            // EOF or EIO: child side closed.
            let status = pty::pty_wait(pid, false).ok().flatten().unwrap_or(0);
            let _ = tx.send(PtyEvent::Exit(status));
            break;
        }
        if unsafe { *libc::__errno_location() } == libc::EINTR {
            continue;
        }
        let status = pty::pty_wait(pid, false).ok().flatten().unwrap_or(-1);
        let _ = tx.send(PtyEvent::Exit(status));
        break;
    }
    // Publish the close BEFORE dropping the fd, so guarded writers bail
    // out instead of racing a possible fd-number reuse.
    closed.store(true, Ordering::Release);
    // SAFETY: plain C close; this thread is the fd's last owner.
    unsafe { libc::close(fd) };
}

/// Ask a session's child to die and unwind its reader thread.
///
/// SIGHUPs the child's whole process group (the child is a session leader
/// from `setsid`, so the group covers the shell and its jobs), flags the
/// reader for shutdown — it closes the master fd itself within its poll
/// timeout — and escalates to SIGKILL after one second if the group has
/// not exited by then. Safe to call on already-dead sessions.
pub fn terminate(sess: &mut Session) {
    sess.stop.store(true, Ordering::Relaxed);
    let pid = sess.handle.child_pid;
    if pid <= 0 {
        return;
    }
    signal_group(pid, libc::SIGHUP);
    // Watchdog for SIGHUP-resistant children; reaps the child either way.
    let spawned = thread::Builder::new()
        .name("pty-kill".to_string())
        .spawn(move || {
            for _ in 0..50 {
                thread::sleep(Duration::from_millis(20));
                if matches!(pty::pty_wait(pid, true), Ok(Some(_))) {
                    return;
                }
            }
            signal_group(pid, libc::SIGKILL);
            let _ = pty::pty_wait(pid, false);
        })
        .is_ok();
    if !spawned {
        // No watchdog possible: escalate right away instead.
        signal_group(pid, libc::SIGKILL);
    }
}

/// Signal the child's process group, falling back to the bare pid.
fn signal_group(pid: i32, sig: i32) {
    // SAFETY: plain C calls; errors (e.g. ESRCH) are ignored on purpose.
    unsafe {
        if libc::kill(-pid, sig) < 0 {
            let _ = libc::kill(pid, sig);
        }
    }
}

/// Drain pty events into the terminal. Call once per UI frame.
pub fn pump(sess: &mut Session) -> Result<()> {
    loop {
        match sess.events.try_recv() {
            Ok(PtyEvent::Output(data)) => sess.term.vt_write(&data),
            Ok(PtyEvent::Exit(status)) => {
                if sess.exit.is_none() {
                    log::debug!("pty exit: {status}");
                }
                sess.exit = Some(status);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                if sess.exit.is_none() {
                    sess.exit = Some(0);
                }
                break;
            }
        }
    }
    Ok(())
}

/// Snapshot the current terminal state into plain render data.
pub fn frame(sess: &mut Session) -> Result<Frame> {
    snapshot_frame(
        &mut sess.term,
        &mut sess.render_state,
        &mut sess.row_it,
        &mut sess.cell_it,
    )
}

/// Resize terminal + pty window. `cell_{w,h}_px` are font cell metrics.
pub fn resize(
    sess: &mut Session,
    cols: u16,
    rows: u16,
    cell_w_px: u32,
    cell_h_px: u32,
) -> Result<()> {
    if cols == 0 || rows == 0 {
        return Ok(());
    }
    sess.term.resize(cols, rows, cell_w_px, cell_h_px)?;
    // Keep the size-query effect answering with live geometry.
    *sess.cell_px.lock().unwrap_or_else(|e| e.into_inner()) = (cell_w_px, cell_h_px);
    pty::pty_resize(sess.handle.master_fd, cols, rows)
}

/// Current cell pixel metrics (mouse/selection geometry).
pub fn cell_px(sess: &Session) -> (u32, u32) {
    *sess.cell_px.lock().unwrap_or_else(|e| e.into_inner())
}

/// Pid of the session's child (0 when unknown).
pub fn child_pid(sess: &Session) -> i32 {
    sess.handle.child_pid
}

/// Write raw bytes to the pty.
///
/// A no-op Ok once the reader thread has closed the master fd (the fd
/// number may have been reused by another pane's pty by then).
/// `send_key`/`paste` route through here, so they are covered too.
pub fn write(sess: &Session, data: &[u8]) -> Result<()> {
    if sess.closed.load(Ordering::Acquire) {
        return Ok(());
    }
    pty::pty_write(sess.handle.master_fd, data)
}

/// Current OSC 0/2 title ("" when unset).
pub fn title(sess: &Session) -> String {
    sess.term.title().map(str::to_string).unwrap_or_default()
}

/// Send one key event through the ghostty encoder into the pty.
///
/// `configure` fills in action/key/mods/text on the reusable event object.
pub fn send_key<F>(sess: &mut Session, configure: F) -> Result<()>
where
    F: FnOnce(&mut KeyEvent),
{
    // Start from a neutral event each time.
    sess.key_event.set_action(Action::Press);
    sess.key_event
        .set_key(libghostty_vt::key::Key::Unidentified);
    sess.key_event.set_mods(Mods::empty());
    sess.key_event.set_utf8::<&str>(None);
    configure(&mut sess.key_event);

    sess.key_encoder.set_options_from_terminal(&sess.term);
    let mut out = Vec::with_capacity(32);
    sess.key_encoder.encode_to_vec(&sess.key_event, &mut out)?;
    if !out.is_empty() {
        write(sess, &out)?;
    }
    Ok(())
}

/// Paste text safely (bracketed paste when the terminal asked for it).
pub fn paste(sess: &mut Session, text: &str) -> Result<()> {
    let bracketed = sess.term.mode(Mode::BRACKETED_PASTE).unwrap_or(false);
    // paste::encode consumes `data` in place, so each attempt starts fresh.
    let mut capacity = text.len() + 32;
    loop {
        let mut data = text.as_bytes().to_vec();
        let mut buf = vec![0u8; capacity];
        match libghostty_vt::paste::encode(&mut data, bracketed, &mut buf) {
            Ok(len) => {
                buf.truncate(len);
                return write(sess, &buf);
            }
            Err(libghostty_vt::Error::OutOfSpace { required }) => {
                capacity = required + 32;
                continue;
            }
            Err(e) => return Err(e).context("paste encode"),
        }
    }
}
