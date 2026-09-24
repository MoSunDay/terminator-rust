//! Session wiring: pty reader thread, terminal state, key encoding.

use std::io;
use std::os::fd::RawFd;
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

use crate::effects::{self, CellPx};
use crate::pty::{self, PtyHandle};
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

    // The fork already happened: until the reader thread exists nobody
    // owns or closes the master fd, so EVERY failure below is cleaned up
    // at this single call site (kill child + close master). See
    // `assemble`: it spawns the reader thread LAST, so an Err from
    // it always means "no reader thread owns the fd yet".
    match build_session(opts, handle) {
        Ok(sess) => Ok(sess),
        Err(e) => {
            kill_pty_child(handle.child_pid, handle.master_fd);
            Err(e)
        }
    }
}

/// Everything after a successful `pty::open_pty`: terminal setup, the
/// fallible constructors, and finally the reader thread. All `?` returns
/// here happen while the caller still owns the master fd.
fn build_session(opts: &SessionOpts, handle: PtyHandle) -> Result<Session> {
    let term = fresh_terminal(opts)?;
    assemble(term, opts, handle)
}

/// Cap on retained VT continuation bytes: the replay-safe suffix of an
/// unfinished escape sequence / UTF-8 rune left over by the last
/// `vt_write`. Tracking is DISABLED by default, and snapshot encoding
/// only works on a mid-sequence parser when tracking was enabled BEFORE
/// the bytes that produced that state arrived - so every session turns
/// it on up front (1 MiB is far beyond any real sequence) and stays
/// snapshot-encodable for its whole life (see `snapshot`).
const SNAPSHOT_CONTINUATION_MAX: usize = 1 << 20;

/// A fresh terminal with this session's engine limits applied.
fn fresh_terminal(opts: &SessionOpts) -> Result<Terminal<'static, 'static>> {
    let mut term = Terminal::new(opts.cols, opts.rows)?;
    configure_terminal(&mut term, opts)?;
    Ok(term)
}

/// Engine-level limits every session terminal gets: the scrollback
/// ceiling and the continuation tracking snapshots depend on.
fn configure_terminal(term: &mut Terminal<'static, 'static>, opts: &SessionOpts) -> Result<()> {
    term.set_scrollback_max_lines(Some(opts.scrollback_lines))?;
    term.set_continuation_max_bytes(SNAPSHOT_CONTINUATION_MAX)?;
    Ok(())
}

/// Wire a fully-configured terminal and pty handle into a `Session`,
/// spawning the reader thread last. Every `?` return here happens while
/// the caller still owns the master fd (a spawn failure means no reader
/// thread ever owned it).
fn assemble(
    term: Terminal<'static, 'static>,
    opts: &SessionOpts,
    handle: PtyHandle,
) -> Result<Session> {
    let mut term = term;

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
    // Build every fallible field before the reader thread exists; the
    // struct literal below is infallible.
    let render_state = RenderState::new()?;
    let row_it = RowIterator::new()?;
    let cell_it = CellIterator::new()?;
    let key_encoder = KeyEncoder::new()?;
    let key_event = KeyEvent::new()?;
    let pointer = crate::mouse::new_pointer_state().context("pointer state")?;

    let (tx, rx) = channel::<PtyEvent>();
    let read_fd = handle.master_fd;
    let pid = handle.child_pid;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&stop);
    let closed_reader = Arc::clone(&closed);
    thread::Builder::new()
        .name("pty-reader".to_string())
        .spawn(move || reader_loop(read_fd, pid, stop_reader, closed_reader, tx))
        .context("spawning reader thread")?;

    Ok(Session {
        handle,
        events: rx,
        stop,
        closed,
        term,
        render_state,
        row_it,
        cell_it,
        key_encoder,
        key_event,
        exit: None,
        cell_px,
        pointer,
    })
}

/// Kill the just-forked child and close the master fd: no reader thread
/// will own or close the fd on this path, so clean up both ourselves.
/// Reaps so a retried spawn (app `ensure_sessions`) leaves no zombie.
fn kill_pty_child(pid: i32, master: RawFd) {
    signal_group(pid, libc::SIGKILL);
    // SAFETY: plain C close.
    unsafe {
        libc::close(master);
    }
    let _ = pty::pty_wait(pid, false);
}

/// Reap the child for the reader's exit event - without ever letting a
/// non-positive pid reach waitpid: waitpid(0)/waitpid(-1) would block on /
/// reap UNRELATED children of this process, so an unknown or invalid pid
/// maps straight to a failure status instead.
fn reap_exit_status(pid: i32, on_error: i32) -> i32 {
    if pid <= 0 {
        return -1;
    }
    pty::pty_wait(pid, false).ok().flatten().unwrap_or(on_error)
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
        if n == 0 || io::Error::last_os_error().raw_os_error().unwrap_or(0) == libc::EIO {
            // EOF or EIO: child side closed.
            let status = reap_exit_status(pid, 0);
            let _ = tx.send(PtyEvent::Exit(status));
            break;
        }
        if io::Error::last_os_error().raw_os_error().unwrap_or(0) == libc::EINTR {
            continue;
        }
        let status = reap_exit_status(pid, -1);
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

/// Signal the reader thread to exit at its next poll boundary (<= ~200ms).
/// Does NOT signal or close the child; the master fd stays open until the
/// reader thread closes it (it remains the sole closer).
pub fn stop_reader(sess: &Session) {
    sess.stop.store(true, Ordering::Relaxed);
}

/// True once the reader thread has closed the master fd (its `closed` flag).
pub fn reader_done(sess: &Session) -> bool {
    sess.closed.load(Ordering::Acquire)
}

/// Duplicate the pty master fd (F_DUPFD_CLOEXEC). The dup is independent of
/// the reader thread's close.
pub fn dup_master(sess: &Session) -> io::Result<std::os::fd::OwnedFd> {
    // Same guard as `write`: once the reader has closed the master, the fd
    // number may already belong to another pane's pty.
    if reader_done(sess) {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "master fd already closed by the reader thread",
        ));
    }
    pty::dup_fd(sess.handle.master_fd)
}

