//! The clock, pomodoro and alarm shown at the right of the tab strip.
//!
//! Everything here is a function of "now" (Unix seconds) so it can be tested; the state
//! is saved in `timers.toml` in the data folder, so a running pomodoro and a set alarm
//! survive restarts (they keep going by the wall clock while farsub is closed).
//!
//! Alerts (system notification + sound) are fired by a small background thread when a
//! deadline passes, so they arrive even while the window is minimized and not drawing;
//! the GUI flashes the widget on its next frame.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ pomodoro

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Work,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Work => "Work",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Run {
    /// Not started (or reset): the widget is an empty stripe.
    #[default]
    Idle,
    Running {
        ends: i64,
    },
    /// Paused, or waiting to start the next phase (when phases don't start by themselves).
    Paused {
        left: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pomodoro {
    /// Minutes.
    pub work: u32,
    pub short_break: u32,
    pub long_break: u32,
    /// A long break after this many work sessions.
    pub long_every: u32,
    /// The next phase starts by itself when one ends.
    pub auto_start: bool,
    pub phase: Phase,
    pub run: Run,
    /// Work sessions finished in this cycle.
    pub done: u32,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Pomodoro {
            work: 25,
            short_break: 5,
            long_break: 15,
            long_every: 4,
            auto_start: false,
            phase: Phase::Work,
            run: Run::Idle,
            done: 0,
        }
    }
}

impl Pomodoro {
    pub fn duration(&self, p: Phase) -> i64 {
        let m = match p {
            Phase::Work => self.work,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
        };
        m.max(1) as i64 * 60
    }

    /// Seconds left in this phase (None when idle).
    pub fn remaining(&self, now: i64) -> Option<i64> {
        match self.run {
            Run::Idle => None,
            Run::Running { ends } => Some((ends - now).max(0)),
            Run::Paused { left } => Some(left),
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run, Run::Running { .. })
    }

    /// Start, or resume when paused.
    pub fn start(&mut self, now: i64) {
        self.run = match self.run {
            Run::Idle => Run::Running {
                ends: now + self.duration(self.phase),
            },
            Run::Paused { left } => Run::Running { ends: now + left },
            r => r,
        };
    }

    pub fn pause(&mut self, now: i64) {
        if let Run::Running { ends } = self.run {
            self.run = Run::Paused {
                left: (ends - now).max(0),
            };
        }
    }

    pub fn toggle(&mut self, now: i64) {
        if self.is_running() {
            self.pause(now)
        } else {
            self.start(now)
        }
    }

    pub fn reset(&mut self) {
        self.phase = Phase::Work;
        self.run = Run::Idle;
        self.done = 0;
    }

    /// End this phase now and move on to the next one (running if this one was running
    /// or phases start by themselves).
    pub fn skip(&mut self, now: i64) {
        let running = self.is_running() || self.auto_start;
        self.advance();
        let d = self.duration(self.phase);
        self.run = if running {
            Run::Running { ends: now + d }
        } else {
            Run::Paused { left: d }
        };
    }

    fn advance(&mut self) {
        self.phase = match self.phase {
            Phase::Work => {
                self.done += 1;
                if self.done >= self.long_every.max(1) {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            }
            Phase::LongBreak => {
                self.done = 0;
                Phase::Work
            }
            Phase::ShortBreak => Phase::Work,
        };
    }

    /// The phase that just ended (if one did), after moving on to the next.
    pub fn tick(&mut self, now: i64) -> Option<Phase> {
        let Run::Running { ends } = self.run else {
            return None;
        };
        if now < ends {
            return None;
        }
        let ended = self.phase;
        self.advance();
        let d = self.duration(self.phase);
        self.run = if self.auto_start {
            // Keep the schedule; if farsub was closed past it, start from now.
            let next = ends + d;
            Run::Running {
                ends: if next > now { next } else { now + d },
            }
        } else {
            Run::Paused { left: d }
        };
        Some(ended)
    }

    /// "Work 2 of 4" style position in the cycle.
    pub fn position(&self) -> String {
        match self.phase {
            Phase::Work => format!("Work {} of {}", self.done + 1, self.long_every.max(1)),
            p => p.label().to_string(),
        }
    }
}

// ------------------------------------------------------------------ alarm

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Alarm {
    /// "HH:MM", local time ("" = never set).
    pub time: String,
    pub label: String,
    /// Ring every day at that time (else once).
    pub repeat: bool,
    /// When it rings next (Unix seconds); None = not set.
    pub at: Option<i64>,
}

/// "18:30" / "8:05" → (18, 30).
pub fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// The next time the local clock reads h:m, strictly after `now`.
pub fn next_occurrence(h: u32, m: u32, now: i64) -> i64 {
    use chrono::{Duration, Local, TimeZone};
    let Some(local_now) = Local.timestamp_opt(now, 0).single() else {
        return now + 60;
    };
    let mut day = local_now.date_naive();
    for _ in 0..3 {
        if let Some(naive) = day.and_hms_opt(h, m, 0) {
            // A time skipped by a DST change rings an hour later.
            let t = Local
                .from_local_datetime(&naive)
                .earliest()
                .or_else(|| Local.from_local_datetime(&(naive + Duration::hours(1))).earliest());
            if let Some(t) = t
                && t.timestamp() > now
            {
                return t.timestamp();
            }
        }
        day = day.succ_opt().unwrap_or(day);
    }
    now + 24 * 3600
}

impl Alarm {
    /// Set it for its time (next occurrence). False if the time doesn't parse.
    pub fn arm(&mut self, now: i64) -> bool {
        match parse_hm(&self.time) {
            Some((h, m)) => {
                self.time = format!("{h:02}:{m:02}");
                self.at = Some(next_occurrence(h, m, now));
                true
            }
            None => false,
        }
    }

    pub fn disarm(&mut self) {
        self.at = None;
    }

    /// Rang (and whether it was missed while farsub was closed): repeat rearms it for
    /// the next day, else it's cleared (its time stays for next time).
    pub fn tick(&mut self, now: i64) -> Option<bool> {
        let at = self.at?;
        if now < at {
            return None;
        }
        let missed = now - at > 90;
        self.at = None;
        if self.repeat {
            self.arm(now);
        }
        Some(missed)
    }
}

// ------------------------------------------------------------------ clock

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub hour24: bool,
    pub seconds: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Clock {
            hour24: true,
            seconds: false,
        }
    }
}

