//! Session wiring: pty reader thread, terminal state, key encoding.

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::thread;

use anyhow::{bail, Context, Result};
use libghostty_vt::key::{Action, Encoder as KeyEncoder, Event as KeyEvent, Mods};
use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::terminal::Mode;
use libghostty_vt::Terminal;

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
        }
    }
}

/// One terminal pane: pty + terminal + render helpers.
///
/// The reader thread owns nothing but a pty fd clone; all state lives here
/// and is only touched from the UI thread.
pub struct Session {
    handle: PtyHandle,
    events: Receiver<PtyEvent>,
    pub term: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_it: RowIterator<'static>,
    cell_it: CellIterator<'static>,
    key_encoder: KeyEncoder<'static>,
    key_event: KeyEvent<'static>,
    /// Non-empty once the child has exited / the master closed.
    pub exit: Option<i32>,
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

    let write_fd = handle.master_fd;
    // The closure only captures a Copy fd, so 'static holds.
    term.on_pty_write(move |_t, data| {
        let _ = pty::pty_write(write_fd, data);
    })?;

    let (tx, rx) = channel::<PtyEvent>();
    let read_fd = handle.master_fd;
    let pid = handle.child_pid;
    thread::Builder::new()
        .name("pty-reader".to_string())
        .spawn(move || reader_loop(read_fd, pid, tx))
        .context("spawning reader thread")?;

    Ok(Session {
        handle,
        events: rx,
        term,
        render_state: RenderState::new()?,
        row_it: RowIterator::new()?,
        cell_it: CellIterator::new()?,
        key_encoder: KeyEncoder::new()?,
        key_event: KeyEvent::new()?,
        exit: None,
    })
}

/// Blocking reader loop for the pty master; runs on its own thread.
fn reader_loop(fd: i32, pid: i32, tx: std::sync::mpsc::Sender<PtyEvent>) {
    let mut buf = [0u8; 8192];
    loop {
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
    // SAFETY: plain C close.
    unsafe { libc::close(fd) };
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
    pty::pty_resize(sess.handle.master_fd, cols, rows)
}

/// Write raw bytes to the pty.
pub fn write(sess: &Session, data: &[u8]) -> Result<()> {
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
