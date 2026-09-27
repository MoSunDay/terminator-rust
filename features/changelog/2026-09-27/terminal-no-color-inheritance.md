Commit: 84a080f4409c54bf941a1dc92d162736876af468

# Terminal child color environment

## Context

When the GUI was started with `NO_COLOR=1`, local pane children inherited
that setting. Codex then suppressed its composer shading and accent colors
even though the terminal supported truecolor and answered color queries.

## Change

Local and remote session shells no longer inherit the launcher's `NO_COLOR`.
An explicit local pane environment override remains available. The terminal
continues to advertise `COLORTERM=truecolor`.

## Verification

The PTY regression checks the default and explicit override. A graphical
X11 probe launched the GUI with `NO_COLOR=1`, then confirmed that Codex's
composer background and accent colors appeared. A separate live probe
captured alternating visible and hidden frames for blinking terminal text.
Workspace build, tests, formatting and Clippy passed.
