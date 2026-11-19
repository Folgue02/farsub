# Editor

- **Settings** (Preferences: Settings, Cmd+,): a **window of its own**
  (decided 2026-11-15; another farsub process, `--settings`, with the same config and data
  folders) with two panes. On the left, `Default Settings.toml`: read-only, every setting
  with its default and an explanation, regenerated from the running version. On the
  right, your settings file, focused. A new user file starts nearly empty;
  you copy over only what you change. Saving applies it in every window: each farsub
  window checks the settings file about once a second and reloads it when it changes
  (unless it has the file open with unsaved edits). Closing either pane closes both (the
  other only if it has no unsaved changes) and then the window.
- **Key Bindings** (Preferences: Key Bindings, decided 2026-11-18): the same kind of
  window (`--keymap`). Left, `Default Key Bindings.toml`: read-only, every command id you
  can bind with its name, then the built-in keymap as it is. Right, your `keymap.toml`
  (next to the settings file), created with a commented example if missing. Your
  bindings are added to the defaults; one for the same keys replaces it. Saving applies
  it; every window also checks `keymap.toml` about once a second, like the settings file.
  Running Settings or Key Bindings inside that window opens the other pair there too;
  the window closes when the last pair does. Project › Edit Project is the same kind of
  window for a project file (see [Projects](projects.md)).
- With taplo installed (it installs itself on the first TOML file when `auto_install` is
  on), these files are checked against farsub's schemas: a misspelled setting or an
  unknown command id is an error, keys complete, and hovering one explains it (see
  [Language servers](language-servers.md)).
- **Config folder created on first start** (decided 2026-11-15): if the settings file
  doesn't exist, farsub creates its folder and a starter `config.toml`, so a new user always
  has one to open. Standard places per platform: `%APPDATA%\farsub` on Windows (data in
  `%LOCALAPPDATA%\farsub`), `$XDG_CONFIG_HOME/farsub` or `~/.config/farsub` elsewhere (data
  in `$XDG_DATA_HOME/farsub` or `~/.local/share/farsub`). Windows has no `$HOME` by
  default; `~` means `%USERPROFILE%` there. `FARSUB_CONFIG` / `FARSUB_DATA` override them.
