# Menus and keys

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
