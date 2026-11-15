# Multiple cursors

Decided 2026-11-15: as in Sublime, every cursor is a full selection. Typing, deleting,
moving, Enter, Tab and the edit commands act at all of them at once, and one Undo undoes
the edit everywhere (and brings the cursors back). Code folding was considered and
dropped (the user doesn't want it).

## Making cursors

| Action | macOS | Windows / Linux |
|---|---|---|
| Select the word, then add its next occurrence | Cmd+D | Ctrl+D |
| Select every occurrence | Cmd+Alt+G (Sublime's Ctrl+Cmd+G can't be written in the keymap), Alt+F3 | Alt+F3 |
| Split the selection into one per line | Cmd+Shift+L | Ctrl+Shift+L |
| Add a cursor on the line above / below | Ctrl+Shift+Up / Down | Ctrl+Alt+Up / Down |
| Add a cursor, or remove the one clicked | Cmd+Click | Ctrl+Click |
| Add a word | Cmd+double-click | Ctrl+double-click |
| Column (box) selection | Alt+drag | Alt+drag |
| Find All: a cursor on every match | Alt+Enter in the find panel | Alt+Enter in the find panel |
| Back to one cursor | Esc | Esc |

- **Cmd+D** from a bare caret selects the word and then adds the next occurrences as
  **whole words, case-sensitive** ("foo" skips "foobar"). From text you selected
  yourself it matches anywhere in words. It wraps around the file, skips what's already
  selected, and says "No more occurrences" at the end. The newest selection is the
  primary one: the view follows it.
- Select All Occurrences uses the selection, or the word at the caret (whole words).
- Add Cursor Above / Below starts from the first / last selection and keeps the column
  it aimed for (like Up / Down).
- Alt+drag selects the same display columns on every line it crosses. Lines too short to
  reach the box are skipped, unless the box is only a column of carets.
- On macOS Ctrl+Shift+Up / Down used to move lines; Move Line stays on Alt+Up / Down (as
  before) and keeps Ctrl+Shift+Up / Down on Windows / Linux.
- A plain click (or Goto, Select All, reloading the file) goes back to one cursor.

## Editing with several cursors

- Every cursor gets the edit. Cursors whose selections end up overlapping, or carets
  that meet, merge into one.
- Line commands (Delete Line, Duplicate, Move Line, Toggle Comment, Indent / Unindent,
  Join Lines, Insert Line Before / After) run **once per line**: two cursors on the same
  line don't delete two lines.
- **Copy / Cut:** each selection's text, one per line of the clipboard. With only carets
  (nothing selected), each cursor's whole line, like Sublime.
- **Paste:** when the clipboard has as many pieces as there are cursors (what they
  copied, or as many lines), each cursor gets its own piece; otherwise every cursor
  gets the whole text.
- Completion and language-server edits apply at the primary cursor; the others move
  with the text.
- Drawing: every selection is highlighted and every caret drawn, blinking together; the
  current-line highlight (if on) marks each caret's line. The status bar says
  "3 selection regions" instead of the line and column.
