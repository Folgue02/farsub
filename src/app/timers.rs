//! App: the clock, pomodoro and alarm widgets' dialogs (the widgets themselves are drawn
//! by gui/timers.rs; the timing lives in crate::timers).

use super::*;
use crate::timers::{Run, now};

/// Which widget a dialog sets up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerKind {
    Clock,
    Pomodoro,
    Alarm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerAction {
    StartPause,
    Skip,
    Reset,
    SetAlarm,
    ClearAlarm,
    Done,
}

pub enum FieldKind {
    Text(Box<LineEdit>),
    Toggle(bool),
    Button(TimerAction),
}

pub struct Field {
    pub id: &'static str,
    pub label: String,
    pub kind: FieldKind,
}

/// A small form: text fields, toggles and buttons, one row each.
pub struct TimerDialog {
    pub kind: TimerKind,
    pub sel: usize,
    pub fields: Vec<Field>,
    pub error: Option<String>,
}

impl TimerDialog {
    pub fn title(&self) -> &'static str {
        match self.kind {
            TimerKind::Clock => "Clock",
            TimerKind::Pomodoro => "Pomodoro",
            TimerKind::Alarm => "Alarm",
        }
    }

    fn text(&self, id: &str) -> String {
        self.fields
            .iter()
            .find_map(|f| match (&f.kind, f.id == id) {
                (FieldKind::Text(e), true) => Some(e.text().trim().to_string()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn toggle(&self, id: &str) -> bool {
        self.fields
            .iter()
            .any(|f| f.id == id && matches!(f.kind, FieldKind::Toggle(true)))
    }

    /// The selected row's text field, if it is one.
    pub fn input(&mut self) -> Option<&mut LineEdit> {
        match &mut self.fields.get_mut(self.sel)?.kind {
            FieldKind::Text(e) => Some(e),
            _ => None,
        }
    }
}

fn text(id: &'static str, label: &str, v: &str) -> Field {
    Field {
        id,
        label: label.into(),
        kind: FieldKind::Text(Box::new(LineEdit::new(v))),
    }
}

fn toggle(id: &'static str, label: &str, on: bool) -> Field {
    Field {
        id,
        label: label.into(),
        kind: FieldKind::Toggle(on),
    }
}

fn button(label: &str, a: TimerAction) -> Field {
    Field {
        id: "",
        label: label.into(),
        kind: FieldKind::Button(a),
    }
}

impl App {
    /// Open a widget's dialog (and stop it flashing).
    pub fn open_timer_dialog(&mut self, kind: TimerKind) {
        self.completion = None;
        self.hover = None;
        match kind {
            TimerKind::Pomodoro => self.timers.flash.0 = false,
            TimerKind::Alarm => self.timers.flash.1 = false,
            TimerKind::Clock => {}
        }
        let d = self.build_timer_dialog(kind, None);
        self.modal = Some(Modal::Timers(d));
    }

    /// A widget was clicked: a flashing one is dismissed, otherwise its dialog opens.
    pub fn timer_widget_clicked(&mut self, kind: TimerKind) {
        match kind {
            TimerKind::Pomodoro if self.timers.flash.0 => self.timers.flash.0 = false,
            TimerKind::Alarm if self.timers.flash.1 => self.timers.flash.1 = false,
            _ => self.open_timer_dialog(kind),
        }
    }

    /// The status line under a dialog's title.
    pub fn timer_summary(&self, kind: TimerKind) -> String {
        let t = &self.timers;
        let n = now();
        match kind {
            TimerKind::Clock => t.clock.format(n),
            TimerKind::Pomodoro => {
                let p = &t.pomodoro;
                match p.run {
                    Run::Idle => format!("Not started · {} min work", p.work.max(1)),
                    Run::Running { .. } => format!(
                        "{} · {} left",
                        p.position(),
                        crate::timers::countdown(p.remaining(n).unwrap_or(0))
                    ),
                    Run::Paused { left } => format!(
                        "{} · paused, {} left",
                        p.position(),
                        crate::timers::countdown(left)
                    ),
                }
            }
            TimerKind::Alarm => match t.alarm.at {
                Some(at) => {
                    let mins = ((at - n).max(0) + 59) / 60;
                    format!(
                        "Set for {}{} · rings in {}",
                        t.alarm.time,
                        if t.alarm.repeat { ", every day" } else { "" },
                        if mins >= 60 {
                            format!("{} h {} min", mins / 60, mins % 60)
                        } else {
                            format!("{mins} min")
                        }
                    )
                }
                None => "Not set".into(),
            },
        }
    }

    fn build_timer_dialog(&self, kind: TimerKind, sel: Option<usize>) -> TimerDialog {
        let t = &self.timers;
        let fields = match kind {
            TimerKind::Clock => vec![
                toggle("hour24", "24-hour clock", t.clock.hour24),
                toggle("seconds", "Show seconds", t.clock.seconds),
                button("Done", TimerAction::Done),
            ],
            TimerKind::Pomodoro => {
                let p = &t.pomodoro;
                let start = match p.run {
                    Run::Running { .. } => "Pause",
                    Run::Paused { left } if left < p.duration(p.phase) => "Resume",
                    _ => "Start",
                };
                vec![
                    text("work", "Work (minutes)", &p.work.to_string()),
                    text("short", "Short break (minutes)", &p.short_break.to_string()),
                    text("long", "Long break (minutes)", &p.long_break.to_string()),
                    text("every", "Long break after (work sessions)", &p.long_every.to_string()),
                    toggle("auto", "Start the next phase automatically", p.auto_start),
                    button(start, TimerAction::StartPause),
                    button("Skip to the next phase", TimerAction::Skip),
                    button("Reset", TimerAction::Reset),
                    button("Done", TimerAction::Done),
                ]
            }
            TimerKind::Alarm => {
                let a = &t.alarm;
                let mut v = vec![
                    text("time", "Time (HH:MM)", &a.time),
                    text("label", "Label", &a.label),
                    toggle("repeat", "Every day", a.repeat),
                    button(
                        if a.at.is_some() { "Update alarm" } else { "Set alarm" },
                        TimerAction::SetAlarm,
                    ),
                ];
                if a.at.is_some() {
                    v.push(button("Clear alarm", TimerAction::ClearAlarm));
                }
                v.push(button("Done", TimerAction::Done));
                v
            }
        };
        let sel = sel.unwrap_or(0).min(fields.len() - 1);
        TimerDialog {
            kind,
            sel,
            fields,
            error: None,
        }
    }

    /// Take the dialog's field values. Err(message) if one doesn't parse.
    fn apply_timer_dialog(&mut self, d: &TimerDialog) -> Result<(), String> {
        let n = now();
        match d.kind {
            TimerKind::Clock => {
                self.timers.clock.hour24 = d.toggle("hour24");
                self.timers.clock.seconds = d.toggle("seconds");
            }
            TimerKind::Pomodoro => {
                let num = |id: &str, what: &str, max: u32| -> Result<u32, String> {
                    d.text(id)
                        .parse::<u32>()
                        .ok()
                        .filter(|v| (1..=max).contains(v))
                        .ok_or_else(|| format!("{what}: a whole number from 1 to {max}"))
                };
                let work = num("work", "Work", 600)?;
                let short = num("short", "Short break", 600)?;
                let long = num("long", "Long break", 600)?;
                let every = num("every", "Long break after", 50)?;
                let p = &mut self.timers.pomodoro;
                let idle_full = !p.is_running()
                    && matches!(p.run, Run::Paused { left } if left == p.duration(p.phase));
                (p.work, p.short_break, p.long_break, p.long_every) = (work, short, long, every);
                p.auto_start = d.toggle("auto");
                // A phase waiting at full length takes the new length; a started one keeps
                // its time (the new lengths apply from the next phase).
                if idle_full {
                    p.run = Run::Paused {
                        left: p.duration(p.phase),
                    };
                }
            }
            TimerKind::Alarm => {
                let time = d.text("time");
                let parsed = crate::timers::parse_hm(&time);
                if parsed.is_none() && (!time.is_empty() || self.timers.alarm.at.is_some()) {
                    return Err("Time: hours and minutes, like 18:30".into());
                }
                let a = &mut self.timers.alarm;
                let moved = a.time != time || a.repeat != d.toggle("repeat");
                a.time = time;
                a.label = d.text("label");
                a.repeat = d.toggle("repeat");
                if a.at.is_some() && moved {
                    a.arm(n);
                }
            }
        }
        self.timers.changed();
        Ok(())
    }

    /// Run a button (after taking the fields); the dialog stays open, rebuilt.
    fn timer_action(&mut self, mut d: TimerDialog, a: TimerAction) {
        if let Err(e) = self.apply_timer_dialog(&d) {
            d.error = Some(e);
            self.modal = Some(Modal::Timers(d));
            return;
        }
        let n = now();
        let t = &mut self.timers;
        match a {
            TimerAction::Done => return,
            TimerAction::StartPause => t.pomodoro.toggle(n),
            TimerAction::Skip => t.pomodoro.skip(n),
            TimerAction::Reset => t.pomodoro.reset(),
            TimerAction::SetAlarm => {
                if !t.alarm.arm(n) {
                    d.error = Some("Time: hours and minutes, like 18:30".into());
                    self.modal = Some(Modal::Timers(d));
                    return;
                }
            }
            TimerAction::ClearAlarm => t.alarm.disarm(),
        }
        if matches!(a, TimerAction::StartPause | TimerAction::Skip | TimerAction::Reset) {
            t.flash.0 = false;
        }
        t.changed();
        let mut nd = self.build_timer_dialog(d.kind, Some(d.sel));
        // Keep the selection on the same button when rows come and go (Clear alarm).
        if let Some(i) = nd
            .fields
            .iter()
            .position(|f| matches!(f.kind, FieldKind::Button(b) if b == a))
        {
            nd.sel = i;
        }
        self.modal = Some(Modal::Timers(nd));
    }

    pub(super) fn timers_key(&mut self, k: Key, mut d: TimerDialog) {
        let n = d.fields.len();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Up => d.sel = (d.sel + n - 1) % n,
            KeyCode::Down => d.sel = (d.sel + 1) % n,
            KeyCode::Tab if k.shift => d.sel = (d.sel + n - 1) % n,
            KeyCode::Tab => d.sel = (d.sel + 1) % n,
            KeyCode::Enter => return self.timer_activate(d),
            _ => {}
        }
        self.modal = Some(Modal::Timers(d));
    }

    /// Typed characters that aren't for a text field: Space toggles / presses.
    pub(super) fn timers_char(&mut self, c: char) {
        if c != ' ' {
            return;
        }
        let Some(Modal::Timers(d)) = self.modal.take() else {
            return;
        };
        self.timer_press(d);
    }

    /// Enter: a button runs; anywhere else the dialog is applied and closed.
    fn timer_activate(&mut self, d: TimerDialog) {
        match d.fields.get(d.sel).map(|f| &f.kind) {
            Some(FieldKind::Button(_)) => self.timer_press(d),
            _ => {
                if let Err(e) = self.apply_timer_dialog(&d) {
                    let mut d = d;
                    d.error = Some(e);
                    self.modal = Some(Modal::Timers(d));
                }
            }
        }
    }

    /// Space or a click on the selected row: flip a toggle, run a button.
    pub fn timer_press(&mut self, mut d: TimerDialog) {
        let Some(f) = d.fields.get_mut(d.sel) else {
            self.modal = Some(Modal::Timers(d));
            return;
        };
        match &mut f.kind {
            FieldKind::Toggle(on) => {
                *on = !*on;
                self.modal = Some(Modal::Timers(d));
            }
            FieldKind::Button(a) => {
                let a = *a;
                self.timer_action(d, a);
            }
            FieldKind::Text(_) => self.modal = Some(Modal::Timers(d)),
        }
    }

    /// A row was clicked in the dialog.
    pub fn timer_row_clicked(&mut self, row: usize) {
        let Some(Modal::Timers(mut d)) = self.modal.take() else {
            return;
        };
        if row >= d.fields.len() {
            self.modal = Some(Modal::Timers(d));
            return;
        }
        d.sel = row;
        if matches!(d.fields[row].kind, FieldKind::Text(_)) {
            self.modal = Some(Modal::Timers(d));
        } else {
            self.timer_press(d);
        }
    }
}
