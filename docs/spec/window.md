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
- **Pinned tabs** (decided 2026-11-24): File: Pin Tab / File: Unpin Tab, in the command
  palette and the tab's right-click menu (which shows only the one that applies).
  - A pinned tab is drawn **icon only** (about 40px, no ×), and its name shows on hover.
    A modified pinned file shows a small dot by its icon. The last pinned tab has a
    heavier right edge, separating the pinned tabs from the rest.
  - Pinned tabs sit at the **start** of their group's strip. Pinning puts the tab after
    the other pinned ones; unpinning puts it first among the unpinned ones.
  - Dragging keeps that order: an unpinned tab dropped among the pinned ones goes right
    after them, and a pinned tab dropped among the unpinned ones goes right before them.
    Moved to another group, a tab keeps its pin and joins that group's pinned tabs.
  - **Close Other Tabs** and **Close Tabs to the Right** leave pinned tabs open. Closing
    a pinned tab itself (Cmd+W, middle-click, File: Close) still closes it.
  - Pins are kept in the workspace, so they survive restarts and project switches.
  - Pinning doesn't change the Ctrl+Tab recent order or the tab numbers (Cmd+1–9 count
    pinned tabs first, as they're drawn).
- **Terminal tabs** (decided 2026-11-24): Terminal: New Terminal in Editor Area
  (**Ctrl+Alt+`**, View menu, command palette) opens a shell as a tab of the focused
  group, next to files. See [Bottom panel](bottom-panel.md) for how terminals behave.
  - It looks different from a file tab: the terminal's own darker background, a green
    bar on top instead of the accent one, a green `>_` icon, and a monospace title of
    the shell and the folder it started in (`zsh · editor`). When the shell exits with a
    non-zero code the icon turns red and `exit N` shows before the ×.
  - It's a tab like any other: it drags between groups, pins, takes part in Ctrl+Tab
    recent switching and Next/Previous File, and Cmd+1–9 selects it. With it focused,
    keys go to the shell, except Cmd shortcuts on macOS, the terminal's few
    always-available shortcuts, and switching tabs and groups (Ctrl+Tab, Ctrl+PageUp/
    PageDown, Focus Group).
  - File and editing commands (Save, Find, Set Syntax, …) do nothing in it and are
    greyed out in menus; the status bar shows no line, column or syntax.
  - Closing it while its shell is still running **asks first** ("The terminal … is
    still running. Kill it and close the tab?"); an exited one closes without asking.
    Close Other Tabs / Tabs to the Right keep running terminals open (and say so).
  - Terminal tabs aren't kept by the workspace or hot exit (a shell can't be brought
    back), and Reopen Closed File doesn't bring them back.
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
- **Ctrl+Tab switches to recent files** (decided 2026-11-20; Ctrl on every platform,
  also macOS): it goes to the file used before the current one, in any group. Pressing
  Tab again while Ctrl is still held goes one further back (A → B → C), and the order
  doesn't change until Ctrl is let go; then the file it stopped on counts as the most
  recent. So Ctrl+Tab, released, flips between the last two files. Ctrl+Shift+Tab goes
  the other way (from the least recent). No list pops up: the tabs switch as you go.
  From the command palette ("View: Switch to Recent File") it acts like one press.
  Next/Previous File in tab order stay on Ctrl+PageDown/PageUp and Cmd+Alt+→/←. In the
  bottom panel's terminal, Ctrl+Tab still cycles the panel's terminals; in a terminal
  tab it switches tabs like anywhere in the editor area. On macOS, farsub turns off the system's
  window tabbing, whose "Show Next Tab" / "Show Previous Tab" Window-menu items would
  otherwise take Ctrl+Tab and Ctrl+Shift+Tab (farsub windows are separate windows).
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
