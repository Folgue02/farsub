# Text input boxes

- Every one-line input (the find panel's Find / Where / Replace fields, prompts, Goto Anything and the other palettes, the
  console line, the keymap search) is a one-line editor buffer: it edits exactly like the
  editor, through the same keymap commands and the same code. Word movement, line
  start/end, selection (Shift+…), delete word / to line start, Select All, Undo/Redo and
  Cut/Copy/Paste all work, and rebinding them in `keymap.toml` changes them everywhere.
- Text commands go to the focused input from every path: keys, the macOS menu bar, the
  command palette. Keys that aren't text commands (Enter, Esc, Up/Down, Tab completion)
  belong to the box's owner.
- In an empty input, Backspace and Select All fall through to the owner (Select All with
  an empty console line selects the log text).
- **Password fields** (the WakaTime API key): what's typed or pasted is drawn as one dot
  per character. Copy and Cut do nothing there, and the text never reaches the log.
  Editing otherwise works as in any input.
- Dropped: the old Ctrl+U "clear input" shortcut; use Select All and type.
