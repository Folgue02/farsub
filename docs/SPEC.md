# farsub — product specification

The living description of how farsub works and should behave, one page per area. When a
decision changes, the page is corrected (not appended to). "Cmd" means Cmd on macOS and
Ctrl on Windows/Linux; "Ctrl" means the Control key on every platform.

- [Principles](spec/principles.md): familiar editor feel, own file formats,
  frictionless, keyboard first, nothing hard-coded, never lose work
- [Command line](spec/command-line.md): opening files and folders, `--wait`, `--empty`,
  New Window, headless commands
- [Window layout](spec/window.md): areas of the window, `ui_scale`, editor groups
  (columns), tab dragging, pinned tabs, terminal tabs
- [Editor](spec/editor.md): settings view, config folder window, caret, indentation, word wrap, files deleted or
  changed on disk, encodings, saving, hover, scrollbars and smooth scrolling
- [Languages and highlighting](spec/languages-and-highlighting.md): language detection,
  tree-sitter, themes, installable grammars, the regex fallback
- [Multiple cursors](spec/multiple-cursors.md): Cmd+D, column selection, editing at
  every cursor; no code folding
- [Find, Replace and Find in Files](spec/find.md): the find panel, its toggles, replace,
  searching folders, Find Results
- [File icons](spec/file-icons.md): built-in icons and icon packages
- [Completion](spec/completion.md): the LSP completion list
- [Goto Anything](spec/goto-anything.md): the file finder
- [Bottom panel](spec/bottom-panel.md): terminals (in the panel or as editor tabs) and
  the console
- [Context menus](spec/context-menus.md)
- [Packages](spec/packages.md)
- [Sidebar](spec/sidebar.md)
- [Projects and workspaces](spec/projects.md): project files, workspaces, hot exit
- [Language servers](spec/language-servers.md): server files, the installer and manager,
  LSP Info
- [Text input boxes](spec/inputs.md): every one-line input edits like the editor
- [Menus and keys](spec/menus-and-keys.md): menu bars, platform key defaults, the keymap
- [Clock, pomodoro and alarm](spec/timers.md): the tab-strip widgets, their dialogs,
  alerts and theme colors
- [Licenses and attribution](spec/licenses.md): the Help page listing every crate and
  bundled data in the build, with license texts
- [WakaTime](spec/wakatime.md): built-in time tracking (off by default), the API key in
  `~/.wakatime.cfg`, heartbeats through wakatime-cli
- [Releases](spec/releases.md): versions, the GitHub release workflow and what each
  platform's archive holds
