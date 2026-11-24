//! Built-in WakaTime time tracking (off by default: `wakatime = true` turns it on).
//!
//! Everything WakaTime-specific lives here so it can move to a plugin later. It follows
//! WakaTime's plugin guide (wakatime.com/help/creating-plugin):
//! - Heartbeats are sent by the official `wakatime-cli`, run in the background; FarSub
//!   never talks to WakaTime's API itself.
//! - A heartbeat goes out when the focused file changes, when it's edited, and when it's
//!   saved (`--write`). An edit or focus on the same file sends again only after
//!   2 minutes; a save or another file always sends.
//! - The API key lives in `~/.wakatime.cfg` (`[settings]` `api_key`), shared with every
//!   other WakaTime plugin. wakatime-cli reads it from there, so the key is never on a
//!   command line, in FarSub's log or on screen.
//! - wakatime-cli is found where the official plugins install it
//!   (`~/.wakatime/wakatime-cli-<os>-<arch>`) or on PATH. When it's missing, FarSub says
//!   so; "WakaTime: Install wakatime-cli" downloads the official release.
//!
//! `$WAKATIME_HOME`, when set, replaces the home folder for both, as in wakatime-cli.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::{App, AppEvent, EventTx, Modal, Prompt, PromptKind};
use crate::lineedit::LineEdit;

/// The same file gets another (non-save) heartbeat only after this long.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(120);

const RELEASES: &str = "https://github.com/wakatime/wakatime-cli/releases/latest/download";

/// wakatime-cli's exit code for an API key the server rejected.
const EXIT_AUTH: i32 = 104;
/// Exit codes that only mean "queued offline, sent later" (API unreachable, backing off).
const EXIT_QUEUED: [i32; 2] = [102, 112];

/// Results from background work.
#[derive(Debug)]
pub enum Event {
    /// wakatime-cli was downloaded (its path) or the download failed.
    Installed(Result<PathBuf, String>),
    /// The server rejected the API key.
    #[cfg_attr(test, allow(dead_code))]
    KeyRejected,
}

/// One heartbeat, as passed to wakatime-cli.
#[derive(Clone, Debug, PartialEq)]
pub struct Heartbeat {
    pub file: PathBuf,
    /// Sent because the file was saved.
    pub write: bool,
    /// 1-based line and column of the caret.
    pub line: usize,
    pub column: usize,
    pub lines: usize,
    /// WakaTime's name for the file's language (see `language_name`).
    pub language: Option<String>,
    /// The project folder holding the file: (name, path).
    pub project: Option<(String, PathBuf)>,
    /// The file isn't on disk yet (a new file not saved).
    pub unsaved: bool,
}

impl Heartbeat {
    /// wakatime-cli's arguments. The API key is never one of them: wakatime-cli reads it
    /// from `~/.wakatime.cfg` itself.
    pub fn args(&self) -> Vec<String> {
        let v = env!("CARGO_PKG_VERSION");
        let mut a = vec![
            "--entity".to_string(),
            self.file.to_string_lossy().into_owned(),
            "--plugin".into(),
            format!("FarSub/{v} farsub-wakatime/{v}"),
            "--lineno".into(),
            self.line.to_string(),
            "--cursorpos".into(),
            self.column.to_string(),
            "--lines-in-file".into(),
            self.lines.to_string(),
        ];
        if self.write {
            a.push("--write".into());
        }
        if let Some(l) = &self.language {
            // FarSub's language wins when WakaTime knows it; otherwise wakatime-cli
            // detects one and falls back to FarSub's.
            a.extend(["--language".into(), l.clone()]);
            a.extend(["--alternate-language".into(), l.clone()]);
        }
        if let Some((name, folder)) = &self.project {
            // The guide: never `--project`, so wakatime-cli's own detection (git…) wins.
            a.extend(["--alternate-project".into(), name.clone()]);
            a.extend([
                "--project-folder".into(),
                folder.to_string_lossy().into_owned(),
            ]);
        }
        if self.unsaved {
            a.push("--is-unsaved-entity".into());
        }
        a
    }
}

/// When to send: the guide's rule.
#[derive(Default, Debug)]
pub struct Throttle {
    last_file: Option<PathBuf>,
    last_sent: Option<Instant>,
}

impl Throttle {
    /// A save, another file, or more than 2 minutes since the last heartbeat.
    pub fn should_send(&self, file: &Path, write: bool, now: Instant) -> bool {
        write
            || self.last_file.as_deref() != Some(file)
            || self
                .last_sent
                .is_none_or(|t| now.saturating_duration_since(t) > HEARTBEAT_EVERY)
    }

