# Editor

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
- **Files deleted on disk** while open (checked about once a second while the window is
  in use, and when it regains focus): the tab shows a red "(deleted)", the window title
  says "(deleted)", the status bar says so once, and the sidebar stops listing it. The
  buffer keeps its contents and counts as unsaved (as in Sublime), so hot exit keeps them
  across restarts. Closing it asks "…no longer exists on disk. Close it and lose its
  contents?". Saving recreates the file (and its folder). If the file reappears, the
  "(deleted)" mark goes away.
- **Files changed on disk** by another program (same check; decided 2026-11-14, like
  Sublime): an unedited buffer reloads in place as one undoable edit (only the changed
  part is replaced, so the cursor and view stay put), and the status bar says so. A buffer
  with unsaved edits asks "…changed on disk… Reload it and lose them? (y/n)", once per
  change; "n" keeps the edits. Saving over a file that changed since it was read asks
  "Overwrite it with this version?" first. Touching a file without changing its text does
  nothing. "File: Revert File" reloads from disk on demand (undoable).
- **Encodings** (decided 2026-11-14): files open in their own encoding and save back in it,
  byte for byte. A byte-order mark wins (UTF-8, UTF-16 LE/BE; the BOM is kept); valid UTF-8
  is UTF-8; anything else is guessed with `chardetng` (Firefox's detector: Windows-1252 /
  Latin-1, Shift_JIS, GBK, KOI8-R…). A NUL byte without a UTF-16 BOM means binary (not
  opened). A file that isn't plain UTF-8 says so when opened and shows its encoding in the
  status bar (click it, or "File: Convert to UTF-8", to save it as UTF-8 from then on).
  Text the encoding can't hold is never written as substitutes: Save refuses, naming the
  characters, and the file is left alone. Hot exit restores unsaved edits with the file's
  encoding.
- **Crash-safe saving** (decided 2026-11-14): Save writes a temporary file next to the
  original, flushes it to disk and renames it over the original (keeping its
  permissions), so a crash or a full disk never leaves a half-written file. Exceptions,
  written in place instead: symlinks (the link would be replaced by a file), files with
  several hard links (they'd stop sharing the file), and folders where a new file can't be
  created. **Differs from Sublime**, whose `atomic_save` is off by default; there's no
  setting for it here.
- Current-line highlight is **off by default** (`highlight_current_line = true` turns it
  on).
- Mouse hover (LSP): resting the pointer on a symbol for 500 ms shows the server's hover
  info; diagnostics under the pointer are listed on top in their severity color. Works
  without a server for diagnostics alone. The popup closes when the pointer leaves both the
  word and the popup, on typing, or on Esc. Cmd+I shows the same at the cursor.
- Right-click moves the cursor to the click (unless it lands inside the selection, which is
  kept) and opens the editor context menu.

## Scrollbars

- Every editor pane has a vertical scrollbar on the right: thin, wider on hover (Sublime
  style). Drag the thumb, or click the track to jump there. Errors/warnings are marked
  along it in their colors. The text area leaves room for it.
- A horizontal scrollbar appears under the text when a line is wider than the pane (only
  with word wrap off). The vertical scrollbar works in whole lines.
- **Smooth scrolling:** the mouse wheel and trackpad scroll the editor and the sidebar by
  pixels (rows can be partly visible at the top), and the scrollbar thumb moves
  continuously. Moving the cursor, Goto and Page Up/Down still snap to whole lines.
- Scrolling stops hard at the ends: the furthest down is the last line at the top of the
  pane, with no partial row past it (it used to slide a bit further and snap back).
  Sideways, wheel / trackpad scrolling keeps fractions of a column (slow swipes move) and
  stops once the end of the widest line is in view.
- Scrollbars only scroll; the cursor doesn't move. The mouse wheel scrolls anywhere over
  the pane, including over the scrollbars.