/// Encode the full terminal state (screen + scrollback + VT parser state).
/// Call AFTER stop_reader + a final pump. Returns Ok(None) when the engine
/// cannot encode (caller falls back to a fresh terminal).
pub fn snapshot(sess: &mut Session) -> Result<Option<Vec<u8>>> {
    // Input must already be quiesced (the caller's stop_reader + pump
    // contract): encoding walks the whole scrollback and reads the
    // continuation tracked since `configure_terminal` enabled it.
    match sess.term.encode_snapshot_alloc(None) {
        Ok(bytes) => Ok(bytes.map(|b| b.to_vec())),
        Err(e) => {
            log::warn!("terminal snapshot encode failed: {e}");
            Ok(None)
        }
    }
}

/// Build a Session around an ALREADY-OPEN master fd (e.g. received via
/// SCM_RIGHTS from another process). `handle.child_pid` must be the real
/// child pid (may be a foreign process; kill(-pid) still works, waitpid will
/// just return ECHILD which pty_wait maps to Some(0)). `snap` restores full
/// state when Some and decodable; otherwise a fresh Terminal at opts size.
/// Starts a new reader thread on the fd. Takes ownership of the fd.
pub fn adopt_session(
    handle: PtyHandle,
    snap: Option<&[u8]>,
    opts: &SessionOpts,
) -> Result<Session> {
    let master = handle.master_fd;
    let term = restore_terminal(snap, opts)?;
    match assemble(term, opts, handle) {
        Ok(sess) => Ok(sess),
        Err(e) => {
            // No reader thread owns the fd on this path (`assemble` spawns
            // it last; a spawn failure means there is no owner), so close it
            // ourselves. The child is deliberately NOT signalled: it may be
            // a foreign process the caller wants left running.
            // SAFETY: plain C close.
            unsafe { libc::close(master) };
            Err(e)
        }
    }
}

/// The terminal an adopted session starts from: the decoded snapshot when
/// one was supplied and decodes, else a fresh one at the requested size.
fn restore_terminal(snap: Option<&[u8]>, opts: &SessionOpts) -> Result<Terminal<'static, 'static>> {
    let bytes = match snap {
        Some(b) if !b.is_empty() => b,
        _ => return fresh_terminal(opts),
    };
    let decoded = libghostty_vt::snapshot::Decoder::new_buf(bytes).and_then(|dec| dec.decode());
    match decoded {
        Ok(mut term) => {
            // Decoded terminals come back with continuation tracking OFF
            // (the decoder's limit is an input check, not runtime policy)
            // and the snapshot's own scrollback ceiling: re-apply ours so
            // the adopted session behaves like a spawned one and stays
            // re-snapshot-able itself.
            configure_terminal(&mut term, opts)?;
            Ok(term)
        }
        Err(e) => {
            log::warn!("snapshot undecodable, adopting a fresh terminal: {e}");
            fresh_terminal(opts)
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

/// Plain A..Z ghostty keys (the ones that form C0 control bytes with Ctrl).
fn is_letter_key(k: libghostty_vt::key::Key) -> bool {
    use libghostty_vt::key::Key as K;
    matches!(
        k,
        K::A | K::B
            | K::C
            | K::D
            | K::E
            | K::F
            | K::G
            | K::H
            | K::I
            | K::J
            | K::K
            | K::L
            | K::M
            | K::N
            | K::O
            | K::P
            | K::Q
            | K::R
            | K::S
            | K::T
            | K::U
            | K::V
            | K::W
            | K::X
            | K::Y
            | K::Z
    )
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
    // Vendored-encoder workaround: with ANY kitty keyboard flags pushed
    // (crossterm apps like opencoder push them at startup), the pinned
    // encoder silently drops plain Ctrl+letter presses (its kitty path
    // bails when the event carries no text), killing ^C/^D/^Z delivery.
    // Route exactly those C0 combos through the legacy encoder instead:
    // clear the flag on the ENCODER OPTIONS only - the terminal's real
    // flag state (what the child pushed and can query or pop) is never
    // touched, and the per-key refresh above restores the child's flags
    // on the very next key.
    if sess.key_event.mods() == Mods::CTRL
        && is_letter_key(sess.key_event.key())
        && sess
            .term
            .kitty_keyboard_flags()
            .is_ok_and(|f| !f.is_empty())
    {
        sess.key_encoder
            .set_kitty_flags(libghostty_vt::key::KittyKeyFlags::empty());
    }
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
