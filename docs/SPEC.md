# farsub — product specification

The living description of how farsub works and should behave. When a decision changes,
this document is corrected (not appended to). "Cmd" means Cmd on macOS and Ctrl on
Windows/Linux; "Ctrl" means the Control key on every platform.

## 1. Principles

- **Sublime Text feel.** Layout, keys, palettes and behaviors follow Sublime Text unless a
  decision below says otherwise. Where VS Code has the better-known convention (integrated
  terminal), we follow VS Code.
- **Keyboard first, mouse welcome.** Every action is reachable from the keyboard (shortcut,
  command palette, or F10 menus). Mouse interactions mirror what Sublime/VS Code do.
- **Nothing hard-coded.** Languages, servers, key bindings and menus ship as data in
  `runtime/` and can be overridden from the user's config folder. Code only implements
  commands and mechanisms.
- **Never lose work.** Hot exit keeps unsaved changes across restarts; destructive file
  operations go to the system Trash and ask first.

## 2. Command line

- `farsub [PATHS…]` opens files (`FILE[:LINE[:COL]]`), folders and `.farsub-project` files;
  `-` reads stdin into a new untitled buffer.
- **Does not block the shell**: the command hands off to a background process and returns
  immediately (like `subl`). `-w/--wait` keeps it in the foreground until the window
  closes, for use as `$EDITOR` / `git config core.editor "farsub -w"`.
- Argument errors are reported in the shell (exit code 2) before handing off.
- What opens: a project file → that project; folders → those folders (or added to the last
  session with `-a`); files only → the last session (or the file's project root if there
  is none); nothing → the last session, else the current folder. `-n` skips the session.
- Headless commands run in the foreground and never open a window: `--list-servers`,
  `--install-server`, `--uninstall-server`, `--check-config`. `--config` and `--data-dir`
  redirect config/data folders.

## 3. Window layout

```
┌ menu bar (in-window on Windows/Linux; system menu bar on macOS) ───────────────┐
│ sidebar │ tabs                                                                  │
│ FOLDERS │ editor                                                                │
│         ├───────────────────────────────────────────────────────────────────────┤
│         │ bottom panel: TERMINAL n │ CONSOLE  (optional, resizable)              │
│         ├───────────────────────────────────────────────────────────────────────┤
│         │ input panel (find, prompts)                                           │
├ status bar ──────────────────────────────────────────────────────────────────────┤
```

- `ui_scale` (default 1.0, range 0.5–3) zooms the **whole interface**: every font, widget,
  margin and hit area, like browser zoom. It's separate from `font_size` and from View →
  Zoom In/Out (Cmd+= / Cmd+-), which size only the editor text, as in Sublime. Changing it
  in the settings file applies immediately. The macOS menu bar is drawn by the OS and isn't
  scaled.
