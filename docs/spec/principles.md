# Principles

What FarSub is trying to be; every other page follows from these.

- **Familiar editor feel.** Layout, keys, palettes and behaviors follow the conventions of
  established desktop code editors unless a decision below says otherwise. Where VS Code
  has the better-known convention (integrated terminal), we follow VS Code.
- **Own formats.** FarSub borrows good experiences (layout, keys, behaviors) from other
  editors but not their file formats or engines: no `.tmTheme` themes or other editors'
  packages. Highlighting is tree-sitter; themes, queries and packages are FarSub's own
  formats (decided 2026-11-14, done the same day). The one exception: the fallback for
  languages without a tree-sitter grammar uses syntect, which reads Sublime Text's
  `.sublime-syntax` grammar format (bat's ~220 grammars plus `runtime/syntaxes/`).
- **Frictionless.** Users never have to install things by hand to get a working editor:
  a file's highlighting grammar installs itself in the background when the file opens
  (`auto_install`, on by default). Language servers are the exception (decided
  2026-11-19): they're big downloads and running programs, so they install only when
  the user picks one in Tools › Language Servers; opening a file says which can. Configuration is for customizing
  and enjoying the app, not for making it work. Installing extras is one step: pick a
  file or folder, or a name from a list.
- **Keyboard first, mouse welcome.** Every action is reachable from the keyboard (shortcut,
  command palette, or F10 menus). Mouse interactions mirror what VS Code does.
- **Nothing hard-coded.** Languages, servers, key bindings and menus ship as data in
  `runtime/` and can be overridden from the user's config folder. Code only implements
  commands and mechanisms.
- **Never lose work.** Hot exit keeps unsaved changes across restarts; destructive file
  operations go to the system Trash and ask first.
