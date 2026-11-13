# farsub

A Sublime Text-style code editor in Rust (egui/eframe): Goto Anything, command palette,
symbol search, syntax highlighting, LSP, and a sidebar you can drive entirely from the
keyboard, including file operations. The mouse works too.

```bash
cargo run --release -- [folder-or-file]
```

"Cmd" below means Cmd on macOS and Ctrl elsewhere; both always work.

## Palettes

| Key | Palette | Prefix |
| --- | --- | --- |
| Cmd+P | Goto Anything: fuzzy file finder (respects .gitignore); `main.rs:42` opens at a line | – |
| Cmd+Shift+P, F1 | Command palette (every action and its shortcut) | `>` |
| Cmd+R | Symbols in the file (LSP, keyword fallback), with live preview | `@` |
| Cmd+Shift+R | Symbols in the project (LSP) | `#` |
| Ctrl+G | Goto line | `:` |
| Cmd+E | Switch open file | |

Type a prefix in any palette to switch modes, as in Sublime.

## Sidebar (keyboard)

Cmd+0 focuses the sidebar (Cmd+0 again or Esc returns to the editor), Cmd+B shows or hides
it, and Cmd+Shift+E reveals the current file.

| Key | Action |
| --- | --- |
| ↑ ↓ / j k, PgUp/PgDn, Home/End, g / G | move |
| → / l, ← / h | expand, collapse / go to parent |
| Enter / o | open file (Alt+Enter keeps focus in the sidebar) or toggle folder |
| Space, Shift+↑↓ | mark items for multi-item operations |
| a | new file in the selected folder (`sub/dir/file.rs` creates folders) |
| Shift+A | new folder |
| r, F2 | rename / move (edit the full path relative to the project) |
| c / Cmd+C, x / Cmd+X | copy, cut |
| p / Cmd+V | paste into the selected folder (name clashes become `name copy.ext`) |
| Shift+D | duplicate |
| d, Delete | move to the Trash (asks first; recoverable) |
| y | copy the absolute path |
| . | toggle hidden files |
| F5 / R | refresh |
| Esc | clear marks, then clear cut/copy, then back to the editor |

Path prompts support Tab completion. Renaming or moving files that are open updates their
tabs and language-server state.

## Editor

Sublime defaults: Cmd+D (select word, then next occurrence), Cmd+L, Cmd+Shift+K,
Cmd+Shift+D, Alt+↑/↓ (move line), Cmd+/ (comment), Cmd+] / Cmd+[, Cmd+Enter,
Cmd+Shift+Enter, Cmd+J, F9 (sort lines). Also Cmd+F (find panel: Enter next,
Shift+Enter previous), Cmd+Alt+F (replace all), Cmd+Z / Cmd+Shift+Z, Cmd+S,
Cmd+Shift+S, Cmd+Alt+S, Cmd+W, Cmd+Shift+T (reopen closed), Ctrl+Tab and Cmd+1…9
(switch tabs), Cmd+= / Cmd+- (zoom), Alt+- (jump back). Copying with nothing selected
takes the whole line. Brackets and quotes auto-close, and Enter auto-indents.

## LSP

Built-in language configs: **Rust** (rust-analyzer), **Python** (pyright), **TypeScript /
JavaScript** incl. TSX/JSX (typescript-language-server), **Go** (gopls), **C / C++**
(clangd). Servers start on demand per project root.

Ctrl+Space completion (also automatic), Cmd+I hover, F12 definition, Shift+F12 references,
F2 rename, Alt+Shift+F format, F8 / Shift+F8 next/previous diagnostic. Diagnostics show as
gutter dots, squiggles and status-bar counts. Edits are synced incrementally when the
server supports it. Set `FARSUB_LSP_LOG=/tmp/lsp.log` to trace all traffic.

## Settings and adding languages

Cmd+, opens `~/.config/farsub/config.toml` (created from a commented template). Saving it
applies the changes immediately. `$FARSUB_CONFIG` overrides the location.

```toml
[editor]
font_size = 14.0
tab_size = 4
theme = "Monokai Extended"   # or pick one live: "Preferences: Select Color Theme"

# A new language
[[languages]]
name = "lua"
extensions = ["lua"]
comment = "--"
tab_size = 2
roots = [".luarc.json"]
[languages.server]
command = "lua-language-server"
[languages.server.settings.Lua]          # answers workspace/configuration
diagnostics = { globals = ["vim"] }

# Replacing a built-in one (same name)
[[languages]]
name = "python"
extensions = ["py", "pyi"]
comment = "#"
server = { command = "pylsp" }
```

Per-language keys: `language_id`, `extensions`, `filenames`, `comment`, `tab_size`,
`use_tabs`, `roots`, `syntax` (highlighting override), and `server` (`command`, `args`,
`env`, `initialization_options`, `settings`).

## Tests

`cargo test` runs buffer, config, tree and file-op unit tests, plus GUI end-to-end tests
that drive the real interface headlessly with egui_kittest. The LSP test uses clangd if
it's installed. Set `FARSUB_SHOTS=dir` to save rendered screenshots.