- **Editor groups** (Sublime's View › Layout): 1–4 columns side by side, each with its own
  tab strip; every file is in exactly one group. Layout: Single / 2 / 3 / 4 Columns
  (macOS Cmd+Alt+1–4; Windows/Linux Alt+Shift+1–4). Focus Group N: Ctrl+1–4. Move File
  to Group N: Ctrl+Shift+1–4 (adds columns if needed, and focus follows the file). Focus
  Next Group has no default key (command palette).
- Clicking in a group focuses it. The focused group's active tab has the accent bar; the
  others are dimmed and show no caret. Dragging a divider resizes the columns
  (double-click evens them out).
- Opening a file puts it in the focused group; a file already open elsewhere is focused
  in its own group. Next/Previous File, Close Other Tabs / Tabs to the Right, and tab
  numbers (Cmd+1–9 on macOS; Alt+1–9 on Windows/Linux, as in Sublime) work within the
  focused group.
- A group whose last tab closes stays focused and empty; editing commands do nothing
  there. Removing columns moves their tabs into the last remaining group.
- The layout, each file's group and the focused group are saved in the workspace.
- Rows and grid layouts aren't supported yet (columns only).
- The sidebar is resizable by dragging its right edge (double-click resets); its width is
  saved in the workspace. Cmd+B toggles it, Cmd+0 focuses it.
- The bottom panel is resizable by dragging its top edge; its height and visibility are
  saved in the workspace.
- Status bar, right side (all clickable): LSP status → LSP Info; indentation → Indentation
  menu; syntax name → syntax picker.

## 4. Editor

- **Settings** (Preferences: Settings, Cmd+,): as in Sublime, opens two panes. On the
  left, `Default Settings.toml`: read-only, every setting with its default and an
  explanation, regenerated from the running version. On the right, your settings file,
  focused. A new user file starts nearly empty (Sublime-style); you copy over only what you
  change. Saving applies it. Closing either pane closes both (the other only if it has no
  unsaved changes) and restores the previous layout. **Difference from Sublime:** Sublime
  opens these in a new window; farsub is single-window, so they open as a two-column
  layout in the current window.

- Mouse pointer: I-beam over text, normal arrow over selected text (Sublime), arrow over
  the gutter.
- Indentation: detected per file on open (tabs vs spaces; space width from the most common
  indentation change between lines; a file indented with N spaces gets tab width N). The
  Indentation menu offers spaces/tabs, tab width 1–8, guess from buffer, convert to
  spaces/tabs (one undo step). Manual choices are remembered per file in the workspace.
- Indentation guides at every level; the block around the cursor is highlighted.
  `indent_guides = false` disables them.
- Caret: Sublime's setting names. Width = 1 + `caret_extra_width`; it reaches
  `caret_extra_top` px above and `caret_extra_bottom` px below the line. Defaults 1.5 / 1 /
  1: a little thicker than a hairline and a little taller than the line.
- Caret animation `caret_style`: `"smooth"` (default, as in Sublime: fades out and back in
  once per second), `"blink"` (hard on/off, 0.5 s each) or `"solid"`. It stays solid for
  0.5 s after every move or edit, and only animates while the window has focus (no
  background repaints).
- Goto Symbol (Cmd+R) lists symbol **names only**, as in Sublime: `add`, `Point`,
  `impl Debug for Point<T>`, Markdown headings without `#`. The kind (fn, struct, class…)
  is shown dimmed on the right; signatures never appear. Nested symbols are indented.
  Results come from the language server when one is attached, otherwise from a built-in
  scan of declaration keywords.
- **Word wrap** (`word_wrap`, default **on**): long lines wrap at the pane width, breaking
  after spaces when possible (mid-word otherwise). Continuation rows are indented like
  the line (Sublime's `indent_subsequent_lines`); the line number and diagnostic dot show
  on the first row only; indent guides run through all rows. Up/Down move by visual
  rows, keeping the visual column; Home/End and Page Up/Down still work on whole lines.
  Clicking past the end of a wrapped row puts the caret at that row's end. No horizontal
  scrolling while wrapping. View › Word Wrap / "View: Toggle Word Wrap" (Alt+Z) toggles
  it per file, remembered in the workspace. **Differs from Sublime**, whose default
  `"auto"` wraps only prose (Markdown, plain text); here the default is on everywhere.
- Current-line highlight is **off by default** (`highlight_current_line = true` turns it
  on).
- Mouse hover (LSP): resting the pointer on a symbol for 500 ms shows the server's hover
  info; diagnostics under the pointer are listed on top in their severity color. Works
  without a server for diagnostics alone. The popup closes when the pointer leaves both the
  word and the popup, on typing, or on Esc. Cmd+I shows the same at the cursor.
- Right-click moves the cursor to the click (unless it lands inside the selection, which is
  kept) and opens the editor context menu.
- Syntax: detected from extension/first line/packages; the status-bar syntax picker
  overrides it per file, and the file's language (comment token, indentation defaults, LSP
  server) follows the chosen syntax.
- Bundled grammars: two-face's 220 (bat's set: Lua, Ruby, Java, Kotlin, Swift, Zig, Nix,
  Terraform, Svelte, Vue, TOML, YAML, Dockerfile, …) plus farsub's own in
  `runtime/syntaxes/` (Gleam, Just, KDL, Prisma, Meson). build.rs precompiles them into
  one dump, so startup cost doesn't grow with the number of grammars. `Packages/` grammars
  are added on top at runtime.
