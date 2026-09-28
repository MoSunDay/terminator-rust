//! Receive terminal clipboard writes (OSC 52) without reading the host clipboard.
use libghostty_vt::terminal::{ClipboardLocation, ClipboardWriteError};
use libghostty_vt::Terminal;
use std::sync::{Arc, Mutex};

/// Last pending plain-text write; bounded and drained by the UI thread.
pub type PendingCopy = Arc<Mutex<Option<String>>>;
const MAX_BYTES: usize = 100_000;

pub fn install(term: &mut Terminal<'static, 'static>) -> anyhow::Result<PendingCopy> {
    let pending = Arc::new(Mutex::new(None));
    let target = Arc::clone(&pending);
    term.on_clipboard_write(move |_, write| {
        if write.location() != ClipboardLocation::Standard {
            return Err(ClipboardWriteError::Unsupported);
        }
        let text = write
            .contents()
            .find(|c| c.mime == "text/plain")
            .ok_or(ClipboardWriteError::Unsupported)?
            .data;
        if text.len() > MAX_BYTES {
            return Err(ClipboardWriteError::InvalidData);
        }
        if !text.is_empty() {
            *target.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
        }
        Ok(())
    })?;
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tmux_empty_selector_and_direct_copy_are_received() {
        let mut term = Terminal::new(20, 4).unwrap();
        let pending = install(&mut term).unwrap();
        // Actual load-buffer -w output from the deployed tmux configuration.
        term.vt_write(b"\x1b]52;;dGVybWluYXRvci1jbGlwYm9hcmQtcHJvYmU=\x07");
        assert_eq!(
            pending.lock().unwrap().take().as_deref(),
            Some("terminator-clipboard-probe")
        );
        // Chunked SSH reads and the alternate ST terminator.
        term.vt_write(b"\x1b]52;c;aGVs");
        assert!(pending.lock().unwrap().is_none());
        term.vt_write(b"bG8=\x1b\\");
        assert_eq!(pending.lock().unwrap().take().as_deref(), Some("hello"));
    }
    #[test]
    fn query_and_primary_selection_do_not_overwrite_clipboard() {
        let mut term = Terminal::new(20, 4).unwrap();
        let pending = install(&mut term).unwrap();
        term.vt_write(b"\x1b]52;c;?\x07\x1b]52;p;aGVsbG8=\x07");
        assert!(pending.lock().unwrap().is_none());
    }
}
