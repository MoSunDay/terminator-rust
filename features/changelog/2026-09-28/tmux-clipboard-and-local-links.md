Commit: 10c1d1515e5963d9441e8df0f9c0ab3aeecd667c

# Copy from SSH/tmux TUIs and open links on the client

Mouse-aware TUIs own their text selection. Terminal-local selection still copies,
but previously Command+C without a local selection was discarded and incoming
OSC 52 writes had no clipboard callback. tmux load-buffer -w was confirmed to
emit OSC 52 with an empty selector under the deployed configuration.

- Queue bounded plain-text OSC 52 writes and drain them into egui's OS clipboard.
  Read requests remain unsupported; primary-selection writes do not replace the
  system clipboard. The queue holds at most one 100 KB text value.
- Forward Command+C / Ctrl+Shift+C when a mouse-aware TUI owns selection. Both
  folded copy events and key shortcut events use the same route. Local selection
  takes priority. Extended encoding carries the copy chord; legacy plain text or
  Ctrl+C fallback is rejected, so copying cannot interrupt the child process.
- Command-click (macOS) or Ctrl-click resolves OSC 8 targets and visible HTTP(S)
  URLs, including terminal soft wraps, and opens them through the local browser.
  A drag cancels opening. PTY output alone never opens a browser. Other URL schemes
  are not launched. A remote program reporting “Opened” does not confirm that a
  browser opened on the SSH client's computer.

The managed tmux wheel configuration on ssh_dev now enables set-clipboard on and
xterm-256color hyperlinks. Mouse/wheel routing stays enabled. The target server
codex-20260928-091931 was updated live; the shared configuration applies on future
launches/attaches. Existing application binaries need the updated client code;
PTY migration can preserve the original shell, SSH and remote Codex processes.

Checks: clipboard protocol tests, URL resolution tests, keyboard copy tests, and
scripts/bin/e2e-clipboard-links.sh (private Xvfb, tmux, real OS clipboard and a
browser recorder). Use Shift-drag then Command+C for terminal-local copying in
older running clients.

The deployed tmux copy route negotiates `copy-transport-v1` through an OSC 777
marker. Terminator sends the private key CSI 9001~; tmux User90 forwards raw
CSI 99;9u to the TUI. This avoids tmux normalizing ordinary extended copy
chords into Alt+C or Ctrl+C. Both client-attached and after-new-session hooks
announce the capability, including direct new-session launches.

Live rollout: 35 managed tmux servers loaded the shared configuration without
changing pane PIDs. All desktop tabs were moved to the new client. Final state
is 9 tabs / 28 unique live panes; two local connections were reattached to
existing remote sessions. Two duplicate migration offers caused by background
acknowledgement timeouts were removed without signalling the retained PTYs.
The real target session clipboard and full private-Xvfb end-to-end copy/link
suite passed. Migration timeout reconciliation remains a separate known issue.
