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
- Each item shows a kind icon: a small colored badge with a glyph per kind (function,
  method, variable, field/property, class/struct, interface, module, keyword, snippet,
  constant, enum, file…), plus the label and detail.