- Built-in languages (`runtime/languages/`, ~75): each gives file extensions / file names
  / shebang detection, line-comment token, indentation defaults, grammar, and servers.
  Invariants (enforced by tests): no two built-in languages claim the same extension or
  file name, every claimed file gets a real grammar (never Plain Text), and every listed
  server exists. Languages whose only comments are block comments (HTML, CSS, XML,
  Markdown, OCaml, Svelte, Vue) have no comment token yet, so Toggle Comment does nothing
  there. Block-comment support is still to do.

## 4a. Scrollbars

- Every editor pane has a vertical scrollbar on the right: thin, wider on hover (Sublime
  style). Drag the thumb, or click the track to jump there. Errors/warnings are marked
  along it in their colors. The text area leaves room for it.
- A horizontal scrollbar appears under the text when a line is wider than the pane (only
  with word wrap off). The vertical scrollbar works in whole lines.
- **Smooth scrolling:** the mouse wheel and trackpad scroll the editor and the sidebar by
  pixels (rows can be partly visible at the top), and the scrollbar thumb moves
  continuously. Moving the cursor, Goto and Page Up/Down still snap to whole lines.
- Scrollbars only scroll; the cursor doesn't move. The mouse wheel scrolls anywhere over
  the pane, including over the scrollbars.

## 4b. File icons

- File-type icons in the sidebar (folders get an open/closed folder), editor tabs, Goto
  Anything and Switch Open File. `file_icons = false` turns them off.
- Built-in icons are drawn (no font or image files): a coloured badge with a 1–3 letter
  label per file type, and a plain page for unknown types. The mapping is data:
  `runtime/icons.toml` (`[[icons]]` with `names` / `extensions`, `label`, `color`;
  `[default]`, `folder_color`). Exact names win over extensions, the longest extension
  wins (`.d.ts` beats `.ts`), and later rules win ties.
- **Icon packages:** any folder in `Packages/` with an `icons.toml` (e.g. installed with
  "Packages: Install Package (git URL)") is an icon package. Its rules may use `image =
  "path.png"` (relative to the package) instead of a label, plus `folder_image` /
  `folder_open_image`. All packages load in name order after the built-ins, then the
  user's own `icons.toml` on top. `icon_theme = "<package>"` uses only that package. PNG
  only (no SVG). Packages: Reload applies changes.
- Sublime icon packages (e.g. A File Icon) are **not** compatible: they map icons through
  Sublime syntax/theme preference files, a different format.

## 5. Completion (LSP)

- Opens automatically after 3 identifier characters or a server trigger character, or with
  Ctrl+Space.
- **Nothing is selected when the list opens.** Ctrl+N / Ctrl+P select the next/previous
  item. On Windows/Linux these keys only navigate while the list is open (otherwise they are
  Cmd+N / Cmd+P = New File / Goto Anything). Mouse click selects and accepts.
- Enter / Tab accept the selected item. With nothing selected they behave normally
  (newline / indent) and close the list. Esc closes it. Arrow keys and Page Up/Down move
  the editor cursor and close the list. Typing more characters refilters the list and
  clears the selection again.
- Each item shows a kind icon: a small colored badge with a glyph per kind (function,
  method, variable, field/property, class/struct, interface, module, keyword, snippet,
  constant, enum, file…), plus the label and detail.

## 5a. Goto Anything (Cmd+P)

