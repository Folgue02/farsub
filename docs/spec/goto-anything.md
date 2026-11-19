# Goto Anything (Cmd+P)

- Lists the project's files, **excluding** gitignored files (in or outside a git
  repository), hidden files and the project's exclude patterns.
- A project folder with `index = false` (decided 2026-11-18, for slow network folders)
  isn't read in the background: Goto Anything offers the files the sidebar has listed
  so far (open a folder in the sidebar to add its files), and the title says "unindexed
  folders: listed files only". Find in Files still searches it when asked (in the
  background).
- Each file is a two-line row: its icon, the file name larger and bold,
  and the path below it in a smaller dim font. Fuzzy-match characters are highlighted in
  both lines. Switch Open File uses the same rows.
