# Window layout

```
┌ menu bar (in-window on Windows/Linux; system menu bar on macOS) ───────────────┐
│ sidebar │ tabs                                     ╲alarm╲pomodoro╲ clock      │
│ FOLDERS │ editor                                                                │
│         ├───────────────────────────────────────────────────────────────────────┤
│         │ bottom panel: TERMINAL n │ CONSOLE  (optional, resizable)              │
├─────────┴───────────────────────────────────────────────────────────────────────┤
│ find panel (Find / Replace / Find in Files), or a prompt                        │
├ status bar ──────────────────────────────────────────────────────────────────────┤
```

- **Tab strip scrolling**: when the tabs don't fit, the mouse wheel over the strip
  (vertical or horizontal) scrolls it sideways, stopping at the first and last tab; it
  doesn't change the active tab or scroll the editor. Switching to a tab that's out of
  view scrolls the strip to show it.
- A tab switches on **press**; its **×** closes on release, and only when the press was
  on the × too. (Switching to a half-hidden tab scrolls it into view, which can slide
  its × under the mouse; that release doesn't close it.)
- **Dragging a tab** (decided 2026-11-20) moves it: onto a tab strip (its own or another
  group's), it goes where the accent bar shows, between two tabs; onto another group's
  editor (which is tinted), it goes to the end of that group's tabs. The moved file
  becomes current and focus follows it. Dropping it anywhere else does nothing.
- The right end of the (rightmost) tab strip holds the alarm, pomodoro and clock
  widgets; see [Clock, pomodoro and alarm](timers.md).

- `ui_scale` (default 1.0, range 0.5–3) zooms the **whole interface**: every font, widget,
  margin and hit area, like browser zoom. It's separate from `font_size` and from View →
  Zoom In/Out (Cmd+= / Cmd+-), which size only the editor text. Changing it
  in the settings file applies immediately. The macOS menu bar is drawn by the OS and isn't
  scaled.
- **Editor groups** (View › Layout): 1–4 columns side by side, each with its own
  tab strip; every file is in exactly one group. Layout: Single / 2 / 3 / 4 Columns
  (macOS Cmd+Alt+1–4; Windows/Linux Alt+Shift+1–4). Focus Group N: Ctrl+1–4. Move File
  to Group N: Ctrl+Shift+1–4 (adds columns if needed, and focus follows the file). Focus
  Next Group has no default key (command palette).
- Clicking in a group focuses it. The focused group's active tab has the accent bar; the
  others are dimmed and show no caret. Dragging a divider resizes the columns
  (double-click evens them out).
- Opening a file puts it in the focused group; a file already open elsewhere is focused
  in its own group. Next/Previous File, Close Other Tabs / Tabs to the Right, and tab
  numbers (Cmd+1–9 on macOS; Alt+1–9 on Windows/Linux) work within the
  focused group.
- A group whose last tab closes stays focused and empty; editing commands do nothing
  there. Removing columns moves their tabs into the last remaining group.
- The layout, each file's group and the focused group are saved in the workspace.
- Rows and grid layouts aren't supported yet (columns only).
- The sidebar is resizable by dragging its right edge (double-click resets); its width is
  saved in the workspace. Cmd+B toggles it, Cmd+0 focuses it.
- The bottom panel is resizable by dragging its top edge; its height and visibility are
  saved in the workspace.
- Status bar, right side (all clickable): LSP status → LSP Info; indentation → Indentation
  menu; syntax name → syntax picker.
