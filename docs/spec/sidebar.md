# Sidebar

- Files and folders matched by `.gitignore` (and `.git/info/exclude`) are listed but
  **dimmed**, as in VS Code; everything inside an ignored folder counts as
  ignored. This works whether or not the folder is a git repository.
- **Slow (network) drives** (decided 2026-11-18, for Samba shares and other slow mounts):
  folders are listed in a background thread, one folder at a time as you open it, so
  the window never waits on the drive. A rebuild of the tree waits up to 60 ms in all for
  listings (a local folder is done by then, so it shows at once); a folder that takes
  longer shows a dim "loading…" after its name and fills in when it arrives. Refreshing
  (after saves and file operations) keeps the old entries on screen until the new
  listing is in. Revealing a file waits for the folders on the way. Each listing is one
  request to the drive: entry types come with the listing (only symlinks are looked up,
  to see whether they point at a folder), and the .gitignore files are read from the
  listed folder and its parents. A folder whose listing fails closes.

- Every project folder is a top-level node. Keyboard: arrows/hjkl move and fold, Enter
  opens, Space marks, `a` new file, `A` new folder, `r` rename/move, `c`/`x`/`p`
  copy/cut/paste, `D` duplicate, `y` copy path, `d`/Delete trash (asks), `.` hidden files.
- Hover highlight; folders open/close with a slide + rotating arrow animation.
- Rows react when the mouse button goes **down** (a folder opens/closes, a file is
  selected), not on release; double-click opens a file. Editor tabs, the bottom panel's
  tabs and the menu-bar titles also react on press (decided 2026-11-15, like VS
  Code). Buttons (a tab's ×, menu items, the find panel's and status bar's buttons) still
  act on release, so moving away before letting go cancels. A tab's × closes only if
  the press was on it too.
- **Opening a file reveals it** (decided 2026-11-16; `reveal_opened_file`, default on):
  opening a file to look at it (Goto Anything, File › Open, a Find in Files result, Go
  to Definition; also when it's already open) selects it in the sidebar, expanding its
  folders, and scrolls it into view. Focus stays in the editor. Files opened in the
  background (Replace in Files, LSP edits, restoring the session) and switching tabs
  don't move the sidebar. With the setting off, View: Reveal File in Sidebar still does
  it on demand.
- **No folder, no sidebar** (decided 2026-11-18): a window with no folder open (an empty
  window, the settings window, a project or session without folders) starts with the
  sidebar hidden, whatever `show_sidebar` or the last session says. Toggle Sidebar
  (Cmd+B) still opens it, showing "No folder open"; opening or adding a folder shows it,
  and removing the last folder hides it.
