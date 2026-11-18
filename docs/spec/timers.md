# Clock, pomodoro and alarm

Decided 2026-11-16. Three widgets sit at the right end of the tab strip (the rightmost
strip when there are columns), left to right: **alarm, pomodoro, clock**. They're slanted
segments; one not in use (no alarm set, pomodoro not started) is a thin empty stripe and
widens to show its time when used. They're hidden when the strip is too narrow
(< 320 points).

- When tabs overflow, they slide under the widgets along a diagonal edge matching the
  segments' slant (not a straight cut). The middle of that slant is the one boundary
  between them: left of it the mouse goes to the tabs, right of it to the widgets
  (no dead band), and a tab scrolled into view stops there.
- **Hover** highlights a segment (like sidebar rows) and shows a tooltip with its state;
  **click** opens its setup dialog. A segment that's **flashing** after an alert stops
  on a click instead (the next click opens the dialog).
- Commands (palette, Tools menu): Timers: Pomodoro…, Timers: Start / Pause Pomodoro,
  Timers: Alarm…, Timers: Clock….
- **Clock**: local time; 24-hour (default) or 12-hour, seconds optional.
- **Pomodoro**: the classic cycle: work, short break, and a long break after every N
  work sessions (defaults 25 / 5 / 15 minutes, every 4). The segment shows the time left,
  in the work or break color; dimmed while paused or waiting. "Start the next phase
  automatically" (default off): otherwise when a phase ends the next one waits at full
  length until started. Buttons: Start / Pause / Resume, Skip to the next phase, Reset.
  New lengths apply to a phase that hasn't started; a started one keeps its time.
- **Alarm**: one alarm, a local time (HH:MM) with an optional label, once or every day.
  The segment shows its time while set. Ringing clears a one-time alarm (its time stays
  in the dialog for next time); a daily one is set for the next day.
- **Alerts** when a phase ends or the alarm rings: the segment flashes (until clicked),
  plus a system notification and a sound, sent from a background thread so they arrive
  while the window is minimized. Done with what each OS ships: macOS `osascript` and
  `afplay` (Glass); Windows PowerShell toast and `Alarm01.wav`; Linux `notify-send` (or
  `gdbus` to the notification service) and `canberra-gtk-play` / `paplay` (else the
  terminal bell).
- **Persistence**: `timers.toml` in the data folder holds the settings, a running
  pomodoro (its end time) and the alarm, so they survive restarts and keep going by the
  wall clock. A phase that ended or an alarm that rang while farsub was closed alerts on
  the next start ("rang while farsub was closed"). The file is shared by every farsub
  window (decided 2026-11-18): each window checks it about once a second and takes
  another window's change (starting a pomodoro, setting the alarm). A due alert's
  notification and sound come from one window only: the first to claim it, by creating
  a marker file for that alert in `timer-alerts/` in the data folder (creating a file that
  exists fails, so the others skip it; markers older than a day are removed). Every
  window still flashes its widget.
- **Dialog keys**: Up/Down (Tab/Shift+Tab) move between rows, typing edits a field,
  Space flips a toggle or presses a button, Enter presses a button or applies the dialog
  and closes it, Esc closes without applying. A bad value (work length 0, time 25:00) is
  refused with a message and the dialog stays.
- **Theme colors** (`[editor]` keys, all optional): `timer_background` (segments),
  `clock`, `pomodoro` (work), `pomodoro_break`, `alarm`, `timer_alert` (the flash).
