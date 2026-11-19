# File icons

- File-type icons in the sidebar (folders get an open/closed folder), editor tabs, Goto
  Anything and Switch Open File. `file_icons = false` turns them off.
- Built-in icons are drawn (no font or image files): a coloured badge with a 1–3 letter
  label per file type, and a plain page for unknown types. The mapping is data:
  `runtime/icons.toml` (`[[icons]]` with `names` / `extensions`, `label`, `color`;
  `[default]`, `folder_color`). Exact names win over extensions, the longest extension
  wins (`.d.ts` beats `.ts`), and later rules win ties.
- **Icon packages:** any folder in `Packages/` with an `icons.toml` (installed with
  "Packages: Install Package (git URL)", "…from File…" or "…from Folder…") is an icon package. Its rules may use `image =
  "path.png"` (relative to the package) instead of a label, plus `folder_image` /
  `folder_open_image`. All packages load in name order after the built-ins, then the
  user's own `icons.toml` on top. `icon_theme = "<package>"` uses only that package. PNG
  only (no SVG). Packages: Reload applies changes.
- Icon packages made for other editors (e.g. A File Icon) are **not** compatible: they map
  icons through that editor's syntax/theme preference files, a different format.
