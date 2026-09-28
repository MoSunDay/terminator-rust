#!/usr/bin/env python3
"""Deliver Codex's detached hook notification to its owning Linux terminal.

Codex captures hook stdout and detaches the controlling tty. Resolve its
nearest same-user ancestor tty; use inherited tmux only when its pane tty
matches that ancestor. Write only the canonical notice payload there. No
prompt text or hook input is forwarded.
"""
import os
from pathlib import Path
import select
import stat
import subprocess
import sys
import time


def run(args):
    return subprocess.run(args, capture_output=True, timeout=1, check=True)


def ancestor_terminal():
    pid = os.getppid()
    for _ in range(16):
        proc = Path("/proc") / str(pid)
        if pid <= 1 or proc.stat().st_uid != os.getuid():
            break
        for fd in (1, 2, 0):
            try:
                path = os.readlink(proc / "fd" / str(fd))
                if path.startswith("/dev/pts/"):
                    return path
            except OSError:
                pass
        # comm can contain spaces and parentheses; fields follow its final ')'.
        pid = int((proc / "stat").read_text().rsplit(")", 1)[1].split()[1])
    return None


def terminal():
    ancestor = ancestor_terminal()
    pane = os.environ.get("TMUX_PANE")
    if ancestor and os.environ.get("TMUX") and pane:
        try:
            path = run(["tmux", "display-message", "-p", "-t", pane, "#{pane_tty}"])
            if path.stdout.decode().strip() == ancestor:
                return ancestor, True
        except subprocess.SubprocessError:
            pass
    return ancestor, False


def write_notice(fd, payload, timeout=1):
    deadline = time.monotonic() + timeout
    while payload:
        try:
            written = os.write(fd, payload)
            if written:
                payload = payload[written:]
                continue
        except BlockingIOError:
            pass
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([], [fd], [], remaining)[1]:
            raise TimeoutError("terminal did not accept the complete notice")


def main():
    try:
        path, tmux = terminal()
        if path is None:
            return
        info = os.stat(path)
        if not path.startswith("/dev/pts/") or not stat.S_ISCHR(info.st_mode):
            return
        if info.st_uid != os.getuid():
            return
        result = subprocess.run(
            ["/usr/local/bin/terminator-ctl", "notice"], capture_output=True,
            start_new_session=True, timeout=1, check=True,
        )
        payload = result.stdout
        if payload != b"\x1b]9;terminator-rust notice\x07":
            return
        if tmux:
            payload = b"\x1bPtmux;" + payload.replace(b"\x1b", b"\x1b\x1b") + b"\x1b\\"
        fd = os.open(path, os.O_WRONLY | os.O_NOCTTY | os.O_NONBLOCK)
        try:
            write_notice(fd, payload)
        finally:
            os.close(fd)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print("terminator notice:", error, file=sys.stderr)


if __name__ == "__main__":
    main()