- **Font** (`font_face`, default `""` = the built-in Hack):
  any installed font family. Preferences › Font… ("Preferences: Select Font") lists the
  installed fonts, monospaced ones first; moving through the list previews each in the
  editor, Enter keeps it and **writes `font_face` into your settings file** (no editing
  by hand), Esc goes back. It applies to monospace text (the editor, input boxes); the
  built-in font fills in characters the chosen one lacks. Fonts egui can't draw (no
  glyph outlines, like macOS's bitmap-only "GB18030 Bitmap", whose line height comes
  out as NaN and crashed the editor) aren't listed; a `font_face` naming one, or a font
  that isn't installed, falls back to the built-in one with a message. **Not the system
  font dialog** (decided 2026-11-15): macOS's font panel is a floating panel that reports
  through the window's responder chain, which egui/winit doesn't expose; Linux has no
  font chooser portal (only GTK's own dialog); only Windows' ChooseFont would be easy,
  and a picker on one platform only goes against the same-everywhere principle. The
  list also previews live in the editor, which the native panels don't.
  Select Color Theme now also keeps the chosen theme in the settings file.
- **Trailing spaces on blank lines** (`trim_automatic_white_space`, default on):
  Enter on a line holding only indentation empties that line (the new line
  still gets the indentation), so blank lines between blocks don't keep spaces. Moving
  the cursor off such a line without pressing Enter leaves it as it is.

- **Mouse selection** (decided 2026-11-18, as in VS Code): the caret goes
  where the button went down, so a drag can start on a line's first character. Double-
  click and drag extends by whole words, triple-click and drag by whole lines, always
  keeping the word or line first clicked (also when dragging backwards). Clicks are
  counted on the press (within 0.5 s and a few pixels of the last one). Dragging past
  the top or bottom of the pane, also outside the window, keeps selecting and scrolls
  toward the pointer, faster the further away it is, until the button is released.
- Mouse pointer: I-beam over text, normal arrow over selected text, arrow over
  the gutter.
- Indentation: detected per file on open (tabs vs spaces; space width from the most common
  indentation change between lines; a file indented with N spaces gets tab width N). The
  Indentation menu offers spaces/tabs, tab width 1–8, guess from buffer, convert to
  spaces/tabs (one undo step). Manual choices are remembered per file in the workspace.
- Indentation guides at every level; the block around the cursor is highlighted.
  `indent_guides = false` disables them.
- Caret: width = 1 + `caret_extra_width`; it reaches
  `caret_extra_top` px above and `caret_extra_bottom` px below the line. Defaults 1.5 / 1 /
  1: a little thicker than a hairline and a little taller than the line.
- Caret animation `caret_style`: `"smooth"` (default: fades out and back in
  once per second), `"blink"` (hard on/off, 0.5 s each) or `"solid"`. It stays solid for
  0.5 s after every move or edit, and only animates while the window has focus (no
  background repaints). The smooth fade redraws at ~30 fps (it used to ask for 60), which
  halves idle redraws while the window is focused.
- Goto Symbol (Cmd+R) lists symbol **names only**: `add`, `Point`,
  `impl Debug for Point<T>`, Markdown headings without `#`. The kind (fn, struct, class…)
  is shown dimmed on the right, and (decided 2026-11-18) as its Codicon before the name,
  the same icons as completion, for symbols from a language server (Goto Symbol and
  Goto Symbol in Project); signatures never appear. Nested symbols are indented, icon
  included.
  Results come from the language server when one is attached, otherwise from a built-in
  scan of declaration keywords.
- **Word wrap** (`word_wrap`, default **on**): long lines wrap at the pane width, breaking
  after spaces when possible (mid-word otherwise). Continuation rows are indented like
  the line; the line number and diagnostic dot show
  on the first row only; indent guides run through all rows. Up/Down move by visual
  rows, keeping the visual column; Home/End and Page Up/Down still work on whole lines.
  Clicking past the end of a wrapped row puts the caret at that row's end. No horizontal
  scrolling while wrapping. View › Word Wrap / "View: Toggle Word Wrap" (Alt+Z) toggles
  it per file, remembered in the workspace. The default is on everywhere,
  code included, not only for prose (Markdown, plain text).
- **Files deleted on disk** while open (checked about once a second while the window is
  in use, and when it regains focus; the check runs in a background thread, so a slow
  network drive doesn't freeze the window, and a result for a file that was saved or
  renamed meanwhile is dropped): the tab shows a red "(deleted)", the window title
  says "(deleted)", the status bar says so once, and the sidebar stops listing it. The
  buffer keeps its contents and counts as unsaved, so hot exit keeps them
  across restarts. Closing it asks "…no longer exists on disk. Close it and lose its
  contents?". Saving recreates the file (and its folder). If the file reappears, the
  "(deleted)" mark goes away.
- **Files changed on disk** by another program (same check; decided 2026-11-14):
  an unedited buffer reloads in place as one undoable edit (only the changed
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
  created. It's always on; there's no setting for it.
- Current-line highlight is **off by default** (`highlight_current_line = true` turns it
  on).
- Mouse hover (LSP): resting the pointer on a symbol for 500 ms shows the server's hover
  info; diagnostics under the pointer are listed on top in their severity color. Works
  without a server for diagnostics alone. The popup closes when the pointer leaves both the
  word and the popup, on typing, or on Esc. Cmd+I shows the same at the cursor.
- Right-click moves the cursor to the click (unless it lands inside the selection, which is
  kept) and opens the editor context menu.

## Scrollbars

- Every editor pane has a vertical scrollbar on the right: thin, wider on hover.
  Drag the thumb, or click the track to jump there. Errors/warnings are marked
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