- Lists the project's files, **excluding** gitignored files (in or outside a git
  repository), hidden files and the project's exclude patterns.
- Each file is a two-line row, as in Sublime: its icon, the file name larger and bold,
  and the path below it in a smaller dim font. Fuzzy-match characters are highlighted in
  both lines. Switch Open File uses the same rows.

## 6. Bottom panel

One panel, tabs in its header: one tab per terminal ("TERMINAL n: <title>", the title is
the shell name or what the program sets), then CONSOLE. Every terminal tab has its own ×
(shown on the active/hovered tab); middle-click also closes it. Header buttons: `+` new
terminal, `x` kills the current terminal (or hides the panel on the console tab).

### 6.1 Terminal (VS Code style)

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

### 6.2 Console (app log)

- Opened from the command palette: **"View: Show Console"** (no default shortcut), or the
  View menu. It is a tab of the bottom panel.
- Shows the app log: status messages, language-server log messages/start/exit/initialize
  failures, server install output, package errors, panics. Also written to
  `<data>/logs/farsub.log`.
- Text is **selectable**: drag to select, double-click selects a word, Cmd+A selects all,
  Cmd+C copies.
- Long lines **wrap** by default; `wrap` in the console input toggles wrapping, and with
  wrapping off the panel scrolls sideways (Shift+wheel / horizontal trackpad scroll).
- Input line: `help`, `clear`, `debug` (show raw server stderr), `filter TEXT`, `wrap`,
  `log`, `lsp`, or any command id. Up/Down recall history. Esc clears the selection, or
  hides the panel when nothing is selected.

## 7. Context menus

- Right-click opens a context menu in the editor, sidebar (selects the row first), editor
  tabs, terminal and console. **Shift+F10** or the **Menu** key opens the same menu for
  the focused area at the cursor/selected row.
- Items are data: `[context.editor|sidebar|tab|terminal|console] items = [...]` in
  `menus.toml` (command ids, `"-"` separators); user files override per section.
- Keyboard: arrows move, Enter runs, a letter runs the item starting with it, Esc or a
  click outside closes. Disabled items are dimmed and skipped.
- egui's own widget focus is never used: every key goes through the app's routing, so Tab
  never moves focus to (and Enter never "clicks") a menu-bar button.

## 8. Sidebar

- Files and folders matched by `.gitignore` (and `.git/info/exclude`) are listed but
  **dimmed**, as in Sublime 4 / VS Code; everything inside an ignored folder counts as
  ignored. This works whether or not the folder is a git repository.

- Every project folder is a top-level node. Keyboard: arrows/hjkl move and fold, Enter
  opens, Space marks, `a` new file, `A` new folder, `r` rename/move, `c`/`x`/`p`
  copy/cut/paste, `D` duplicate, `y` copy path, `d`/Delete trash (asks), `.` hidden files.
- Hover highlight; folders open/close with a slide + rotating arrow animation.

## 9. Projects and workspaces

- `NAME.farsub-project`: folders (path, name, exclude patterns), `[settings]` overriding
  editor settings, optional `[[languages]]`/`[[servers]]`.
- `NAME.farsub-workspace` next to it: open files, cursors, scroll, syntax/indentation
  overrides, expanded folders, sidebar width, panel state, find text, closed files,
  unsaved buffer contents. Folder windows without a project file keep their workspace in
  the data folder. Autosaved every 30 s and on exit.
- Hot exit (default on): quitting never prompts; unsaved changes return next launch.

## 10. Language servers

- nvim-lspconfig-style server files (`servers/*.toml`) and languages listing servers in
  preference order; the first installed (by farsub, or on PATH) is used.
- Mason-like manager (Tools → Language Servers): install via npm / pip venv / cargo / go /
  GitHub release / script into the data folder; status, logs, update, uninstall. Placeholders
  `${install_dir}`, `${root}`, `${home}` in server args/env/settings/initialization options.