    pub fn sent(&mut self, file: &Path, now: Instant) {
        self.last_file = Some(file.to_path_buf());
        self.last_sent = Some(now);
    }
}

/// WakaTime's state in a window.
pub struct WakaTime {
    /// Where `.wakatime.cfg` and `.wakatime/` are ($WAKATIME_HOME or the home folder).
    pub home: Option<PathBuf>,
    /// Look for wakatime-cli on PATH too (off in tests).
    pub search_path: bool,
    throttle: Throttle,
    /// The focused buffer (id, version) last frame: a change is a focus change or an edit.
    seen: Option<(usize, i32)>,
    pub installing: bool,
    told_no_key: bool,
    told_no_cli: bool,
    /// Heartbeats "sent" in tests (wakatime-cli's arguments), instead of running it.
    #[cfg(test)]
    pub sent: Vec<Vec<String>>,
}

impl Default for WakaTime {
    fn default() -> Self {
        WakaTime {
            // Tests never read or write the real ~/.wakatime.cfg or run wakatime-cli.
            home: if cfg!(test) { None } else { default_home() },
            search_path: !cfg!(test),
            throttle: Throttle::default(),
            seen: None,
            installing: false,
            told_no_key: false,
            told_no_cli: false,
            #[cfg(test)]
            sent: Vec::new(),
        }
    }
}

fn default_home() -> Option<PathBuf> {
    match std::env::var("WAKATIME_HOME") {
        Ok(h) if !h.trim().is_empty() => {
            crate::paths::expand_tilde(h.trim()).or_else(|| Some(PathBuf::from(h.trim())))
        }
        _ => crate::paths::home_dir(),
    }
}

impl WakaTime {
    pub fn cfg_path(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|h| h.join(".wakatime.cfg"))
    }

    fn resources(&self) -> Option<PathBuf> {
        self.home.as_ref().map(|h| h.join(".wakatime"))
    }

    /// An API key (or a command that prints one) is set in `~/.wakatime.cfg`.
    pub fn has_key(&self) -> bool {
        self.cfg_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|t| cfg_has_key(&t))
    }

    /// Save the key in `~/.wakatime.cfg`, keeping everything else in it.
    pub fn save_key(&self, key: &str) -> Result<PathBuf, String> {
        let path = self.cfg_path().ok_or("no home folder")?;
        let old = match std::fs::read_to_string(&path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        let new = cfg_with_key(old.as_deref().unwrap_or(""), key);
        std::fs::write(&path, new).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if old.is_none() {
            // A new file holding a secret: readable by its owner only.
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(path)
    }

    /// wakatime-cli: where the official plugins install it, else on PATH.
    pub fn find_cli(&self) -> Option<PathBuf> {
        if let Some(dir) = self.resources() {
            for name in [cli_name(), exe("wakatime-cli")] {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        if self.search_path {
            return crate::lsp::which(&exe("wakatime-cli"));
        }
        None
    }

    /// The focused buffer this frame: a new buffer or a new version is a heartbeat
    /// candidate. Returns true when it is one.
    pub fn changed(&mut self, buf: Option<(usize, i32)>) -> bool {
        let new = buf.is_some() && buf != self.seen;
        self.seen = buf;
        new
    }
}

/// `wakatime-cli-<os>-<arch>`, the official release's name for this platform.
pub fn cli_name() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        o => o,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        a => a,
    };
    exe(&format!("wakatime-cli-{os}-{arch}"))
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The `[settings]` section's value for `key` in a `.wakatime.cfg`.
fn cfg_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut in_settings = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_settings = l == "[settings]";
        } else if in_settings
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim());
        }
    }
    None
}

pub fn cfg_has_key(text: &str) -> bool {
    ["api_key", "apikey", "api_key_vault_cmd"]
        .iter()
        .any(|k| cfg_value(text, k).is_some_and(|v| !v.is_empty()))
}

