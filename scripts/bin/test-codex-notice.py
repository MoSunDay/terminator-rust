#!/usr/bin/env python3
"""Regression checks for Codex's detached terminal notice writer."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "codex_notice", Path(__file__).with_name("codex-notice.py")
)
notice = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notice)


class WriteNoticeTests(unittest.TestCase):
    def test_partial_writes_finish_the_whole_sequence(self):
        chunks = []

        def write(fd, data):
            chunks.append(data[:2])
            return min(2, len(data))

        with patch.object(notice.os, "write", side_effect=write):
            notice.write_notice(3, b"abcdef")
        self.assertEqual(b"".join(chunks), b"abcdef")

    def test_busy_terminal_retries_when_writable(self):
        with patch.object(
            notice.os, "write", side_effect=[BlockingIOError(), 3]
        ) as write, patch.object(
            notice.select, "select", return_value=([], [3], [])
        ) as wait:
            notice.write_notice(3, b"abc")
        self.assertEqual(write.call_count, 2)
        wait.assert_called_once()

    def test_busy_terminal_reports_timeout(self):
        with patch.object(
            notice.os, "write", side_effect=BlockingIOError()
        ), patch.object(
            notice.select, "select", return_value=([], [], [])
        ):
            with self.assertRaisesRegex(TimeoutError, "complete notice"):
                notice.write_notice(3, b"abc")


class TerminalTests(unittest.TestCase):
    def test_stale_tmux_pane_uses_own_ancestor_terminal(self):
        with patch.dict(notice.os.environ, {"TMUX": "old", "TMUX_PANE": "%1"}), patch.object(
            notice, "ancestor_terminal", return_value="/dev/pts/42"
        ), patch.object(
            notice, "run", return_value=SimpleNamespace(stdout=b"/dev/pts/60\n")
        ):
            self.assertEqual(notice.terminal(), ("/dev/pts/42", False))

    def test_current_tmux_pane_uses_passthrough(self):
        with patch.dict(notice.os.environ, {"TMUX": "live", "TMUX_PANE": "%2"}), patch.object(
            notice, "ancestor_terminal", return_value="/dev/pts/42"
        ), patch.object(
            notice, "run", return_value=SimpleNamespace(stdout=b"/dev/pts/42\n")
        ):
            self.assertEqual(notice.terminal(), ("/dev/pts/42", True))


if __name__ == "__main__":
    unittest.main()
