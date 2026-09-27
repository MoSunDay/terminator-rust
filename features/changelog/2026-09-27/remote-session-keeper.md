Commit: 5c8147f6068dcb1bfb21990043a3d5990b32dd20

# Remote session keeper

New remote sessions use the repository's `terminator-session` binary
when it is installed on the target account's SSH `PATH`. The binary keeps a
shell PTY in a detached process and attaches clients through a private Unix
socket named by the saved remote session name. Several clients can attach
without disconnecting one another. A disconnected client leaves the shell
running; the final 4 MiB of output is replayed on the next attach.

`terminator-session list` shows the session name, attached or detached state,
creation directory, and purpose title from the remote target label. The
release and remote deployment packages now contain the binary. Plain SSH is
used when the keeper is absent; that fallback does not preserve a shell
across disconnections. The app does not stop existing external sessions.

The isolated reconnect test covers retained shell state, simultaneous
clients, disconnects, listing, and shell exit. A fake executable test confirms
the bootstrap does not call Zellij even when that executable is present.
Network interruption of a live SSH connection was not rerun for this change.
