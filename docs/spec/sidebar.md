# Sidebar

- Files and folders matched by `.gitignore` (and `.git/info/exclude`) are listed but
  **dimmed**, as in Sublime 4 / VS Code; everything inside an ignored folder counts as
  ignored. This works whether or not the folder is a git repository.

- Every project folder is a top-level node. Keyboard: arrows/hjkl move and fold, Enter
  opens, Space marks, `a` new file, `A` new folder, `r` rename/move, `c`/`x`/`p`
  copy/cut/paste, `D` duplicate, `y` copy path, `d`/Delete trash (asks), `.` hidden files.
- Hover highlight; folders open/close with a slide + rotating arrow animation.
- Rows react when the mouse button goes **down** (a folder opens/closes, a file is
  selected), not on release; double-click opens a file. Editor tabs, the bottom panel's
  tabs and the menu-bar titles also react on press (decided 2026-11-15, like Sublime / VS
  Code). Buttons (a tab's ×, menu items, the find panel's and status bar's buttons) still
  act on release, so moving away before letting go cancels.
