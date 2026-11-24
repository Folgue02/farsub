# Command line

- `farsub [PATHS…]` opens files (`FILE[:LINE[:COL]]`), folders and `.farsub-project` files;
  `-` reads stdin into a new untitled buffer.
- **Does not block the shell**: the command hands off to a background process and returns
  immediately (like `subl`). `-w/--wait` keeps it in the foreground until the window
  closes, for use as `$EDITOR` / `git config core.editor "farsub -w"`.
- **Files open in the window used last** (decided 2026-11-20): when the arguments are
  only files (and/or `-`), and a FarSub window with the same data folder is running,
  they open in the most recently focused one, which comes to the front; no new window
  starts. Folders, projects, no arguments, `-w`, `-n`, `-a`, `--empty`, the settings
  options and `--new-window` still start a new window. How: each window (except the
  settings ones) listens on a localhost port and lists itself in
  `<data>/windows/<pid>-<port>.json` (port, a random token the request must carry, last
  focus time; owner-only on Unix); the listing goes when the window closes, and one whose
  window doesn't answer is removed.
- Argument errors are reported in the shell (exit code 2) before handing off.
- `--empty` opens a clean window: no folder, no project, no session (files given too still
  open in it). File › New Window (Cmd+Shift+N) starts one as a separate
  FarSub process, with the same `--config` / `--data-dir`. An empty window doesn't
  replace the "last session" and keeps no workspace; it starts without the sidebar (see
  [Sidebar](sidebar.md)).
- `--settings` opens a window with just the two settings panes (Preferences: Settings
  starts one); closing them closes the window. Like `--empty`, it keeps no workspace.
  `--keymap` is the same for the key bindings panes (Preferences: Key Bindings), and
  `--edit-project FILE` for a project file beside the example project (Project › Edit
  Project), and `--new-project` for a new, unsaved one (Project › New Project).
- What opens: a project file → that project; folders → those folders (or added to the last
  session with `-a`); files only → the last session (or the file's project root if there
  is none); nothing → the last session, else the current folder. `-n` skips the session.
- Headless commands run in the foreground and never open a window: `--list-servers`,
  `--install-server`, `--uninstall-server`, `--check-config`. `--config` and `--data-dir`
  redirect config/data folders.
