//! UDS client half of the control protocol: resolve the socket, write one
//! newline-terminated JSON request, read exactly one line back.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use ipc_proto::{Request, Response, ENV_SOCKET};

/// Response line cap. An 80-row capture is ~16 KiB, so this only trips on a
/// misbehaving server; it bounds memory before we parse.
const MAX_LINE: usize = 4 * 1024 * 1024;

/// Socket path in priority order: `$TERMINATOR_SOCK` wins even before the
/// file exists, then the first existing default location.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = env_nonempty(ENV_SOCKET) {
        return Some(PathBuf::from(p));
    }
    default_paths().into_iter().find(|p| p.exists())
}

pub fn request(req: &Request, timeout: Duration) -> Result<Response> {
    request_via(socket_path(), req, timeout)
}

/// [`request`] with a `--socket <path>` override: a non-empty flag wins
/// over `$TERMINATOR_SOCK` and the default locations and talks straight to
/// that path; `None`/empty falls back to the default resolution.
pub fn request_flagged(flag: Option<&str>, req: &Request, timeout: Duration) -> Result<Response> {
    match flag.filter(|f| !f.is_empty()) {
        Some(f) => request_at(Path::new(f), req, timeout),
        None => request(req, timeout),
    }
}

/// [`request`] against an explicit socket path (`--socket`, `migrate`
/// targets, `instances` probes): same wire behavior, caller-chosen endpoint.
pub fn request_at(path: &Path, req: &Request, timeout: Duration) -> Result<Response> {
    let mut stream = UnixStream::connect(path).with_context(|| {
        format!(
            "cannot connect to terminator-rust (app running?) at {}",
            path.display()
        )
    })?;
    // Timeouts are best-effort: a socket that refuses them still works.
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let mut line = serde_json::to_string(req).context("encode request")?;
    line.push('\n');
    stream.write_all(line.as_bytes()).context("write request")?;
    let mut buf = String::new();
    // Bound the read itself: a hostile/buggy server must not be able to
    // buffer unbounded bytes into memory before the cap is checked.
    let n = BufReader::new(stream)
        .take(MAX_LINE.saturating_add(1) as u64)
        .read_line(&mut buf)
        .context("read response")?;
    if n == 0 {
        anyhow::bail!("app closed the connection");
    }
    if buf.len() > MAX_LINE {
        anyhow::bail!(
            "oversized response line ({} bytes > {})",
            buf.len(),
            MAX_LINE
        );
    }
    decode_response(&buf)
}

/// Shared tail of [`request`]/[`request_flagged`]: a resolved-but-missing
/// path becomes the familiar "not found" error.
fn request_via(path: Option<PathBuf>, req: &Request, timeout: Duration) -> Result<Response> {
    let path = path.ok_or_else(|| {
        anyhow!(
            "terminator-rust control socket not found (is the app running? expected ${} or {})",
            ENV_SOCKET,
            hint_path().display()
        )
    })?;
    request_at(&path, req, timeout)
}

/// Sibling instance sockets (`ipc*.sock`) in the runtime dir, sorted, no
/// liveness probing (see [`probe`]). This is the `instances` roster and the
/// `--to` target vocabulary for `migrate`.
pub fn discover() -> Vec<PathBuf> {
    ipc_proto::migrate::discover_sockets(&paths::runtime_dir(), None)
}

/// Liveness probe budget per discovered socket: short enough that a dir of
/// dead sockets does not make `instances` feel slow.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Probe one discovered socket: a live terminator-rust instance answers
/// `List`, which comes back as `Some`; every failure (missing file, refused
/// connect, dead peer, malformed reply) folds into `None`.
pub fn probe(path: &Path, timeout: Duration) -> Option<Response> {
    match request_at(path, &Request::List, timeout) {
        Ok(resp @ Response::List { .. }) => Some(resp),
        _ => None,
    }
}

/// One response line -> `Ok(Response)`; app-reported errors become `Err` so
/// `main` prints them like any other failure.
fn decode_response(line: &str) -> Result<Response> {
    let resp: Response = serde_json::from_str(line.trim_end())
        .with_context(|| format!("malformed response: {}", line.trim_end()))?;
    match resp {
        Response::Error { message } => Err(anyhow!(message)),
        resp => Ok(resp),
    }
}

fn default_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = env_nonempty("XDG_RUNTIME_DIR") {
        v.push(PathBuf::from(dir).join("terminator-rust").join("ipc.sock"));
    }
    // Keep in sync with the server's fallback (app/src/ipc/server.rs).
    v.push(paths::config_dir().join("ipc.sock"));
    v
}

/// Where we would have expected the socket, for the "not found" message.
fn hint_path() -> PathBuf {
    env_nonempty(ENV_SOCKET)
        .map(PathBuf::from)
        .or_else(|| default_paths().into_iter().next())
        .unwrap_or_else(|| PathBuf::from("$XDG_RUNTIME_DIR/terminator-rust/ipc.sock"))
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc_proto::CaptureOut;

    #[test]
    fn response_line_roundtrip() {
        let cap = CaptureOut {
            text: "hello\nworld".into(),
            cols: 80,
            rows: 24,
            cursor_x: 1,
            cursor_y: 2,
            cursor_visible: true,
            exit: None,
        };
        let mut line = serde_json::to_string(&Response::Capture(cap.clone())).unwrap();
        line.push('\n');
        assert_eq!(decode_response(&line).unwrap(), Response::Capture(cap));
    }

    #[test]
    fn error_line_becomes_err() {
        let line = serde_json::to_string(&Response::err("no such pane")).unwrap();
        let err = decode_response(&line).unwrap_err().to_string();
        assert_eq!(err, "no such pane");
        assert!(decode_response("not json").is_err());
    }

    #[test]
    fn request_at_and_probe_on_dead_paths() {
        // No server needed: connect failure is the whole behavior under test.
        let dead = Path::new("/nonexistent/terminator-ctl-probe.sock");
        let err = request_at(dead, &Request::List, Duration::from_millis(50)).unwrap_err();
        assert!(err.to_string().contains("cannot connect"), "{err}");
        assert!(probe(dead, Duration::from_millis(50)).is_none());
    }
}
