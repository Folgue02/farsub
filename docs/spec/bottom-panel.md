# Bottom panel

One panel, tabs in its header: one tab per terminal ("TERMINAL n: <title>", the title is
the shell name or what the program sets), then CONSOLE. Every terminal tab has its own ×
(shown on the active/hovered tab); middle-click also closes it. Header buttons: `+` new
terminal, `x` kills the current terminal (or hides the panel on the console tab).

## Terminal (VS Code style)

- **Ctrl+`** toggles the panel showing the terminal (creating one on first use) and focuses
  it; pressing it while the terminal has focus hides the panel. **Ctrl+Shift+`** opens a
  new terminal. Terminals start in the project's first folder with `terminal_shell` if
  set (a program name or path), else the user's `$SHELL`; on Windows the default is
  PowerShell (decided 2026-11-19: PowerShell 7's `pwsh` when installed, else the
  built-in `powershell.exe`; `terminal_shell = "cmd.exe"` brings cmd back).
  `TERM=xterm-256color`. Device status requests (`ESC[6n` cursor position, `ESC[5n`)
  are answered: Windows' pseudo-console asks for the cursor position when it starts and
  shows nothing until it gets an answer, which is why the shell never appeared there.
- **Ctrl+Tab / Ctrl+Shift+Tab** cycle the panel's terminal tabs (wrapping) while a panel
  terminal has focus; in the editor (terminal tabs included) they keep switching files.
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

## Terminals in the editor area (decided 2026-11-24)

- **Terminal: New Terminal in Editor Area** (**Ctrl+Alt+`**, View menu) opens a terminal
  as a tab of the focused editor group instead of in the panel. It starts like a panel
  terminal (project folder, `terminal_shell`). How its tab looks and behaves as a tab
  (dragging, pinning, Ctrl+Tab, what closing asks) is in [Window layout](window.md).
- It's the same terminal: keys, copy/paste, selection, scrolling, the right-click menu,
  and the exit rules (exit 0 closes the tab; a non-zero exit keeps it showing the code
  until a key is pressed) all work as in the panel.
- **Moving between the panel and a tab**: Terminal: Move Terminal into Editor Area
  turns the panel's current terminal into a tab of the focused group; Terminal: Move
  Terminal to Panel sends a terminal tab back to the panel. The shell keeps running and
  keeps its screen and history either way. Both are in the command palette and the
  terminal's right-click menu; Move Terminal to Panel is in the tab's right-click menu
  too.
- The panel lists only its own terminals ("TERMINAL n" counts those); Ctrl+` toggles
  the panel and never picks a terminal that's in a tab.
- Terminal: Kill Terminal kills the focused terminal, whether it's in the panel or a
  tab, without asking (killing is what it was asked to do). Closing a running
  terminal's tab with its × or Cmd+W asks first.

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
