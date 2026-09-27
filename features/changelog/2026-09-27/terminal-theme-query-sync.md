Commit: f93bcaf27b467032a76ce7bfb87f0cecc581701b

# Terminal theme query synchronization

Codex could not discover the foreground/background colors because the egui
palette was never installed in libghostty-vt. OSC 10/11 queries therefore
returned no answer, and Codex fell back to an unshaded composer.

Synchronize foreground, background (including user override), cursor and all
256 palette entries before pumping PTY output, including newly spawned panes.
Update defaults only, preserving application OSC overrides. Declare truecolor
for local PTYs and remote terminator-session shells. Render italic cells through
epaint text formatting while retaining the existing bold/faint rendering work.
Paint pane backgrounds, plain text and IME text from the VT frame's effective
colors, and answer CSI ? 996 n from the current effective background.

Regression coverage checks OSC 10/11/4 response colors, theme changes with OSC
overrides, OSC color reset, PTY COLORTERM defaults/explicit overrides, and
italic glyph geometry. A headless painter check covers effective foreground
and background colors.
No live application is restarted or installed by this change.
