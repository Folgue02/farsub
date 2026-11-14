# farsub

A Sublime Text-style code editor in Rust (egui/eframe): Goto Anything, command palette,
symbol search, syntax highlighting with Sublime packages, LSP with a built-in server
installer, and a sidebar you can drive entirely from the keyboard, including file
operations. The mouse works too.

```bash
cargo install --path .     # puts `farsub` on your PATH
farsub .                   # open the current folder (returns to the shell immediately)
```

How it's supposed to behave is written down in [docs/SPEC.md](docs/SPEC.md).

## Command line

Modeled on Sublime's `subl`:

```text
farsub                      reopen the last project/folders (or the current folder)
farsub .                    open a folder (several folders → a multi-folder window)
farsub src/main.rs:42:5     open a file at line 42, column 5 (in the last session)
farsub app.farsub-project   open a project (also: -p/--project FILE)
farsub -a ../lib            add a folder to the last session's project
farsub -n file.txt          don't restore any session
farsub -w file.txt          wait until the window closes (git config core.editor "farsub -w")
git log | farsub -          read stdin into a new buffer
```

Headless commands (no window): `--list-servers`, `--install-server NAME`,
`--uninstall-server NAME`, `--check-config` (validates config, keymap, menus, languages,
and `--project FILE`; exits non-zero on errors). `--config FILE` and `--data-dir DIR` point
at other config/data folders. `farsub --help` lists everything.

## Projects and workspaces

Like Sublime: a **project** (`NAME.farsub-project`) says what's in the window; its
**workspace** (`NAME.farsub-workspace`, next to it) remembers the session.

```toml
# App.farsub-project
[[folders]]
path = "."                                  # relative to the project file
name = "App"
folder_exclude_patterns = ["target", "node_modules"]
file_exclude_patterns = ["*.min.js"]

[[folders]]
path = "../shared-lib"

[settings]                                  # overrides [editor] settings
tab_size = 2

[[languages]]                               # project-only languages/servers (optional)
name = "lua"
extensions = ["lua"]
```

- Every project folder is a top-level node in the sidebar; exclude patterns hide entries
  there and in Goto Anything (which searches all folders, prefixed with the folder name).
- The workspace stores open files, cursors, selections, scroll positions, expanded
  folders, the find text, recently closed files, and the contents of unsaved buffers
  (**hot exit**: quitting never asks; changes come back next time). Turn that off with
  `hot_exit = false`. Folders opened without a project file get a workspace too (kept in
  the data folder), and the session is also autosaved every 30 seconds.
- **Project** menu: Open Project… (a project file or folder), **Switch Project…**
  (Cmd+Alt+P, recent projects and folders), Save Project As…, Close Project, Add Folder
  to Project…, Remove Folder from Project (the selected root in the sidebar), Edit
  Project (opens the file; saving applies it in place). Adding/removing folders updates
  the project file.
- Starting without arguments reopens the last project or folder set
  (`restore_session = false` to disable).

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
~/.local/share/farsub/                ($FARSUB_DATA or --data-dir)
  servers/<name>/      servers installed by the server manager
  workspaces/          sessions for folders opened without a project file
  recent.toml          recent projects and the last session
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
it, and Cmd+Shift+E reveals the current file. Drag its right edge to resize it
(double-click the edge to reset); the width is saved with the workspace. Rows highlight on hover, and folders open and
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
| Rust | rust_analyzer | yes (prebuilt binary from GitHub releases) |
| C / C++ | clangd | yes (prebuilt binary from GitHub releases) |

**Tools → Language Servers** (Cmd+Alt+L) is a Mason-like manager: it lists every server
with its status (installed, on PATH, not installed, failed), and `i` installs, `u` updates,
`x` uninstalls, Enter shows details and the install log. Installs go into farsub's own data
folder: npm packages under a private prefix, pip packages in a private venv, cargo/go
binaries in a private bin, GitHub release binaries downloaded and unpacked by farsub itself
(no `curl`/`tar`/`sh` needed, so it works the same on Windows), so nothing touches your system. Opening a file whose server is
missing tells you which ones can be installed.

A server definition (`servers/<name>.toml`). `${install_dir}` (the server's install
folder; settings using it are dropped when the server comes from PATH), `${root}` and
`${home}` are replaced in `args`, `env`, `settings` and `initialization_options`:

```toml
name = "lua_ls"
command = "lua-language-server"
args = []
root_markers = [".luarc.json", ".git"]
description = "Lua language server"

[settings.Lua]                    # answers workspace/configuration
diagnostics = { globals = ["vim"] }

[initialization_options]          # sent with initialize

[install]                         # one of: npm, pip, cargo, go, github, script
npm = ["some-package"]
bin = "lua-language-server"       # if it differs from `command`
```

Prebuilt binaries from a GitHub release work on every platform. Asset globs are keyed
`"<os>-<arch>"` or just `"<os>"` (macos/linux/windows, x86_64/aarch64). `.zip`, `.tar.gz`
and `.gz` are unpacked, and the executable is found by name inside:

```toml
[install.github]
repo = "LuaLS/lua-language-server"
# tag = "3.15.0"                  # latest release when omitted
[install.github.assets]
macos-aarch64 = "lua-language-server-*-darwin-arm64.tar.gz"
macos-x86_64 = "lua-language-server-*-darwin-x64.tar.gz"
linux-x86_64 = "lua-language-server-*-linux-x64.tar.gz"
windows = "lua-language-server-*-win32-x64.zip"
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

**LSP Info** (Cmd+Alt+I, Tools → LSP Info, or click the server name / "no LSP" in the
status bar) is like Neovim's `:LspInfo`. It shows the current file's language and syntax,
every configured server in preference order with its status (attached, running,
installed, on PATH, not installed), and the attached server's command, root, pid, uptime,
sync mode, capabilities, diagnostics and recent stderr output, plus all running servers.
`i` installs the first missing server for this file, `r` restarts servers, `s` opens the
server manager.

Completion opens on its own (or with Ctrl+Space) with nothing selected: Ctrl+N / Ctrl+P
pick an item, Enter/Tab accept it; otherwise Enter and Tab work as usual. Items show a kind
icon (function, method, variable, field, class, …).

Features: hover (rest the mouse on a symbol for half a second, or Cmd+I at the cursor; diagnostics under
the pointer show on top), F12 definition, Shift+F12
references, F2 rename, Alt+Shift+F format, F8 / Shift+F8 diagnostics. Edits sync
incrementally when the server supports it. `FARSUB_LSP_LOG=/tmp/lsp.log` traces traffic.

## Terminal

**Ctrl+`** toggles the integrated terminal (like VS Code), **Ctrl+Shift+`** opens another
one. Terminals live in the bottom panel next to the console, start your `$SHELL` in the
project folder, and get every key including Ctrl+C (Cmd shortcuts stay app shortcuts on
macOS). Cmd+C / Ctrl+Shift+C copy the selection, Cmd+V / Ctrl+Shift+V paste; drag to select,
scroll for history (also Shift+Page Up/Down). Ctrl+Tab / Ctrl+Shift+Tab cycle terminal tabs
while a terminal is focused. `exit` closes the tab; a non-zero exit keeps it open showing the
code until you press a key. Close any tab with its ×, a middle-click, or Terminal: Kill
Terminal. `terminal_shell = "/bin/bash"` picks a different shell.

## Context menus

Right-click the editor, sidebar, tabs, terminal or console for a context menu (Shift+F10 or
the Menu key opens it from the keyboard). The items are data: edit `[context.editor]`,
`[context.sidebar]`, `[context.tab]`, `[context.terminal]` or `[context.console]` in
`menus.toml`.

## Console

"View: Show Console" in the command palette (or View → Show Console) shows the app log: status
messages, language-server output (`window/logMessage`, start/exit with exit codes,
initialize failures), server install output, package loading errors, and panics. The
same log is written to `<data>/logs/farsub.log`. The input line takes:

- `help`, `clear`, `debug` (also show servers' raw stderr), `filter TEXT`, `log` (open the
  log file), `lsp` (print LSP Info)
- any command id, e.g. `restart_lsp`, `toggle_sidebar`, `goto_file` (Up/Down for history)

Text is selectable (drag, double-click for a word, Cmd+A, Cmd+C). Long lines wrap; `wrap`
turns wrapping off so the panel scrolls sideways (Shift+wheel or a horizontal trackpad
swipe). Drag the panel's top edge to resize it; Esc hides it.

## Indentation

Each file's indentation is detected when it opens (tabs vs spaces, and the space width
from how indentation changes between lines). Click "Spaces: N" / "Tab Size: N" in the
status bar (or View → Indentation…) for Indent Using Spaces / Tabs, Tab Width 1–8,
Guess Settings From Buffer, and Convert Indentation to Spaces / Tabs (one undo step).
Choices are remembered per file in the workspace. Indentation guides are drawn at each
level, with the block around the cursor highlighted (`indent_guides = false` hides them). The current-line highlight is off by default;
`highlight_current_line = true` turns it on.

`ui_scale = 1.25` zooms the whole interface (fonts, sidebar, tabs, menus, panels) like
browser zoom. Cmd+= / Cmd+- still change only the editor font size, as in Sublime.

Goto Symbol (Cmd+R) lists names only (`add`, `Point`, `impl Debug for Point<T>`) with the
kind on the right, as in Sublime. No signatures.

The caret uses Sublime's settings: `caret_style = "smooth"` (fade, the default), `"blink"` or
`"solid"`, and `caret_extra_width` / `caret_extra_top` / `caret_extra_bottom` in pixels
(defaults 1.5 / 1 / 1).

## Syntax per file

Like Sublime, click the syntax name at the right of the status bar (or View → Syntax…) to
pick a syntax for the current file from a fuzzy list. The file's language follows the
syntax, so choosing "Bourne Again Shell" for an extension-less script also gives it `#`
comments and bash-language-server. The choice is remembered in the workspace.

## Syntax packages (Sublime-style)

Drop Sublime Text packages into `~/.config/farsub/Packages/`: any `.sublime-syntax` file
adds or overrides a grammar, and any `.tmTheme` becomes a color theme (pick it with
Preferences → Color Theme or `theme = "..."`). They load in the background at startup.

- **Packages: Install Package (git URL)…** clones a package repository into `Packages/`.
- **Packages: Remove Package…** moves one to the Trash.
- **Packages: Browse Packages Folder** opens it in your file manager.
- **Packages: Reload Syntaxes & Themes** reloads after editing grammars by hand.

## Tests

`cargo test` runs unit tests (buffer, config, keymap, tree, file ops, installer, packages,
projects, workspaces, CLI planning), CLI integration tests against the built binary, and
GUI end-to-end tests that drive the real interface headlessly with egui_kittest.
The LSP test uses clangd if it's installed. `cargo test -- --ignored` also runs real
npm and pip server installs (network). Set `FARSUB_SHOTS=dir` to save screenshots.