/// `text` with `[settings]` `api_key` set to `key` (added, or replacing the old one).
pub fn cfg_with_key(text: &str, key: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let (mut in_settings, mut had_settings, mut done) = (false, false, false);
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if in_settings && !done {
                insert_before_blank(&mut out, format!("api_key = {key}"));
                done = true;
            }
            in_settings = l == "[settings]";
            had_settings |= in_settings;
        } else if in_settings
            && let Some((k, _)) = l.split_once('=')
            && k.trim() == "api_key"
        {
            if !done {
                out.push(format!("api_key = {key}"));
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !had_settings {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push("[settings]".into());
        }
        out.push(format!("api_key = {key}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Add a line at the end of the section, before the blank lines that end it.
fn insert_before_blank(out: &mut Vec<String>, line: String) {
    let at = out
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(0, |i| i + 1);
    out.insert(at, line);
}

/// A WakaTime API key: a UUID, optionally prefixed `waka_` (what the official plugins
/// accept).
pub fn valid_key(key: &str) -> bool {
    let k = key.strip_prefix("waka_").unwrap_or(key);
    let parts: Vec<&str> = k.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    parts.len() == 5
        && parts
            .iter()
            .zip(lens)
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// WakaTime's name for a FarSub language title, where they differ. WakaTime matches
/// names without regard to case.
pub fn language_name(title: &str) -> String {
    match title {
        "Plain Text" => "Text",
        "JavaScript (JSX)" => "JSX",
        "TypeScript (TSX)" => "TSX",
        "JSON with Comments" => "JSON",
        "Protocol Buffers" => "Protocol Buffer",
        "Vim script" => "VimL",
        t => t,
    }
    .to_string()
}

/// Download the official wakatime-cli release for this platform into `~/.wakatime/`.
#[cfg_attr(test, allow(dead_code))]
fn install(dir: PathBuf, tx: EventTx) {
    std::thread::spawn(move || {
        let res = (|| -> Result<PathBuf, String> {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let name = cli_name();
            let stem = name.strip_suffix(".exe").unwrap_or(&name);
            let zip = format!("{stem}.zip");
            let url = format!("{RELEASES}/{zip}");
            crate::log::info("wakatime", format!("downloading {url}"));
            let archive = dir.join(&zip);
            {
                let mut r = crate::installer::http_get(&url)?.into_reader();
                let mut f = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
                std::io::copy(&mut r, &mut f).map_err(|e| format!("download failed: {e}"))?;
            }
            let res = crate::installer::extract(&archive, &dir, &name);
            let _ = std::fs::remove_file(&archive);
            res?;
            let bin = dir.join(&name);
            if !bin.is_file() {
                return Err(format!("{name} not found in {zip}"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755));
            }
            Ok(bin)
        })();
        tx.send(AppEvent::WakaTime(Event::Installed(res)));
    });
}

/// Run wakatime-cli in the background; only its exit code comes back.
#[cfg_attr(test, allow(dead_code))]
fn spawn(cli: PathBuf, args: Vec<String>, tx: EventTx) {
    std::thread::spawn(move || {
        let mut c = std::process::Command::new(&cli);
        c.args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        crate::proc::hidden(&mut c);
        match c.status() {
            Ok(s) => match s.code() {
                Some(0) => {}
                Some(EXIT_AUTH) => {
                    tx.send(AppEvent::WakaTime(Event::KeyRejected));
                }
                Some(n) if EXIT_QUEUED.contains(&n) => crate::log::debug(
                    "wakatime",
                    format!("heartbeat queued offline (exit code {n})"),
                ),
                n => crate::log::warn(
                    "wakatime",
                    format!(
                        "wakatime-cli failed (exit code {}); its log is ~/.wakatime/wakatime.log",
                        n.map_or("?".into(), |n| n.to_string())
                    ),
                ),
            },
            Err(e) => crate::log::warn("wakatime", format!("running {}: {e}", cli.display())),
        }
    });
}

// ========================================================================= the app's side

impl App {
    fn wakatime_on(&self) -> bool {
        self.config.editor.wakatime
    }

    /// Commands that only show (and work) with WakaTime on.
    pub fn wakatime_command(cmd: crate::commands::Command) -> bool {
        use crate::commands::Command::*;
        matches!(cmd, WakaTimeSetApiKey | WakaTimeInstallCli)
    }

    /// Every frame: a focus change or an edit of the focused file is a heartbeat.
    pub fn wakatime_tick(&mut self, now: Instant) {
        if !self.wakatime_on() {
            self.wakatime.changed(None);
            return;
        }
        let cur = self.buf().filter(|b| b.path.is_some()).map(|b| (b.id, b.version));
        if self.wakatime.changed(cur) {
            self.wakatime_heartbeat(false, now);
        }
    }

    /// The focused file was saved.
    pub fn wakatime_saved(&mut self) {
        if self.wakatime_on() {
            self.wakatime_heartbeat(true, Instant::now());
        }
    }

    fn wakatime_heartbeat(&mut self, write: bool, now: Instant) {
        let Some(hb) = self.current_heartbeat(write) else {
            return;
        };
        if !self.wakatime.throttle.should_send(&hb.file, write, now) {
            return;
        }
        if !self.wakatime.has_key() {
            if !std::mem::replace(&mut self.wakatime.told_no_key, true) {
                self.msg("WakaTime needs your API key: WakaTime: Set API Key…");
            }
            return;
        }
        let Some(cli) = self.wakatime.find_cli() else {
            if !self.wakatime.installing
                && !std::mem::replace(&mut self.wakatime.told_no_cli, true)
            {
                self.msg(
                    "WakaTime needs wakatime-cli: WakaTime: Install wakatime-cli downloads it",
                );
            }
            return;
        };
        self.wakatime.throttle.sent(&hb.file, now);
        let args = hb.args();
        crate::log::debug("wakatime", format!("heartbeat {}", args.join(" ")));
        #[cfg(test)]
        {
            let _ = cli;
            self.wakatime.sent.push(args);
        }
        #[cfg(not(test))]
        spawn(cli, args, self.event_tx());
    }

    fn current_heartbeat(&self, write: bool) -> Option<Heartbeat> {
        let b = self.buf()?;
        let file = b.path.clone()?;
        let project = (self.project.folders.iter())
            .filter(|f| file.starts_with(&f.path))
            .max_by_key(|f| f.path.components().count())
            .map(|f| (f.name.clone(), f.path.clone()));
        Some(Heartbeat {
            unsaved: !file.exists(),
            write,
            line: b.cursor.line + 1,
            column: b.cursor.col + 1,
            lines: b.lines.len(),
            language: Some(language_name(&self.language_title(&b.hl.syntax))),
            project,
            file,
        })
    }

    /// "WakaTime: Set API Key…": a password prompt.
    pub fn wakatime_ask_key(&mut self) {
        let title = if self.wakatime.has_key() {
            "WakaTime API key (replaces the one in ~/.wakatime.cfg)"
        } else {
            "WakaTime API key (wakatime.com/api-key; saved in ~/.wakatime.cfg)"
        };
        self.completion = None;
        self.hover = None;
        self.modal = Some(Modal::Prompt(Prompt {
            title: title.into(),
            input: LineEdit::secret(),
            kind: PromptKind::WakaTimeKey,
        }));
    }

    pub fn wakatime_set_key(&mut self, key: &str) {
        if key.is_empty() {
            return;
        }
        if !valid_key(key) {
            return self.msg(
                "That isn't a WakaTime API key (wakatime.com/api-key shows yours); nothing saved",
            );
        }
        match self.wakatime.save_key(key) {
            Ok(p) => {
                self.wakatime.told_no_key = false;
                self.msg(format!("Saved the WakaTime API key in {}", p.display()));
            }
            Err(e) => self.msg(format!("Couldn't save the WakaTime API key: {e}")),
        }
    }

    /// "WakaTime: Install wakatime-cli": download the official release.
    pub fn wakatime_install(&mut self) {
        if self.wakatime.installing {
            return;
        }
        let Some(dir) = self.wakatime.resources() else {
            return self.msg("No home folder to install wakatime-cli into");
        };
        self.wakatime.installing = true;
        self.msg("Downloading wakatime-cli from github.com/wakatime/wakatime-cli…");
        #[cfg(not(test))]
        install(dir, self.event_tx());
        #[cfg(test)]
        let _ = dir;
    }

    pub fn on_wakatime_event(&mut self, ev: Event) {
        match ev {
            Event::Installed(Ok(p)) => {
                self.wakatime.installing = false;
                self.wakatime.told_no_cli = false;
                self.msg(format!("Installed wakatime-cli: {}", p.display()));
            }
            Event::Installed(Err(e)) => {
                self.wakatime.installing = false;
                crate::log::error("wakatime", format!("install failed: {e}"));
                self.msg(format!("wakatime-cli install failed: {e}"));
            }
            Event::KeyRejected => {
                self.msg("WakaTime rejected the API key: WakaTime: Set API Key… to change it")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_follows_the_guide() {
        let t0 = Instant::now();
        let (a, b) = (Path::new("/p/a.rs"), Path::new("/p/b.rs"));
        let mut t = Throttle::default();
        assert!(t.should_send(a, false, t0), "first heartbeat");
        t.sent(a, t0);
        // Same file, within 2 minutes: nothing.
        assert!(!t.should_send(a, false, t0 + Duration::from_secs(30)));
        assert!(!t.should_send(a, false, t0 + HEARTBEAT_EVERY));
        // A save always sends; so does another file.
        assert!(t.should_send(a, true, t0 + Duration::from_secs(1)));
        assert!(t.should_send(b, false, t0 + Duration::from_secs(1)));
        // Same file after more than 2 minutes.
        assert!(t.should_send(a, false, t0 + HEARTBEAT_EVERY + Duration::from_secs(1)));
        // Switching files resets the clock for the new one.
        t.sent(b, t0 + Duration::from_secs(10));
        assert!(t.should_send(a, false, t0 + Duration::from_secs(11)));
        assert!(!t.should_send(b, false, t0 + Duration::from_secs(11)));
    }

    #[test]
    fn heartbeat_args() {
        let hb = Heartbeat {
            file: PathBuf::from("/p/src/main.rs"),
            write: true,
            line: 3,
            column: 7,
            lines: 40,
            language: Some("Rust".into()),
            project: Some(("p".into(), PathBuf::from("/p"))),
            unsaved: false,
        };
        let a = hb.args().join(" ");
        let v = env!("CARGO_PKG_VERSION");
        assert!(a.starts_with("--entity /p/src/main.rs "), "{a}");
        assert!(a.contains(&format!("--plugin FarSub/{v} farsub-wakatime/{v}")), "{a}");
        for f in [
            "--write",
            "--lineno 3",
            "--cursorpos 7",
            "--lines-in-file 40",
            "--language Rust",
            "--alternate-project p",
            "--project-folder /p",
        ] {
            assert!(a.contains(f), "{f} in {a}");
        }
        assert!(!a.contains("--project "), "never --project: {a}");
        assert!(!a.contains("--key"), "the key is never an argument: {a}");
        assert!(!a.contains("--is-unsaved-entity"));
        let hb = Heartbeat {
            write: false,
            unsaved: true,
            ..hb
        };
        let a = hb.args().join(" ");
        assert!(!a.contains("--write") && a.contains("--is-unsaved-entity"), "{a}");
    }

    #[test]
    fn cfg_key_round_trip() {
        // A fake key, split so secret scanners don't flag it.
        let k = concat!("waka", "_", "00000000-0000-0000-0000-000000000000");
        assert!(!cfg_has_key(""));
        let t = cfg_with_key("", k);
        assert_eq!(t, format!("[settings]\napi_key = {k}\n"));
        assert!(cfg_has_key(&t));
        assert_eq!(cfg_value(&t, "api_key"), Some(k));
        // Replaces the old key, keeps everything else in place.
        let old = "[settings]\ndebug = false\napi_key = old\nhidefilenames = true\n\n[git]\nsubmodules_disabled = false\n";
        let t = cfg_with_key(old, k);
        assert_eq!(
            t,
            format!("[settings]\ndebug = false\napi_key = {k}\nhidefilenames = true\n\n[git]\nsubmodules_disabled = false\n")
        );
        // No key yet: added at the end of [settings], before the next section.
        let t = cfg_with_key("[settings]\ndebug = true\n\n[git]\nx = 1\n", k);
        assert_eq!(t, format!("[settings]\ndebug = true\napi_key = {k}\n\n[git]\nx = 1\n"));
        // No [settings] section: one is added.
        let t = cfg_with_key("[git]\nx = 1\n", k);
        assert_eq!(t, format!("[git]\nx = 1\n\n[settings]\napi_key = {k}\n"));
        // An api_key outside [settings] doesn't count.
        assert!(!cfg_has_key("[git]\napi_key = x\n"));
        assert!(cfg_has_key("[settings]\napi_key_vault_cmd = pass waka\n"));
    }

    #[test]
    fn key_format() {
        assert!(valid_key("00000000-0000-0000-0000-000000000000"));
        assert!(valid_key(concat!("waka", "_", "DEADBEEF-0000-0000-0000-000000000000")));
        assert!(!valid_key(""));
        assert!(!valid_key("hunter2"));
        assert!(!valid_key("00000000-0000-0000-0000-00000000000g"));
        assert!(!valid_key(" 00000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn cli_names_match_the_releases() {
        let n = cli_name();
        assert!(n.starts_with("wakatime-cli-"), "{n}");
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert_eq!(n, "wakatime-cli-darwin-arm64");
        }
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(n, "wakatime-cli-linux-amd64");
        }
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert_eq!(n, "wakatime-cli-windows-amd64.exe");
        }
    }
}
