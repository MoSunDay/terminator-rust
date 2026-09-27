Commit: dfac4c0272e337805f3954d3f431ad8473c052c9

# Terminal theme query synchronization

Codex could not discover the foreground/background colors because the egui
palette was never installed in libghostty-vt. OSC 10/11 queries therefore
returned no answer, and Codex fell back to an unshaded composer.

Synchronize foreground, background (including user override), cursor and all
256 palette entries before pumping PTY output, including newly spawned panes.
Update defaults only, preserving application OSC overrides. Declare truecolor
for local PTYs and remote terminator-session shells. Render italic cells through
epaint text formatting while retaining the existing bold/faint rendering work.

Regression coverage checks OSC 10/11/4 response colors, theme changes with OSC
overrides, PTY COLORTERM defaults/explicit overrides, and italic text shapes.
No live application is restarted or installed by this change.