- Built-in servers (~45). Installable from the manager on macOS, Linux and Windows:
  - npm (needs Node.js): pyright, basedpyright, ts_ls (`typescript@5`, since TypeScript
    7 has no `tsserver`), bashls, yamlls, jsonls / cssls / html_ls
    (vscode-langservers-extracted), taplo, dockerls, intelephense, svelte, elmls, vimls,
    prismals.
  - pip (needs Python): pylsp, cmake_ls. go (needs Go): gopls.
  - GitHub release binaries: rust_analyzer, clangd, lua_ls, marksman, zls, texlab,
    tinymist, clojure_lsp.
  - On PATH only (installed with the language's own toolchain; the description says how):
    ruby_lsp, solargraph, jdtls, kotlin_language_server, sourcekit_lsp, csharp_ls, ols,
    nimlangserver, hls, ocamllsp, elixir_ls, erlang_ls, metals, dartls, gleam, nil_ls,
    nixd, terraformls, r_language_server.
- Root markers may be globs (`*.csproj`, `*.cabal`): a directory matches when any of its
  entries matches.
- Any server with a source that works on this OS/CPU can be installed from the manager. A
  recipe with nothing for this platform shows as "not installed" (no install action), and
  installing it explains why.
- GitHub release source (`[install.github]`, like Mason's): `repo`, optional `tag` (latest
  by default), and `assets` globs keyed `"<os>-<arch>"` or `"<os>"` (os: macos, linux,
  windows; arch: x86_64, aarch64). The download, `.zip` / `.tar.gz` / `.tar.xz` / `.gz` extraction and
  executable lookup all happen inside farsub, so they don't depend on `curl`, `tar` or `sh`
  and work the same on Windows. `GITHUB_TOKEN` is used if set (API rate limit).
- LSP Info (Cmd+Alt+I / status bar): this file's language/syntax, configured servers and
  their status, attached server details (command, root, pid, sync, capabilities, stderr),
  all running servers; `i` install, `r` restart, `s` manager.
- A server that fails `initialize` is shown as failed with its reason (never stuck on
  "starting").

## 10a. Text input boxes

- Every one-line input (find, replace, prompts, Goto Anything and the other palettes, the
  console line, the keymap search) is a one-line editor buffer: it edits exactly like the
  editor, through the same keymap commands and the same code. Word movement, line
  start/end, selection (Shift+…), delete word / to line start, Select All, Undo/Redo and
  Cut/Copy/Paste all work, and rebinding them in `keymap.toml` changes them everywhere.
- Text commands go to the focused input from every path: keys, the macOS menu bar, the
  command palette. Keys that aren't text commands (Enter, Esc, Up/Down, Tab completion)
  belong to the box's owner.
- In an empty input, Backspace and Select All fall through to the owner (Select All with
  an empty console line selects the log text).
- Dropped: the old Ctrl+U "clear input" shortcut; use Select All and type.

## 11. Menus and keys

- macOS: native menu bar. Native shortcuts only come from global/editor bindings and each
  key combination is registered once (a sidebar-only binding must never capture a key
  while typing — e.g. Cmd+X always cuts text in the editor).
- Windows/Linux: in-window menu bar; F10 opens the in-window menus everywhere.
- Cursor movement, selection and deletion are keymap commands (`move_word_left`,
  `select_to_line_end`, `delete_word_left`, … in `keymap.toml`) with platform defaults:
  macOS Option = word, Cmd = line/document, Ctrl+A/E = line start/end; Windows/Linux
  Ctrl = word, Ctrl+Home/End = document. Nothing about them is hard-coded.
- On macOS, Cmd and Ctrl are different keys in bindings (`Cmd+A` ≠ `Ctrl+A`); on
  Windows/Linux both mean Ctrl.
- Keymap viewer: Cmd+Shift+/ lists all bindings; user overrides in `keymap.toml`
  (`command = ""` unbinds, `platforms` limits).
