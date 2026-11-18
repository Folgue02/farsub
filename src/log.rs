//! The application log shown in the console panel (like Sublime's console). Any thread can
//! write to it: the UI, language-server readers, installers, package loading.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{LazyLock, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Monotonic id (stable while the entry exists; used for console selections).
    pub id: u64,
    pub time: String,
    pub level: Level,
    pub source: String,
    pub text: String,
}

struct Log {
    entries: VecDeque<Entry>,
    /// Increases with every entry (lets the console notice new output).
    seq: u64,
    file: Option<std::fs::File>,
}

const CAP: usize = 5000;

static LOG: LazyLock<Mutex<Log>> = LazyLock::new(|| {
    Mutex::new(Log {
        entries: VecDeque::new(),
        seq: 0,
        file: None,
    })
});
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Repaint the UI when something is logged from another thread.
pub fn set_waker(f: impl Fn() + Send + Sync + 'static) {
    let _ = WAKE.set(Box::new(f));
}

/// Also append everything to a file (started afresh each launch; the last launch's log
/// is kept beside it as `<name>.previous.log`, so a crash's panic message survives the
/// restart).
pub fn set_file(path: &std::path::Path) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if path.exists() {
        let _ = std::fs::rename(path, path.with_extension("previous.log"));
    }
    if let Ok(f) = std::fs::File::create(path) {
        LOG.lock().unwrap().file = Some(f);
    }
}

pub fn push(level: Level, source: &str, text: impl AsRef<str>) {
    let time = chrono::Local::now().format("%H:%M:%S%.3f").to_string();
    {
        let mut log = LOG.lock().unwrap();
        for line in text.as_ref().lines().filter(|l| !l.trim().is_empty()) {
            if let Some(f) = &mut log.file {
                let _ = writeln!(f, "{time} {:<5} [{source}] {line}", level.tag());
            }
            log.seq += 1;
            let id = log.seq;
            log.entries.push_back(Entry {
                id,
                time: time.clone(),
                level,
                source: source.to_string(),
                text: line.to_string(),
            });
            if log.entries.len() > CAP {
                log.entries.pop_front();
            }
        }
    }
    if let Some(w) = WAKE.get() {
        w();
    }
}

pub fn debug(source: &str, text: impl AsRef<str>) {
    push(Level::Debug, source, text)
}
pub fn info(source: &str, text: impl AsRef<str>) {
    push(Level::Info, source, text)
}
pub fn warn(source: &str, text: impl AsRef<str>) {
    push(Level::Warn, source, text)
}
pub fn error(source: &str, text: impl AsRef<str>) {
    push(Level::Error, source, text)
}

#[allow(dead_code)]
pub fn seq() -> u64 {
    LOG.lock().unwrap().seq
}

pub fn clear() {
    LOG.lock().unwrap().entries.clear();
}

/// Entries matching a filter (`min` level, text/source substring), oldest first.
pub fn entries(min: Level, filter: &str) -> Vec<Entry> {
    let f = filter.to_lowercase();
    LOG.lock()
        .unwrap()
        .entries
        .iter()
        .filter(|e| e.level >= min)
        .filter(|e| {
            f.is_empty()
                || e.text.to_lowercase().contains(&f)
                || e.source.to_lowercase().contains(&f)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_filters() {
        let tag = format!("test-{}", std::process::id());
        push(Level::Debug, &tag, "noisy detail");
        push(Level::Warn, &tag, "first\nsecond line\n\n");
        let all: Vec<_> = entries(Level::Debug, &tag);
        assert_eq!(
            all.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["noisy detail", "first", "second line"]
        );
        assert_eq!(
            entries(Level::Info, &tag).len(),
            2,
            "debug hidden at info level"
        );
        assert!(
            entries(Level::Debug, "second line")
                .iter()
                .any(|e| e.source == tag)
        );
    }
}
