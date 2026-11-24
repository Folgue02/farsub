# farsub

A code editor in Rust (egui/eframe): Goto Anything, command palette,
symbol search, tree-sitter syntax highlighting, LSP with a built-in server
installer, and a sidebar you can drive entirely from the keyboard, including file
operations. The mouse works too.

```bash
cargo install --path .     # puts `farsub` on your PATH
farsub .                   # open the current folder (returns to the shell immediately)
```

How it's supposed to behave is written down in [docs/SPEC.md](docs/SPEC.md) (an index of one page per area, in `docs/spec/`).

Optional [WakaTime](https://wakatime.com) time tracking is built in, off by default: set
`wakatime = true`, then use "WakaTime: Set API Key…" (or reuse the key already in
`~/.wakatime.cfg`). See [docs/spec/wakatime.md](docs/spec/wakatime.md).

## Command line

Usage:

```text
farsub                      reopen the last project/folders (or the current folder)
farsub .                    open a folder (several folders → a multi-folder window)
farsub src/main.rs:42:5     open a file at line 42, column 5 (in the window used last,
                            else a new window with the last session)
farsub --new-window a.txt   open files in a new window instead
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

A **project** (`NAME.farsub-project`) says what's in the window; its
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
  folders, the find text and the find panel's options, recently closed files, and the contents of unsaved buffers
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
~/.config/farsub/  (Windows: %APPDATA%\farsub)   ($FARSUB_CONFIG points at config.toml elsewhere)
  config.toml          [editor] settings (+ optional [[languages]] / [[servers]])
  languages/<name>.toml   add a language, or replace a built-in with the same name
  servers/<name>.toml     add/replace a language server (nvim-lspconfig style)
  keymap.toml          add/override/unbind keys
  menus.toml           replace the menu layout
  themes/*.toml        color themes
  queries/<grammar>/highlights.scm   override a grammar's highlighting
  Packages/            packages: themes, highlight queries, icons
~/.local/share/farsub/  (Windows: %LOCALAPPDATA%\farsub)   ($FARSUB_DATA or --data-dir)
  servers/<name>/      servers installed by the server manager
  workspaces/          sessions for folders opened without a project file
  recent.toml          recent projects and the last session
```

Saving any of these files inside farsub applies it immediately (Cmd+, opens `config.toml`,
created from a short template the first time farsub starts).

Opening and saving use the system's file dialogs: **Cmd+O** opens a file, **File › Open
Folder…** a folder, and Save As / Open Project / Add Folder ask the same way. "File: Open
Path… (type it)" in the command palette keeps the typed path prompt with Tab completion.

## Menus and the keymap

On **macOS** the menus live in the system menu bar (built with `muda`), with shortcuts
shown and unavailable items greyed out. On **Windows and Linux** an in-window menu bar shows
the same menus (`show_menu_bar = true` turns it on for macOS too). Either way **F10** opens the
in-window menus from the keyboard: Left/Right switch menus, Up/Down move, Enter runs, a
letter jumps to an item, and Esc closes.

**Cmd+Shift+/** (Cmd+?) or Help → Keyboard Shortcuts lists every binding grouped by where it
applies, with a search box; Enter runs the selected command.

Custom keys in `keymap.toml` (`context` is `global`, `editor` or `sidebar`;
`command = ""` unbinds; `platforms` limits an entry). Preferences: Key Bindings
(File › Key Bindings…) opens it beside the defaults, which list every command id:

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

Type a prefix in any palette to switch modes.

## Sidebar (keyboard)

Cmd+0 focuses the sidebar (Cmd+0 again or Esc returns to the editor), Cmd+B shows or hides
it, and Cmd+Shift+E reveals the current file. Drag its right edge to resize it
(double-click the edge to reset); the width is saved with the workspace. Rows highlight on hover, and folders open and
close with a slide and rotating arrow (`animations = false` turns that off).

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

Editing keys: Cmd+D (select word, then next occurrence), Cmd+L, Cmd+Shift+K,
Cmd+Shift+D, Alt+↑/↓ (move line), Cmd+/ (comment), Cmd+] / Cmd+[, Cmd+Enter,
Cmd+Shift+Enter, Cmd+J, F9 (sort lines). Also Cmd+Z / Cmd+Shift+Z, Cmd+S,
Cmd+Shift+S, Cmd+Alt+S, Cmd+W, Cmd+Shift+T (reopen closed), Ctrl+Tab (the file used
before; keep Ctrl held and press Tab again to go further back), Ctrl+PageDown/PageUp and
Cmd+1…9 (switch tabs), Cmd+= / Cmd+- (zoom), Alt+- (jump back). Copying with nothing selected
takes the whole line. Brackets and quotes auto-close, and Enter auto-indents.

## Find, Replace and Find in Files

One panel at the bottom of the window:

| Key | Action |
|---|---|
| Cmd+F | Find (searches as you type; Enter next, Shift+Enter previous, Esc closes) |
| Cmd+Alt+F (Ctrl+H on Windows/Linux) | Replace: Tab to the Replace field; Enter replaces and finds the next |
| Ctrl+Alt+Enter | Replace All (in the file, or the selection with "in selection" on) |
| Cmd+Shift+F | Find in Files |
| F3 / Shift+F3 (also Cmd+G / Cmd+Shift+G on macOS) | Find next / previous |
| F4 / Shift+F4 | Next / previous Find in Files result |
| Cmd+Alt+R / C / W (Alt+R / C / W on Windows/Linux) | Toggle regex / case sensitive / whole word |

Matches are outlined while the panel is open, and it shows "3 of 12". With regex on,
the replacement can use `$1` or `\1`. Find in Files takes a **Where** list such as
`src, *.rs, -target/`; empty means the project folders. It skips `.gitignore`'d and
binary files, and searches unsaved changes as they are in the editor. Results go to a
"Find Results" tab: Enter or double-click on a line opens that match. Replacing in files
asks first, then leaves the changed files open and unsaved; File › Save All writes them.

## Language servers (lspconfig + mason)

Languages list servers in order of preference; the first one that is installed (by
farsub, or found on PATH) runs. Built-in definitions:

| Language | Servers | Installable from farsub |
| --- | --- | --- |
| Python | pyright, basedpyright, pylsp | yes (npm / pip) |
| JavaScript, TypeScript, JSX, TSX | ts_ls (typescript-language-server) | yes (npm) |
| Bash / sh / zsh | bashls (bash-language-server) | yes (npm) |
| Go | gopls | yes (go install) |
| Rust | rust_analyzer | yes (GitHub release) |
| C / C++ / CUDA | clangd | yes (GitHub release) |
| Lua | lua_ls | yes (GitHub release) |
| Zig | zls | yes (GitHub release) |
| Markdown | marksman | yes (GitHub release) |
| LaTeX, BibTeX | texlab | yes (GitHub release) |
| Typst | tinymist | yes (GitHub release) |
| Clojure | clojure_lsp | yes (GitHub release) |
| YAML | yamlls | yes (npm) |
| JSON, JSONC | jsonls | yes (npm) |
| HTML / CSS, SCSS, Less | html_ls / cssls | yes (npm) |
| TOML | taplo | yes (npm) |
| Dockerfile | dockerls | yes (npm) |
| PHP | intelephense | yes (npm) |
| Svelte | svelte | yes (npm) |
| Elm | elmls | yes (npm) |
| Vim script | vimls | yes (npm) |
| Prisma | prismals | yes (npm) |
| CMake | cmake_ls | yes (pip) |
| Java | jdtls | yes (eclipse.org; needs Java 21+) |
| Kotlin | kotlin_lsp (JetBrains), kotlin_language_server | yes (kotlin_lsp brings its own Java, ~370 MB; the other needs Java ≤ 21) |
| Scala | metals | yes (Coursier; needs Java) |
| Ruby | ruby_lsp, solargraph | yes (gem, into farsub's folder; needs Ruby) |
| C# | csharp_ls | yes (dotnet tool; needs the .NET SDK) |
| Gleam, Odin, Terraform | gleam, ols, terraformls | yes (prebuilt downloads) |
| Elixir, Nim | elixir_ls, nimlangserver | yes (need Elixir / Nim) |
| R | r_language_server | yes (R package; needs R) |
| Swift | sourcekit_lsp | no (ships with Xcode / Swift) |
| Haskell, OCaml | hls, ocamllsp | no (ghcup / opam: must match your compiler) |
| Dart, Erlang, Nix | dartls, erlang_ls, nil_ls / nixd | no (Dart SDK, your OTP version, Nix) |
| LakeScript | lks_lsp | no (`lks-lsp`, from the LakeScript toolchain) |

Languages with highlighting, comments and indentation defaults but no server yet: SQL,
GraphQL, Protocol Buffers, PowerShell, Batch, Perl, Julia, Fish, AWK, Groovy / Gradle /
Jenkinsfile, Crystal, F#, Vue, XML, INI / .editorconfig, .env, nginx, Solidity, GLSL,
WGSL, Starlark / Bazel, Make, Just, KDL and Meson.

Highlighting uses tree-sitter. 21 common grammars are compiled in; 37 more (C#,
Swift, Kotlin, Groovy / Gradle, Haskell, SQL, Zig, Dockerfile, Vue, …) **install
themselves** the first time you open such a file: farsub downloads the grammar's source and
compiles it in the background with your C compiler (Xcode Command Line Tools, gcc/clang,
or Visual Studio Build Tools). Turn this off with `auto_install = false`; then
**Tools → Install Grammar…** installs them. Language servers never install by themselves:
opening a file tells you which ones **Tools → Language Servers** can install.
Any other tree-sitter grammar installs from a source folder or archive on your computer:
**Grammars: Install Grammar from File…/Folder…** or `farsub --install-grammar PATH`.
`farsub --list-grammars` shows them all.
Until a file's tree-sitter grammar is installed (or when it can't be compiled), and for
the languages with no tree-sitter grammar yet (AWK, BibTeX, Clojure, Crystal, .env,
GraphQL, Just, LaTeX, Meson, Nim, Perl, Terraform, Typst and WGSL), farsub highlights it
with a bundled regex grammar (syntect, Sublime Text's `.sublime-syntax` format) instead,
so nothing shows as plain text.
Override the fallback with `fallback_syntax = "Grammar Name"` in a language file, and how
its scopes map to theme colors with a `scopes.toml` in the config folder.

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
title = "Lua"                    # display name
# grammar = "lua"                # tree-sitter grammar (defaults to the name)
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

## File icons and icon packages

The sidebar, tabs and Goto Anything show file-type icons. They're mapped in
`runtime/icons.toml`, and an `icons.toml` in your config folder adds or overrides rules:

```toml
[[icons]]
extensions = ["vue"]
label = "vue"          # 1-3 letters on a coloured badge
color = "#41B883"
```

An **icon package** is a folder in `Packages/` with its own `icons.toml` and PNG images:

```toml
folder_image = "img/folder.png"
folder_open_image = "img/folder-open.png"
[[icons]]
extensions = ["rs"]
image = "img/rust.png"   # relative to the package
```

All icon packages are used (in name order), or only one with `icon_theme = "Package
Name"`. `file_icons = false` hides icons. Icon packages made for other editors (such as A File Icon) use a
different format and don't work as they are.

Files matched by `.gitignore` are dimmed in the sidebar and left out of Goto Anything,
even in folders that aren't git repositories.

## Keys in input boxes

The find panel, prompts, palettes and the console line edit exactly like the editor: same
word movement, selection, delete-word, Select All, Undo and clipboard keys, because they
use the same `keymap.toml` commands (`move_word_left`, `select_to_line_end`,
`delete_word_left`, …). Rebind one and it changes everywhere.

## Split panes (editor groups)

Up to 4 columns, each with its own tabs.

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| Layout: Single / 2 / 3 / 4 columns | Cmd+Alt+1–4 | Alt+Shift+1–4 |
| Focus group N | Ctrl+1–4 | Ctrl+1–4 |
| Move file to group N | Ctrl+Shift+1–4 | Ctrl+Shift+1–4 |
| Select tab N in the group | Cmd+1–9 | Alt+1–9 |

Click a pane to focus it, and drag the divider to resize. Each pane has scrollbars
(vertical always, horizontal for long lines). The layout is saved with the
workspace.

Drag tabs to reorder them or move them to another pane. **Pin** a tab from its right-click
menu or the command palette (File: Pin Tab): it shrinks to its icon, stays at the start of
its pane's tabs, survives Close Other Tabs, and stays pinned after a restart.

**Preferences: Settings** (Cmd+,) opens a settings window: the full,
read-only `Default Settings.toml` on the left and your settings file on the right. Copy
over what you want to change and save; every farsub window picks up the change. Closing
either pane closes the window. The settings file and its folder are created the first time
farsub starts.

**Preferences › Font…** lists the fonts installed on your computer (monospaced first) and
previews each one in the editor as you move through them; Enter keeps it (written to your
settings as `font_face`), Esc goes back. **Preferences › Color Theme…** works the same way.

Pressing Enter on a line that holds only indentation leaves that line empty, so blank lines
don't keep trailing spaces (`trim_automatic_white_space = false` turns that off).

## Terminal

**Ctrl+`** toggles the integrated terminal (like VS Code), **Ctrl+Shift+`** opens another
one. Terminals live in the bottom panel next to the console, start your `$SHELL` in the
project folder, and get every key including Ctrl+C (Cmd shortcuts stay app shortcuts on
macOS). Cmd+C / Ctrl+Shift+C copy the selection, Cmd+V / Ctrl+Shift+V paste; drag to select,
scroll for history (also Shift+Page Up/Down). Ctrl+Tab / Ctrl+Shift+Tab cycle terminal tabs
while a terminal is focused. `exit` closes the tab; a non-zero exit keeps it open showing the
code until you press a key. Close any tab with its ×, a middle-click, or Terminal: Kill
Terminal. `terminal_shell = "/bin/bash"` picks a different shell.

**Ctrl+Alt+`** (Terminal: New Terminal in Editor Area) opens a terminal as an editor tab
instead, next to your files. It has a darker background, a green `>_` icon and the shell
and folder as its title. It drags between panes and switches with Ctrl+Tab like any tab,
and asks before closing while its shell is still running. Terminal: Move Terminal to
Panel / into Editor Area moves a terminal between the two places without restarting it.
Terminal tabs aren't restored after a restart.

## Context menus

Right-click the editor, sidebar, tabs, terminal or console for a context menu (Shift+F10 or
the Menu key opens it from the keyboard). The items are data: edit `[context.editor]`,
`[context.sidebar]`, `[context.tab]`, `[context.terminal]` or `[context.console]` in
`menus.toml`.

## Console

"View: Show Console" in the command palette (or View → Show Console) shows the app log: status
messages, language-server output (`window/logMessage`, start/exit with exit codes,
initialize failures), server install output, package loading errors, and panics. The
same log is written to `<data>/logs/farsub.log` (the previous launch's is kept as `farsub.previous.log`). The input line takes:

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

If an open file is deleted on disk, its tab and the window title say "(deleted)"; the
contents stay (as unsaved changes) until you close the tab, which warns you first, or save
to recreate the file. If another program changes an open file, farsub reloads it (undo
gets your version back), or asks first when you have unsaved edits there; saving over an
outside change asks too. Files keep their encoding: a Latin-1, Windows-1252, Shift_JIS or
UTF-16 file saves back exactly as it was (the status bar shows the encoding; **File:
Convert to UTF-8** switches it), and saving never leaves a half-written file behind.
**File → New Window** (Cmd+Shift+N, or `farsub --empty`) opens a
clean window with no folder or session.

Long lines wrap by default (`word_wrap = true`), with wrapped rows indented like the line.
View › Word Wrap (Alt+Z) toggles it for the current file.

`ui_scale = 1.25` zooms the whole interface (fonts, sidebar, tabs, menus, panels) like
browser zoom. Cmd+= / Cmd+- still change only the editor font size.

Goto Symbol (Cmd+R) lists names only (`add`, `Point`, `impl Debug for Point<T>`) with the
kind on the right. No signatures.

Caret settings: `caret_style = "smooth"` (fade, the default), `"blink"` or
`"solid"`, and `caret_extra_width` / `caret_extra_top` / `caret_extra_bottom` in pixels
(defaults 1.5 / 1 / 1).

## Syntax per file

Click the language name at the right of the status bar (or View → Syntax…) to pick the
current file's language from a fuzzy list. Highlighting, comments, indentation and the
language server follow it, so choosing "Bash" for an extension-less script also gives it
`#` comments and bash-language-server. The choice is remembered in the workspace.

## Themes, highlighting and packages

Highlighting is tree-sitter based, and everything visual is data in farsub's own formats:

- **Themes** are TOML (`runtime/themes/*.toml`; add your own in `~/.config/farsub/themes/`):
  an `[editor]` table of colors and a `[syntax]` table keyed by highlight capture, with
  fallback from specific to general (`function.builtin` → `function`):

  ```toml
  name = "My Theme"
  [editor]
  background = "#1E1E1E"
  foreground = "#D4D4D4"
  selection = "#264F78"
  [syntax]
  keyword = "#C586C0"
  string = "#CE9178"
  comment = { fg = "#6A9955", italic = true }
  ```

- **Highlight queries** (`runtime/queries/<grammar>/highlights.scm`) decide what gets which
  capture; a file at `~/.config/farsub/queries/<grammar>/highlights.scm` replaces one.
- **Grammar recipes** (`grammars/<name>.toml`) say where an installable grammar's source is:

  ```toml
  name = "fsharp"
  crate = "tree-sitter-fsharp"   # crates.io source (or: source = "/path/to/grammar")
  version = "0.3.12"
  path = "fsharp"                # folder holding src/parser.c, if not the root
  ```

- **Packages** are folders in `~/.config/farsub/Packages/` that can hold `themes/`,
  `queries/`, `grammars/` (recipes) and an `icons.toml`.
  - **Packages: Install Package (git URL)…** clones one into `Packages/`.
  - **Packages: Install Package from File… / from Folder…** picks a `.zip` / `.tar.gz`
    archive or a folder on your computer (also `farsub --install-package PATH`).
  - **Packages: Remove Package…** moves one to the Trash.
  - **Packages: Browse Packages Folder** opens the folder.
  - **Packages: Reload Syntaxes & Themes** applies changes.

## Tests

`cargo test` runs unit tests (buffer, config, keymap, tree, file ops, installer, packages,
projects, workspaces, CLI planning), CLI integration tests against the built binary, and
GUI end-to-end tests that drive the real interface headlessly with egui_kittest.
The LSP test uses clangd if it's installed. `cargo test -- --ignored` also runs real
npm and pip server installs (network). Set `FARSUB_SHOTS=dir` to save screenshots.

## Versions and releases

Versions follow [semver](https://semver.org) and each release is a git tag `vX.Y.Z`
(the version also lives in `Cargo.toml`, and `farsub --version` prints it). While the
version is 0.x, a minor bump may still change behaviour.

1. Tag the release, from a clean tree:

   ```bash
   scripts/bump-version.sh minor     # or patch / major / 1.4.0; --tag tags the current version
   git push --follow-tags
   ```

2. Pushing the tag runs the **Release** workflow (`.github/workflows/release.yml`). It
   builds every platform with `scripts/release.sh` on GitHub's macOS, Windows and Linux
   machines and publishes a pre-release with:
   - `farsub-<version>-macos-universal.zip`: `farsub.app` (Apple Silicon and Intel).
   - `farsub-<version>-windows-x86_64.zip`: `farsub.exe`.
   - `farsub-<version>-linux-x86_64.tar.gz`: `farsub` (built on Ubuntu 22.04, so it runs
     on that and newer).
   - `SHA256SUMS.txt`.

   Its notes are GitHub's list of changes since the last tag; edit them on the release
   page or with `gh release edit vX.Y.Z --notes-file NOTES.md`. Running the workflow by
   hand (Actions › Release › Run workflow) only builds, and keeps the archives as the
   run's artifacts.

To build the archives yourself instead, `scripts/release.sh` puts them in
`dist/<version>/` with a `SHA256SUMS.txt`. macOS needs a Mac (a universal app when rustup
has both `aarch64-apple-darwin` and `x86_64-apple-darwin`, otherwise this Mac's
architecture) and Linux needs Linux. Windows builds on Windows with `scripts\release.ps1`
(needs rustup and Visual Studio Build Tools), or cross-compiles from a Mac (one-time
setup below).

One-time setup to build macOS and Windows from a Mac (rustup stays out of your PATH; the
script finds it, llvm and lld through Homebrew):

```bash
brew install rustup llvm lld
$(brew --prefix rustup)/bin/rustup toolchain install stable --profile minimal \
    --target aarch64-apple-darwin --target x86_64-apple-darwin --target x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
```

The first Windows build downloads Microsoft's Windows SDK and C runtime (accepting
their license). If Homebrew's own `rust` stops working after llvm is upgraded
(`Symbol not found … libLLVM`), `brew upgrade rust` fixes it. Then create the release on
GitHub from the tag and upload the archives and `SHA256SUMS.txt`.

The macOS app is ad-hoc signed, so on another Mac the first launch needs right-click →
Open (or `xattr -dr com.apple.quarantine farsub.app`). Set `MACOS_SIGN_IDENTITY` to sign it
with a Developer ID certificate instead; notarizing isn't automated. To use the `farsub`
command from a terminal, link it:
`ln -s /Applications/farsub.app/Contents/MacOS/farsub /usr/local/bin/farsub`.
