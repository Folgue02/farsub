# Find, Replace and Find in Files

Decided 2026-11-15, following Sublime: one find panel at the bottom of the window, above
the status bar and across its whole width. It has three modes. Each mode shows its fields
and its toggles; the texts and toggles are kept while it's closed, and saved in the
workspace.

| Mode | Opens with | Fields |
|---|---|---|
| Find | Cmd+F | Find |
| Replace | Cmd+Alt+F (Ctrl+H on Windows/Linux) | Find, Replace |
| Find in Files | Cmd+Shift+F (from anywhere, no file needed) | Find, Where, Replace |

## The panel

- Opening it puts a single-line selection into Find, otherwise the last search, all
  selected so typing replaces it. A selection of several lines becomes the "in
  selection" scope instead.
- **Toggles** (left of the fields, clickable, with tooltips): regular expression (`.*`,
  Cmd+Alt+R; Alt+R on Windows/Linux), case sensitive (`Aa`, Cmd+Alt+C / Alt+C), whole word
  (`"ab"`, Cmd+Alt+W / Alt+W), wrap around, in selection. Find in Files has context
  instead of wrap and in selection. Every toggle is also a command ("Find: Toggle …") and
  sits in the Find menu. Defaults: everything off except wrap and context.
  **Differs from Sublime:** no "preserve case" and no highlight-matches toggle (matches
  are always outlined).
- Case-insensitive by default, as in Sublime. The old prompt's smart case (an uppercase
  letter meant case-sensitive) is gone: the toggle decides.
- Whole word: a match that begins or ends with a word character (letter, digit, `_`)
  mustn't be continued by another one just outside it. So `foo` doesn't match in
  `foobar` or `foo_x`, and `foo(` still matches in `x foo(`.
- Regex syntax is Rust's `regex` crate. It runs line by line (`^` and `$` are line
  starts and ends), so a match never spans lines. A bad pattern is reported next to the
  field in red, and nothing moves.
- Keys in the panel: the fields edit like every input box. **Enter** = find next,
  **Shift+Enter** = previous; in the Replace field Enter = replace and find next.
  **Tab / Shift+Tab** move between the fields. **Esc** closes the panel and keeps the
  match selected (also from the file, while the panel is open). Shortcuts keep working
  in the panel, including the find family's editor keys (F3, Cmd+G…). Clicking a field
  focuses it; clicking the file gives the file the focus and leaves the panel open.
- Buttons: Find, Prev (Find); Replace, Replace All (Replace); Find, Add Folder…, Replace
  (Find in Files). Add Folder… opens the system folder picker and appends the folder to
  Where.

## Find and Replace in a file

- Searches **as you type** from where the cursor was when the panel opened: the first
  match is selected and scrolled into view. With no match the cursor goes back there and
  the field turns red ("No results").
- While the panel is open every match in view is outlined (theme color
  `find_highlight`, else the accent color). The panel shows "3 of 12" when the selection
  is a match, otherwise "12 matches" (counting stops past 50,000).
- Find Next / Previous: F3 / Shift+F3, and Cmd+G / Cmd+Shift+G on macOS. With wrap on,
  passing the end goes back to the top, and the status bar says "Find wrapped to the
  top".
- **Replace** (Cmd+Alt+E on macOS, Ctrl+Shift+H on Windows/Linux, or Enter in the Replace
  field) replaces the selected match, then selects the next one. If the selection isn't
  a match, it just finds the next one. **Replace All** (Ctrl+Alt+Enter) replaces every
  match in the file, or in the selection with "in selection" on, as one undo step, and
  says how many.
- With regex on, the replacement can use groups: `$1`, `${name}`, `\1`, plus `\n` and
  `\t`. `$$` and `\$` give a literal `$`. Without regex the replacement is inserted
  exactly as typed.
- Read-only files (like Find Results) can be searched but not replaced in.

## Find in Files

- **Where** is a comma-separated list. Each entry is one of:
  - a folder or file: absolute, `~/…`, or relative to the first project folder
  - `<project folders>` (also what an empty field means), `<open files>`, `<current file>`
  - an include filter: `*.rs`, `src/*.ts`
  - an exclude filter: `-target/`, `-*.min.js`
  
  Filters on their own mean the project folders. A filter matches a file's name or its
  path inside the folder being searched; an exclude also matches folder names.
  Unknown paths are an error, shown next to Where.
- What's skipped: `.gitignore`'d files and `.git/info/exclude` (git repository or not,
  like the sidebar), `.git` itself, the project's folder/file exclude patterns, binary
  files (a NUL byte in the first 8 KB) and files over 32 MB. Hidden files are searched.
- Open files with unsaved changes are searched as they are in the editor.
- The search runs in the background, reading and searching files on every CPU core in
  batches (results still come in file-name order; about 1 s for 40,000 files / 1.4 GB);
  a new search cancels the previous one. Results go
  into a read-only **"Find Results"** tab (reused by the next search). It starts with what
  was searched, then each file's path followed by its matching lines, numbered like
  `  12: text`. With context on there are 2 lines around each match (`  11  text`) and
  `..` between separate groups. It ends with "N matches across M files (K searched) in
  0.12 s". It stops after 20,000 matches and says so. File names are colored as headings
  (theme capture `markup.heading`), line numbers and context dimmed (`comment`), and
  matches outlined.
- **Opening results:** Enter or double-click on a result line opens the file at that
  match, selected; on a file name, at its first match. **F4 / Shift+F4** ("Find: Next /
  Previous Result") step through the match lines from anywhere: they move the cursor in
  Find Results and open the match.
- Find Results isn't unsaved work: no close prompt, and it's not kept by hot exit.
- **Replace in files:** Enter in the Replace field (or the Replace button) runs the
  search, then asks "Replace N matches in M files with "…"?". Yes opens each file and
  replaces in it, leaving the changes **unsaved** (as in Sublime), so you can review them
  and File › Save All writes them. Nothing on disk changes until you save.
