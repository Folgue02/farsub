# farsub

A terminal file manager + editor: Far Manager's twin blue panels, Sublime Text's editing
keys and Goto Anything palette, syntect syntax highlighting, and LSP support.

```bash
cargo run --release -- [dir-or-file]
```

`Ctrl` and `Cmd` are interchangeable. Cmd only reaches the app in terminals that speak the
kitty keyboard protocol (kitty, WezTerm, Ghostty, recent iTerm2 with "Report keys using CSI u"),
and only if the terminal doesn't bind that Cmd shortcut itself. Without that protocol,
`Ctrl+Shift+<letter>` can't be told apart from `Ctrl+<letter>`, so use `F1` for the command palette.

## Panels (Far)

| Key | Action |
| --- | --- |
| Tab | switch panel |
| Enter | open dir / file, or run the typed command line in the panel's dir |
| Backspace | parent dir (when command line is empty) |
| Insert, Shift+↑/↓, Ctrl+Space | mark files |
| F3 / F4 / Shift+F4 | view / edit / new file |
| F5 / F6 / F7 / F8 | copy / rename-move / mkdir / delete (to the other panel's dir by default) |
| Ctrl+H / Ctrl+R / Ctrl+U | toggle hidden / refresh / swap panels |
| Ctrl+O | toggle panels ↔ editor |
| F10, Ctrl+Q | quit |

Anything you type goes to the command line (`cd` is handled internally; other commands run in `$SHELL`).

## Palettes (Sublime Goto Anything)

| Key | Palette | Prefix |
| --- | --- | --- |
| Ctrl/Cmd+P | fuzzy file finder (respects .gitignore); `file.rs:42` jumps to a line | – |
| Ctrl/Cmd+Shift+P, F1 | command palette (every action and its key) | `>` |
| Ctrl/Cmd+R | symbols in file (LSP, keyword-scan fallback), live preview | `@` |
| Ctrl/Cmd+Shift+R | symbols in project (LSP `workspace/symbol`) | `#` |
| Ctrl+G | goto line | `:` |
| Ctrl/Cmd+E | switch open file | |

You can type a prefix in any palette to switch modes, as in Sublime.

The bottom bar always shows the keys for whatever has focus (panels, editor, sidebar,
palette, prompt, completion list). The editor's tab bar shows a clock on the right.

## Sidebar

Ctrl/Cmd+B shows the project's file tree and focuses it, revealing the current file.
Press it again while the tree has focus to hide it; Esc or Tab returns to the editor.
In the tree: ↑/↓ move, → expand, ← collapse or jump to parent, Enter open, Space preview
(open but stay in the tree), letters jump to the next matching name, Ctrl+H hidden files.

## Editor

Ctrl+S save · Ctrl+Shift+S save as · Ctrl+W/F10 close · Ctrl+Z / Ctrl+Shift+Z undo/redo ·
Ctrl+C/X/V copy/cut/paste (whole line when nothing is selected; system clipboard) ·
Ctrl+D select word / next occurrence · Ctrl+L expand to line · Ctrl+A select all ·
Ctrl+Shift+K delete line · Ctrl+Shift+D duplicate · Alt+↑/↓ move line · Ctrl+/ toggle comment ·
Tab/Shift+Tab indent/outdent · Ctrl+Enter / Ctrl+Shift+Enter insert line after/before ·
Ctrl+J join lines · F9 sort lines · Ctrl+F incremental find, F3 / Shift+F3 next/prev ·
Ctrl+H replace all · Alt/Ctrl+←/→ word motion · Cmd+←/→ line start/end ·
Ctrl+PgUp/PgDn switch file. Brackets and quotes auto-close, Enter auto-indents.

## LSP

Servers start on demand per (language, project root) when you open a file:
rust-analyzer, pyright/pylsp, typescript-language-server, gopls, clangd, lua-language-server,
zls, sourcekit-lsp, ruby-lsp, jdtls, taplo, bash-language-server, nil, and others (see `src/lsp.rs`).

Ctrl+Space completion (also triggers automatically) · Ctrl+K hover · F12 goto definition ·
Shift+F12 references · F2 rename · Alt+Shift+F format · F8 / Shift+F8 next/prev diagnostic ·
Ctrl+- jump back. Diagnostics show as gutter dots, colored underlines, and counts in the status bar.

Set `FARSUB_LSP_LOG=/tmp/lsp.log` to trace all LSP traffic.

Edits are sent to the server incrementally (only the changed ranges) when it supports that.

## Performance notes

- File-finder filtering narrows the previous match set as you type and scores large lists
  on all cores (about 11 ms per keystroke over 200k paths, down from about 46 ms).
- Highlighting works in 12 ms slices per frame, so jumping deep into a huge file never
  freezes input; the rest fills in over the next few frames.
- Dependencies are optimized even in debug builds (`[profile.dev.package."*"]`).
- `cargo test --release bench_ -- --ignored --nocapture` prints the timings.

## Tests

`cargo test` runs the buffer unit tests and end-to-end tests that drive the app with key events
and render into a headless terminal. The LSP test uses clangd if it's installed.
