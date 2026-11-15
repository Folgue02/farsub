# Projects and workspaces

- `NAME.farsub-project`: folders (path, name, exclude patterns), `[settings]` overriding
  editor settings, optional `[[languages]]`/`[[servers]]`.
- `NAME.farsub-workspace` next to it: open files, cursors, scroll, syntax/indentation
  overrides, expanded folders, sidebar width, panel state, find text, the find panel's toggles and its Where / Replace texts, closed files,
  unsaved buffer contents. Folder windows without a project file keep their workspace in
  the data folder. Autosaved every 30 s and on exit.
- Hot exit (default on): quitting never prompts; unsaved changes return next launch.
