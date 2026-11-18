# Projects and workspaces

- `NAME.farsub-project`: folders (path, name, exclude patterns, `index = false` for a
  slow network folder Goto Anything shouldn't read in full; see
  [Goto Anything](goto-anything.md)), `[settings]` overriding
  editor settings, optional `[[languages]]`/`[[servers]]`. Highlighted as TOML.
- **Edit Project** (Project › Edit Project, decided 2026-11-18): like Preferences:
  Settings, a **window of its own** (`--edit-project FILE`). Left, `Example
  Project.farsub-project`: read-only, every part of a project file with comments. Right,
  the project file. Saving it there applies it in the project's window at once: each
  window checks its project file about once a second (folders, settings, languages and
  servers reload in place; a copy with unsaved edits open in that window is left alone).
  Closing either pane closes the window. A window without a project file says to Save
  Project As first.
- **New Project** (Project › New Project, decided 2026-11-18): the same window
  (`--new-project`), with an unsaved project file on the right (one folder, `path =
  "."`, and an empty `[settings]`). Saving asks where to put it (suggests `New
  Project.farsub-project`, adds the extension if left out); it isn't opened anywhere by
  itself.
- **Open This Project in New Window** (decided 2026-11-18; "this" because it opens the
  project file at hand, not the window's own project): opens a `.farsub-project` file as
  its project in another farsub window. The file is the one selected in the sidebar (when
  the sidebar has focus), else the current file. In the Project menu and the command
  palette, and in the sidebar, tab and editor right-click menus, which show it only for
  project files.
- `NAME.farsub-workspace` next to it: open files, cursors, scroll, syntax/indentation
  overrides, expanded folders, sidebar width, panel state, find text, the find panel's toggles and its Where / Replace texts, closed files,
  unsaved buffer contents. Folder windows without a project file keep their workspace in
  the data folder. Autosaved every 30 s and on exit.
- Hot exit (default on): quitting never prompts; unsaved changes return next launch.
