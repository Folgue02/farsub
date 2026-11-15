# Command line

- `farsub [PATHS…]` opens files (`FILE[:LINE[:COL]]`), folders and `.farsub-project` files;
  `-` reads stdin into a new untitled buffer.
- **Does not block the shell**: the command hands off to a background process and returns
  immediately (like `subl`). `-w/--wait` keeps it in the foreground until the window
  closes, for use as `$EDITOR` / `git config core.editor "farsub -w"`.
- Argument errors are reported in the shell (exit code 2) before handing off.
- `--empty` opens a clean window: no folder, no project, no session (files given too still
  open in it). File › New Window (Cmd+Shift+N, as in Sublime) starts one as a separate
  farsub process, with the same `--config` / `--data-dir`. An empty window doesn't
  replace the "last session" and keeps no workspace; its sidebar says "No folder open".
- `--settings` opens a window with just the two settings panes (Preferences: Settings
  starts one); closing them closes the window. Like `--empty`, it keeps no workspace.
- What opens: a project file → that project; folders → those folders (or added to the last
  session with `-a`); files only → the last session (or the file's project root if there
  is none); nothing → the last session, else the current folder. `-n` skips the session.
- Headless commands run in the foreground and never open a window: `--list-servers`,
  `--install-server`, `--uninstall-server`, `--check-config`. `--config` and `--data-dir`
  redirect config/data folders.
