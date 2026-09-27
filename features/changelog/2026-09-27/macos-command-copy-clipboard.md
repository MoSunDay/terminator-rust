Commit: dfac4c0272e337805f3954d3f431ad8473c052c9

# macOS Command+C copies terminal selection

Terminal copy now sends the selected text through egui's window clipboard output. The same path handles Command+C and Ctrl+Shift+C, while bare Ctrl+C still reaches the terminal as an interrupt. Empty selections leave the clipboard intact.

The keyboard regression test checks that Command+C exports selected text and does not send `^C` to the PTY.
