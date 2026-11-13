# farsub

A Sublime Text-style code editor in Rust (egui/eframe): Goto Anything, command palette,
symbol search, syntax highlighting with Sublime packages, LSP with a built-in server
installer, and a sidebar you can drive entirely from the keyboard, including file
operations. The mouse works too.

```bash
cargo run --release -- [folder-or-file]
```

"Cmd" below means Cmd on macOS and Ctrl elsewhere; both always work.

## Everything is data

Nothing about languages, servers, keys or menus is hard-coded. The defaults ship as TOML in
[runtime/](runtime/) (embedded into the binary at build time), and files in your config
folder override them:

```text
~/.config/farsub/                 ($FARSUB_CONFIG points at config.toml elsewhere)
  config.toml          [editor] settings (+ optional [[languages]] / [[servers]])
  languages/<name>.toml   add a language, or replace a built-in with the same name
  servers/<name>.toml     add/replace a language server (nvim-lspconfig style)
  keymap.toml          add/override/unbind keys (like Sublime's user keymap)
  menus.toml           replace the menu layout
  Packages/            Sublime packages: .sublime-syntax grammars, .tmTheme themes
~/.local/share/farsub/servers/<name>/   servers installed by the server manager ($FARSUB_DATA)
```

Saving any of these files inside farsub applies it immediately (Cmd+, opens `config.toml`,
created from a commented template).

## Menus and the keymap

On **macOS** the menus live in the system menu bar (built with `muda`), with shortcuts
shown and unavailable items greyed out. On **Windows and Linux** an in-window menu bar shows
the same menus (`show_menu_bar = true` turns it on for macOS too). Either way **F10** opens the
in-window menus from the keyboard: Left/Right switch menus, Up/Down move, Enter runs, a
letter jumps to an item, and Esc closes.

**Cmd+Shift+/** (Cmd+?) or Help → Keyboard Shortcuts lists every binding grouped by where it
applies, with a search box; Enter runs the selected command.

Custom keys in `keymap.toml` (`context` is `global`, `editor` or `sidebar`;
`command = ""` unbinds; `platforms` limits an entry):

```toml
[[keys]]
keys = "Cmd+K"
command = "hover"
context = "editor"

[[keys]]
keys = "F12"
command = ""          # unbind
context = "editor"
```

Command ids are the snake_case names shown in the keymap viewer (`goto_file`,
`toggle_comment`, `sidebar_new_file`, `manage_servers`, …).

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
it, and Cmd+Shift+E reveals the current file. Rows highlight on hover, and folders open and
close with a Sublime-style slide and rotating arrow (`animations = false` turns that off).

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

## Language servers (lspconfig + mason)

Languages list servers in order of preference; the first one that is installed (by
farsub, or found on PATH) runs. Built-in definitions:

| Language | Servers | Installable from farsub |
| --- | --- | --- |
| Python | pyright, basedpyright, pylsp | yes (npm / pip) |
| JavaScript, TypeScript, JSX, TSX | ts_ls (typescript-language-server) | yes (npm) |
| Bash / sh / zsh | bashls (bash-language-server) | yes (npm) |
| Go | gopls | yes (go install) |
| Rust | rust_analyzer | no (`rustup component add rust-analyzer`) |
| C / C++ | clangd | no (Xcode / LLVM) |

**Tools → Language Servers** (Cmd+Alt+L) is a Mason-like manager: it lists every server
with its status (installed, on PATH, not installed, failed), and `i` installs, `u` updates,
`x` uninstalls, Enter shows details and the install log. Installs go into farsub's own data
folder: npm packages under a private prefix, pip packages in a private venv, cargo/go
binaries in a private bin, so nothing touches your system. Opening a file whose server is
missing tells you which ones can be installed.

A server definition (`servers/<name>.toml`):

```toml
name = "lua_ls"
command = "lua-language-server"
args = []
root_markers = [".luarc.json", ".git"]
description = "Lua language server"

[settings.Lua]                    # answers workspace/configuration
diagnostics = { globals = ["vim"] }

[initialization_options]          # sent with initialize

[install]                         # one of: npm, pip, cargo, go, script
npm = ["some-package"]
bin = "lua-language-server"       # if it differs from `command`
```

A language (`languages/<name>.toml`):

```toml
name = "lua"
extensions = ["lua"]
filenames = []
first_line = "^#!.*\\blua"        # shebang detection
comment = "--"
tab_size = 2
servers = ["lua_ls"]
# syntax = "Lua"                  # force a grammar
```

Features: Ctrl+Space completion (also automatic), Cmd+I hover, F12 definition, Shift+F12
references, F2 rename, Alt+Shift+F format, F8 / Shift+F8 diagnostics. Edits sync
incrementally when the server supports it. `FARSUB_LSP_LOG=/tmp/lsp.log` traces traffic.

## Syntax packages (Sublime-style)

Drop Sublime Text packages into `~/.config/farsub/Packages/`: any `.sublime-syntax` file
adds or overrides a grammar, and any `.tmTheme` becomes a color theme (pick it with
Preferences → Color Theme or `theme = "..."`). They load in the background at startup.

- **Packages: Install Package (git URL)…** clones a package repository into `Packages/`.
- **Packages: Remove Package…** moves one to the Trash.
- **Packages: Browse Packages Folder** opens it in your file manager.
- **Packages: Reload Syntaxes & Themes** reloads after editing grammars by hand.

## Tests

`cargo test` runs unit tests (buffer, config, keymap, tree, file ops, installer, packages)
plus GUI end-to-end tests that drive the real interface headlessly with egui_kittest.
The LSP test uses clangd if it's installed. `cargo test -- --ignored` also runs real
npm and pip server installs (network). Set `FARSUB_SHOTS=dir` to save screenshots.
