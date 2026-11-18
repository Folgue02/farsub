# Bottom panel

One panel, tabs in its header: one tab per terminal ("TERMINAL n: <title>", the title is
the shell name or what the program sets), then CONSOLE. Every terminal tab has its own ×
(shown on the active/hovered tab); middle-click also closes it. Header buttons: `+` new
terminal, `x` kills the current terminal (or hides the panel on the console tab).

## Terminal (VS Code style)

- **Ctrl+`** toggles the panel showing the terminal (creating one on first use) and focuses
  it; pressing it while the terminal has focus hides the panel. **Ctrl+Shift+`** opens a
  new terminal. Terminals start in the project's first folder with the user's `$SHELL`
  (`cmd.exe`/PowerShell on Windows), or `terminal_shell` if set; `TERM=xterm-256color`.
- **Ctrl+Tab / Ctrl+Shift+Tab** cycle terminal tabs (wrapping) while a terminal has
  focus; in the editor they keep switching files.
- Keys go to the shell, including Ctrl combinations (Ctrl+C interrupts). Cmd shortcuts on
  macOS stay app shortcuts and never reach the shell. A short list of app shortcuts always
  works in the terminal on every platform: toggle/new terminal, show console, command
  palette, Goto Anything, focus sidebar. Shift+Page Up/Down scroll the history.
- Copy/paste: Cmd+C copies the selection (on Windows/Linux Ctrl+Shift+C; plain Ctrl+C is
  sent to the shell when nothing is selected); Cmd+V / Ctrl+Shift+V pastes (bracketed paste
  when the program asks for it).
- Mouse: drag to select text; wheel scrolls back through history.
- When the shell exits with code 0 (e.g. `exit`) its tab closes by itself (VS Code); the
  panel hides when it was the last one. A non-zero exit keeps the tab showing
  "[process exited with code N — press any key to close]".
- Killing a tab (×, middle-click, Terminal: Kill Terminal, `x`) kills its shell.
  Terminals are not restored across restarts.

## Console (app log)

- Opened from the command palette: **"View: Show Console"** (no default shortcut), or the
  View menu. It is a tab of the bottom panel.
- Shows the app log: status messages, language-server log messages/start/exit/initialize
  failures, server install output, package errors, panics. Also written to
  `<data>/logs/farsub.log`; the previous launch's log is kept as `farsub.previous.log`
  (so a crash's panic message survives reopening farsub).
- Text is **selectable**: drag to select, double-click selects a word, Cmd+A selects all,
  Cmd+C copies.
- Long lines **wrap** by default; `wrap` in the console input toggles wrapping, and with
  wrapping off the panel scrolls sideways (Shift+wheel / horizontal trackpad scroll).
- Input line: `help`, `clear`, `debug` (show raw server stderr), `filter TEXT`, `wrap`,
  `log`, `lsp`, or any command id. Up/Down recall history. Esc clears the selection, or
  hides the panel when nothing is selected.
