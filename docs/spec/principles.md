# Principles

What farsub is trying to be; every other page follows from these.

- **Sublime Text feel.** Layout, keys, palettes and behaviors follow Sublime Text unless a
  decision below says otherwise. Where VS Code has the better-known convention (integrated
  terminal), we follow VS Code.
- **Inspired by Sublime, not built from Sublime.** farsub borrows Sublime's good
  experiences (layout, keys, behaviors) but not its file formats or engines: no
  `.sublime-syntax`, `.tmTheme` or Sublime packages. Highlighting is tree-sitter; themes,
  queries and packages are farsub's own formats (decided 2026-11-14, done the same day).
- **Frictionless.** Users never have to install things by hand to get a working editor:
  what a file needs (its grammar, its language server) installs itself in the background
  when the file opens (`auto_install`, on by default). Configuration is for customizing
  and enjoying the app, not for making it work. Installing extras is one step: pick a
  file or folder, or a name from a list.
- **Keyboard first, mouse welcome.** Every action is reachable from the keyboard (shortcut,
  command palette, or F10 menus). Mouse interactions mirror what Sublime/VS Code do.
- **Nothing hard-coded.** Languages, servers, key bindings and menus ship as data in
  `runtime/` and can be overridden from the user's config folder. Code only implements
  commands and mechanisms.
- **Never lose work.** Hot exit keeps unsaved changes across restarts; destructive file
  operations go to the system Trash and ask first.
