//! Standard folders per platform: the user's home, farsub's config folder and its data
//! folder. Windows has no $HOME by default, so everything goes through here.
//!
//! | | config | data |
//! |---|---|---|
//! | Windows | `%APPDATA%\farsub` | `%LOCALAPPDATA%\farsub` |
//! | macOS, Linux | `$XDG_CONFIG_HOME/farsub` or `~/.config/farsub` | `$XDG_DATA_HOME/farsub` or `~/.local/share/farsub` |
//!
//! `FARSUB_CONFIG` (the config file) and `FARSUB_DATA` (the data folder) override them.

use std::ffi::OsString;
use std::path::PathBuf;

fn env(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// The user's home folder: $HOME, else (Windows) %USERPROFILE% or %HOMEDRIVE%%HOMEPATH%.
pub fn home_dir() -> Option<PathBuf> {
    home_with(&env)
}

/// farsub's config folder (not created here).
pub fn config_dir() -> Option<PathBuf> {
    config_with(cfg!(windows), &env)
}

/// farsub's data folder: installed servers, grammars, logs, workspaces (not created here).
pub fn data_dir() -> Option<PathBuf> {
    data_with(cfg!(windows), &env)
}

/// `~/rest` (or `~`) with the home folder filled in.
pub fn expand_tilde(s: &str) -> Option<PathBuf> {
    if s == "~" {
        return home_dir();
    }
    let rest = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\"))?;
    Some(home_dir()?.join(rest))
}

type Env<'a> = &'a dyn Fn(&str) -> Option<OsString>;

fn home_with(var: Env) -> Option<PathBuf> {
    if let Some(h) = var("HOME").or_else(|| var("USERPROFILE")) {
        return Some(PathBuf::from(h));
    }
    let (mut d, p) = (var("HOMEDRIVE")?, var("HOMEPATH")?);
    d.push(p);
    Some(PathBuf::from(d))
}

fn config_with(windows: bool, var: Env) -> Option<PathBuf> {
    if windows {
        return var("APPDATA")
            .map(PathBuf::from)
            .or_else(|| home_with(var).map(|h| h.join("AppData").join("Roaming")))
            .map(|b| b.join("farsub"));
    }
    var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_with(var).map(|h| h.join(".config")))
        .map(|b| b.join("farsub"))
}

fn data_with(windows: bool, var: Env) -> Option<PathBuf> {
    if let Some(x) = var("FARSUB_DATA") {
        return Some(PathBuf::from(x));
    }
    if windows {
        return var("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| home_with(var).map(|h| h.join("AppData").join("Local")))
            .map(|b| b.join("farsub"));
    }
    var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_with(var).map(|h| h.join(".local").join("share")))
        .map(|b| b.join("farsub"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.iter().find(|(n, _)| n == k).map(|(_, v)| v.into())
    }

    #[test]
    fn windows_folders_without_home() {
        // A stock Windows session: no HOME, only the Windows variables.
        let w = vars(&[
            ("USERPROFILE", r"C:\Users\ana"),
            ("APPDATA", r"C:\Users\ana\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\ana\AppData\Local"),
        ]);
        assert_eq!(home_with(&w), Some(PathBuf::from(r"C:\Users\ana")));
        assert_eq!(
            config_with(true, &w),
            Some(PathBuf::from(r"C:\Users\ana\AppData\Roaming").join("farsub"))
        );
        assert_eq!(
            data_with(true, &w),
            Some(PathBuf::from(r"C:\Users\ana\AppData\Local").join("farsub"))
        );
        // Only the drive and path: still found.
        let d = vars(&[("HOMEDRIVE", "C:"), ("HOMEPATH", r"\Users\ana")]);
        assert_eq!(home_with(&d), Some(PathBuf::from(r"C:\Users\ana")));
        assert!(config_with(true, &d).is_some());
    }

    #[test]
    fn unix_folders_follow_xdg() {
        let u = vars(&[("HOME", "/home/ana")]);
        assert_eq!(
            config_with(false, &u),
            Some(PathBuf::from("/home/ana/.config/farsub"))
        );
        assert_eq!(
            data_with(false, &u),
            Some(PathBuf::from("/home/ana/.local/share/farsub"))
        );
        let x = vars(&[
            ("HOME", "/home/ana"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("FARSUB_DATA", "/d"),
        ]);
        assert_eq!(config_with(false, &x), Some(PathBuf::from("/cfg/farsub")));
        assert_eq!(data_with(false, &x), Some(PathBuf::from("/d")));
        assert_eq!(home_with(&vars(&[])), None);
    }
}
