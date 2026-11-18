# Completion (LSP)

- Opens automatically after 3 identifier characters or a server trigger character, or with
  Ctrl+Space.
- **Nothing is selected when the list opens.** Ctrl+N / Ctrl+P select the next/previous
  item. On Windows/Linux these keys only navigate while the list is open (otherwise they are
  Cmd+N / Cmd+P = New File / Goto Anything). Mouse click selects and accepts.
- Enter / Tab accept the selected item. With nothing selected they behave normally
  (newline / indent) and close the list. Esc closes it. Arrow keys and Page Up/Down move
  the editor cursor and close the list. Typing more characters refilters the list and
  clears the selection again.
- Each item shows a kind icon, then the label and detail. The icons are VS Code's
  **Codicons** (decided 2026-11-18; CC BY 4.0, bundled as `runtime/fonts/codicon.ttf` and
  credited in Help › Licenses), picked per kind as VS Code does (method cube, variable,
  field, property, class, struct, interface, enum, namespace, keyword, snippet, file,
  folder…), colored by group: purple callables, blue values and members, orange types,
  grey the rest. (Visual Studio's own Image Library isn't used: its license only covers
  apps built with Visual Studio.)