impl Clock {
    pub fn format(&self, now: i64) -> String {
        use chrono::{Local, TimeZone};
        let Some(t) = Local.timestamp_opt(now, 0).single() else {
            return String::new();
        };
        let f = match (self.hour24, self.seconds) {
            (true, false) => "%H:%M",
            (true, true) => "%H:%M:%S",
            (false, false) => "%-I:%M %p",
            (false, true) => "%-I:%M:%S %p",
        };
        t.format(f).to_string()
    }
}

/// 754 → "12:34"; an hour or more → "1:02:34".
pub fn countdown(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

// ------------------------------------------------------------------ state + alerts

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
struct Saved {
    clock: Clock,
    pomodoro: Pomodoro,
    alarm: Alarm,
}

/// A notification due at a time: (Unix seconds, title, body).
type Deadline = (i64, String, String);

pub struct Timers {
    pub clock: Clock,
    pub pomodoro: Pomodoro,
    pub alarm: Alarm,
    /// The widgets flashing after an alert, until clicked: (pomodoro, alarm).
    pub flash: (bool, bool),
    file: Option<PathBuf>,
    deadlines: Arc<Mutex<Vec<Deadline>>>,
}

impl Timers {
    /// Load from `<data>/timers.toml` (defaults if missing) and start the alert thread;
    /// `wake` repaints the GUI after an alert.
    pub fn load(data_dir: Option<&Path>, wake: impl Fn() + Send + 'static) -> Self {
        let file = data_dir.map(|d| d.join("timers.toml"));
        let saved: Saved = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        let deadlines: Arc<Mutex<Vec<Deadline>>> = Arc::default();
        let d = deadlines.clone();
        let _ = std::thread::Builder::new()
            .name("timers".into())
            .spawn(move || alert_thread(d, wake));
        let mut t = Timers {
            clock: saved.clock,
            pomodoro: saved.pomodoro,
            alarm: saved.alarm,
            flash: (false, false),
            file,
            deadlines,
        };
        t.sync_deadlines();
        t
    }

    /// Phase ends and alarms that are due: move on and flash. (The alert thread sends
    /// the notification and sound for the same deadlines.) True if anything changed.
    pub fn tick(&mut self, now: i64) -> bool {
        let mut changed = false;
        if self.pomodoro.tick(now).is_some() {
            self.flash.0 = true;
            changed = true;
        }
        if self.alarm.tick(now).is_some() {
            self.flash.1 = true;
            changed = true;
        }
        if changed {
            self.changed();
        }
        changed
    }

    /// Save, and tell the alert thread the new deadlines. Call after every change.
    pub fn changed(&mut self) {
        self.sync_deadlines();
        if let Some(f) = &self.file {
            let s = Saved {
                clock: self.clock.clone(),
                pomodoro: self.pomodoro.clone(),
                alarm: self.alarm.clone(),
            };
            if let Ok(text) = toml::to_string(&s) {
                if let Some(dir) = f.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(f, text);
            }
        }
    }

    fn sync_deadlines(&mut self) {
        let now = now();
        let mut v: Vec<Deadline> = vec![];
        if let Run::Running { ends } = self.pomodoro.run {
            let p = &self.pomodoro;
            let next = match p.phase {
                Phase::Work if p.done + 1 >= p.long_every.max(1) => "a long break",
                Phase::Work => "a short break",
                _ => "back to work",
            };
            v.push((
                ends,
                format!("{} is over", p.phase.label()),
                format!(
                    "Time for {next}{}",
                    if p.auto_start {
                        " (started)"
                    } else {
                        ": click the pomodoro to start it"
                    }
                ),
            ));
        }
        if let Some(at) = self.alarm.at {
            let late = now - at > 90;
            v.push((
                at,
                format!("Alarm {}", self.alarm.time),
                match (self.alarm.label.trim(), late) {
                    ("", false) => "farsub alarm".into(),
                    ("", true) => "Rang while farsub was closed".into(),
                    (l, false) => l.to_string(),
                    (l, true) => format!("{l} (rang while farsub was closed)"),
                },
            ));
        }
        let mut d = self.deadlines.lock().unwrap();
        // Keep the ones already due that the thread hasn't sent yet (the GUI may move on
        // before the thread wakes).
        d.retain(|x| x.0 <= now);
        for x in v {
            if x.0 > now || !d.iter().any(|y| y.0 == x.0) {
                d.push(x);
            }
        }
    }

    /// Seconds until the widgets need redrawing (the next second or minute boundary).
    pub fn redraw_in(&self, now_ms: i64) -> std::time::Duration {
        let per_second = self.clock.seconds || self.pomodoro.is_running();
        let flashing = self.flash.0 || self.flash.1;
        let ms = if flashing {
            500 - now_ms % 500
        } else if per_second {
            1000 - now_ms % 1000
        } else {
            60_000 - now_ms % 60_000
        };
        std::time::Duration::from_millis(ms.max(20) as u64)
    }
}

fn alert_thread(d: Arc<Mutex<Vec<Deadline>>>, wake: impl Fn()) {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let now = now();
        let due: Vec<Deadline> = {
            let Ok(mut v) = d.lock() else { return };
            let due = v.iter().filter(|x| x.0 <= now).cloned().collect();
            v.retain(|x| x.0 > now);
            due
        };
        if due.is_empty() {
            continue;
        }
        for (_, title, body) in &due {
            alert(title, body);
        }
        wake();
    }
}

