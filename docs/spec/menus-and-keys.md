# Menus and keys

- macOS: native menu bar. Native shortcuts only come from global/editor bindings and each
  key combination is registered once (a sidebar-only binding must never capture a key
  while typing — e.g. Cmd+X always cuts text in the editor).
- Windows/Linux: in-window menu bar; F10 opens the in-window menus everywhere.
- **Opening and saving use the system's file dialogs** (decided 2026-11-15): Cmd+O = File
  › Open File…, File › Open Folder…, Project › Open Project… / Add Folder… / Save Project
  As…, and Save As (also Save on an untitled file). The typed prompt with Tab completion
  stays available as "File: Open Path… (type it)". Open Folder has no default key
  (Cmd+O used to open a folder or a file in one typed prompt).
- Cursor movement, selection and deletion are keymap commands (`move_word_left`,
  `select_to_line_end`, `delete_word_left`, … in `keymap.toml`) with platform defaults:
  macOS Option = word, Cmd = line/document, Ctrl+A/E = line start/end; Windows/Linux
  Ctrl = word, Ctrl+Home/End = document. Nothing about them is hard-coded.
- On macOS, Cmd and Ctrl are different keys in bindings (`Cmd+A` ≠ `Ctrl+A`); on
  Windows/Linux both mean Ctrl.
- Keymap viewer: Cmd+Shift+/ lists all bindings; user overrides in `keymap.toml`
  (`command = ""` unbinds, `platforms` limits), edited with Preferences: Key Bindings
  (File › Key Bindings…) beside the defaults; see [Editor](editor.md). File › Open Config
  Folder ("Preferences: Open farsub's Config Folder") opens the folder holding them in a
  new window.
- **Every shortcut shown comes from the keymap** (decided 2026-11-16): menus, context
  menus, the palette, the find panel, the empty window's hints, the menu bar's hint and
  status messages. They show this platform's names ("Cmd" becomes Ctrl on Windows and
  Linux) and follow rebinding; a hint for an unbound command is left out. No key label is
  written into the code.
