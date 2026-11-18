# Context menus

- Right-click opens a context menu in the editor, sidebar (selects the row first), editor
  tabs, terminal and console. **Shift+F10** or the **Menu** key opens the same menu for
  the focused area at the cursor/selected row.
- Items are data: `[context.editor|sidebar|tab|terminal|console] items = [...]` in
  `menus.toml` (command ids, `"-"` separators); user files override per section.
- Keyboard: arrows move, Enter runs, a letter runs the item starting with it, Esc or a
  click outside closes. Disabled items are dimmed and skipped.
- egui's own widget focus is never used: every key goes through the app's routing, so Tab
  never moves focus to (and Enter never "clicks") a menu-bar button.
- **Popups own the mouse** (decided 2026-11-18): a context menu, a menu bar dropdown,
  the completion list and the command palette take the mouse over their whole box (gaps
  and padding included); a dialog that dims the window (Keyboard Shortcuts, Language
  Servers, LSP Info, confirmations, the timer dialogs) takes it over the whole window.
  What's underneath doesn't react or show its cursor (no I-beam from the editor or
  terminal). The LSP hover popup is the exception: the mouse can move onto it to keep it
  open.