#[cfg(test)]
pub static FIRED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A system notification and a sound, best effort with what each OS ships (nothing to
/// install).
fn alert(title: &str, body: &str) {
    crate::log::info("timers", format!("{title}: {body}"));
    #[cfg(test)]
    {
        FIRED.lock().unwrap().push(format!("{title}: {body}"));
    }
    #[cfg(not(test))]
    {
        notify(title, body);
        sound();
    }
}

#[cfg(not(test))]
fn spawn(cmd: &str, args: &[&str]) -> bool {
    use std::process::{Command, Stdio};
    let mut c = Command::new(cmd);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    match c.spawn() {
        Ok(mut child) => {
            // Reap it so it doesn't linger as a zombie.
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}

#[cfg(not(test))]
fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"farsub\" subtitle \"{}\"",
            q(body),
            q(title)
        );
        spawn("osascript", &["-e", &script]);
    }
    #[cfg(windows)]
    {
        let q = |s: &str| s.replace('\'', "''").replace(['<', '>', '&'], " ");
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $x = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $t = $x.GetElementsByTagName('text'); \
             $t.Item(0).AppendChild($x.CreateTextNode('{}')) > $null; \
             $t.Item(1).AppendChild($x.CreateTextNode('{}')) > $null; \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
            q(title),
            q(body)
        );
        spawn(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script],
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if !spawn("notify-send", &["-a", "farsub", title, body]) {
            spawn(
                "gdbus",
                &[
                    "call",
                    "--session",
                    "--dest=org.freedesktop.Notifications",
                    "--object-path=/org/freedesktop/Notifications",
                    "--method=org.freedesktop.Notifications.Notify",
                    "farsub",
                    "0",
                    "",
                    title,
                    body,
                    "[]",
                    "{}",
                    "-1",
                ],
            );
        }
    }
}

