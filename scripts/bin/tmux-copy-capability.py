#!/usr/bin/env python3
"""Announce the managed tmux copy key to one attached outer terminal."""
import os
import stat
import sys

if len(sys.argv) == 2 and sys.argv[1].startswith("/dev/pts/"):
    try:
        info = os.stat(sys.argv[1])
        if stat.S_ISCHR(info.st_mode) and info.st_uid == os.getuid():
            fd = os.open(sys.argv[1], os.O_WRONLY | os.O_NOCTTY | os.O_NONBLOCK)
            try:
                os.write(fd, b"\x1b]777;notify;terminator-rust;copy-transport-v1\x07")
            finally:
                os.close(fd)
    except OSError:
        pass