#[cfg(not(test))]
fn sound() {
    #[cfg(target_os = "macos")]
    {
        spawn("afplay", &["/System/Library/Sounds/Glass.aiff"]);
    }
    #[cfg(windows)]
    {
        spawn(
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                "(New-Object Media.SoundPlayer \"$env:WINDIR\\Media\\Alarm01.wav\").PlaySync()",
            ],
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let f = "/usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga";
        if !(spawn("canberra-gtk-play", &["-i", "alarm-clock-elapsed"])
            || std::path::Path::new(f).exists() && spawn("paplay", &[f]))
        {
            // Last resort: the terminal bell is all there is.
            print!("\x07");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pomodoro_cycles_work_and_breaks() {
        let mut p = Pomodoro {
            work: 25,
            short_break: 5,
            long_break: 15,
            long_every: 2,
            ..Default::default()
        };
        assert_eq!(p.remaining(0), None);
        p.start(1000);
        assert_eq!(p.remaining(1000 + 60), Some(24 * 60));
        // Pause keeps what's left; resume continues from there.
        p.pause(1000 + 60);
        assert_eq!(p.remaining(99_999), Some(24 * 60));
        p.start(5000);
        assert_eq!(p.tick(5000 + 24 * 60 - 1), None);
        assert_eq!(p.tick(5000 + 24 * 60), Some(Phase::Work));
        // Without auto start the break waits, full length.
        assert_eq!(p.phase, Phase::ShortBreak);
        assert_eq!(p.run, Run::Paused { left: 5 * 60 });
        p.skip(0); // break skipped, back to work (not running: it wasn't)
        assert_eq!((p.phase, p.run), (Phase::Work, Run::Paused { left: 25 * 60 }));
        // The second work session ends in a long break; with auto start it runs at once.
        p.auto_start = true;
        p.start(0);
        assert_eq!(p.tick(25 * 60), Some(Phase::Work));
        assert_eq!(p.phase, Phase::LongBreak);
        assert_eq!(p.run, Run::Running { ends: 25 * 60 + 15 * 60 });
        // Closed for hours: one alert, and the next phase starts from now.
        assert_eq!(p.tick(100_000), Some(Phase::LongBreak));
        assert_eq!((p.phase, p.done), (Phase::Work, 0));
        assert_eq!(p.run, Run::Running { ends: 100_000 + 25 * 60 });
        p.reset();
        assert_eq!((p.phase, p.run, p.done), (Phase::Work, Run::Idle, 0));
    }

    #[test]
    fn alarm_rings_once_or_daily() {
        assert_eq!(parse_hm("8:05"), Some((8, 5)));
        assert_eq!(parse_hm("24:00"), None);
        assert_eq!(parse_hm("noon"), None);
        let now = now();
        let mut a = Alarm {
            time: "7:30".into(),
            ..Default::default()
        };
        assert!(a.arm(now));
        assert_eq!(a.time, "07:30");
        let at = a.at.unwrap();
        assert!(at > now && at <= now + 25 * 3600);
        assert_eq!(a.tick(at - 1), None);
        assert_eq!(a.tick(at), Some(false));
        assert_eq!(a.at, None);
        // Daily: rearmed for the next day; missed while closed is reported as such.
        a.repeat = true;
        a.arm(now);
        let at = a.at.unwrap();
        assert_eq!(a.tick(at + 3600), Some(true));
        assert!(a.at.unwrap() > at + 3600);
        let mut bad = Alarm {
            time: "25:00".into(),
            ..Default::default()
        };
        assert!(!bad.arm(now));
    }

    #[test]
    fn formats_and_persistence() {
        assert_eq!(countdown(754), "12:34");
        assert_eq!(countdown(3754), "1:02:34");
        let c = Clock::default();
        let s = c.format(now());
        assert_eq!(s.len(), 5);
        assert_eq!(&s[2..3], ":");
        let dir = std::env::temp_dir().join(format!("farsub-timers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut t = Timers::load(Some(&dir), || {});
        t.pomodoro.work = 50;
        t.pomodoro.start(now());
        t.alarm.time = "06:00".into();
        t.alarm.arm(now());
        t.changed();
        let back = Timers::load(Some(&dir), || {});
        assert_eq!(back.pomodoro, t.pomodoro);
        assert_eq!(back.alarm, t.alarm);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn due_deadlines_notify_even_if_the_gui_moved_on() {
        let dir = std::env::temp_dir().join(format!("farsub-timers-d-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut t = Timers::load(Some(&dir), || {});
        t.pomodoro.work = 1;
        t.pomodoro.start(now() - 61);
        t.changed();
        // The GUI handles the phase end first; the thread still sends its alert.
        assert!(t.tick(now()));
        assert!(t.flash.0);
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3)
            && !FIRED.lock().unwrap().iter().any(|m| m.starts_with("Work is over"))
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(FIRED.lock().unwrap().iter().any(|m| m.starts_with("Work is over")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
