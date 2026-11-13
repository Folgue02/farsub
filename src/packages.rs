//! Sublime-style packages: folders under `<config>/Packages/` holding `.sublime-syntax`
//! grammars and `.tmTheme` color schemes. A package can be installed with `git clone`
//! (like Package Control installing from a repository URL).

use std::path::{Path, PathBuf};

use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::app::{AppEvent, EventTx};

pub struct Loaded {
    pub syntaxes: SyntaxSet,
    pub themes: Vec<(String, Theme)>,
    pub errors: Vec<String>,
    pub packages: Vec<String>,
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            out.push(p);
        }
    }
}

pub fn list(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Built-in syntaxes plus everything in `dir`. Slow-ish (relinks every grammar), so it runs
/// on a background thread and is swapped in when ready.
pub fn load(dir: Option<&Path>) -> Loaded {
    let base = two_face::syntax::extra_newlines();
    let mut errors = vec![];
    let mut themes = vec![];
    let Some(dir) = dir.filter(|d| d.is_dir()) else {
        return Loaded {
            syntaxes: base,
            themes,
            errors,
            packages: vec![],
        };
    };
    let mut syn_files = vec![];
    walk(dir, "sublime-syntax", &mut syn_files);
    let syntaxes = if syn_files.is_empty() {
        base
    } else {
        let mut b = base.into_builder();
        for f in &syn_files {
            // Load one by one so a broken grammar doesn't take the others down.
            match std::fs::read_to_string(f)
                .map_err(|e| e.to_string())
                .and_then(|t| {
                    syntect::parsing::SyntaxDefinition::load_from_str(&t, true, None)
                        .map_err(|e| e.to_string())
                }) {
                Ok(def) => b.add(def),
                Err(e) => errors.push(format!("{}: {e}", f.display())),
            }
        }
        b.build()
    };
    let mut theme_files = vec![];
    walk(dir, "tmTheme", &mut theme_files);
    for f in theme_files {
        match ThemeSet::get_theme(&f) {
            Ok(t) => {
                let name = t.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| {
                    f.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
                themes.push((name, t));
            }
            Err(e) => errors.push(format!("{}: {e}", f.display())),
        }
    }
    Loaded {
        syntaxes,
        themes,
        errors,
        packages: list(dir),
    }
}

pub fn load_async(dir: Option<PathBuf>, tx: EventTx) {
    std::thread::spawn(move || {
        let l = load(dir.as_deref());
        tx.send(AppEvent::Packages(Box::new(l)));
    });
}

/// Folder name for a repository URL: ".../Dracula-sublime.git" → "Dracula-sublime".
pub fn name_from_url(url: &str) -> Option<String> {
    let last = url.trim().trim_end_matches('/').rsplit(['/', ':']).next()?;
    let name = last.trim_end_matches(".git");
    (!name.is_empty() && !name.contains("..")).then(|| name.to_string())
}

/// `git clone --depth 1 <url> <dir>/<name>` in the background.
pub fn install_git(dir: PathBuf, url: String, tx: EventTx) {
    std::thread::spawn(move || {
        let res = (|| {
            let name = name_from_url(&url).ok_or("can't tell the package name from that URL")?;
            let dest = dir.join(&name);
            if dest.exists() {
                return Err(format!("{name} is already installed"));
            }
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let git = crate::lsp::find_server("git").ok_or("`git` not found")?;
            let out = std::process::Command::new(git)
                .args(["clone", "--depth", "1", &url])
                .arg(&dest)
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            Ok(name)
        })();
        tx.send(AppEvent::PackageInstalled(res));
    });
}

/// Open a folder in the OS file manager.
pub fn reveal(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNTAX: &str = r#"%YAML 1.2
---
name: Farsub Test Lang
file_extensions: [fstl]
scope: source.fstl
contexts:
  main:
    - match: '\b(let|fn)\b'
      scope: keyword.control.fstl
    - match: '"[^"]*"'
      scope: string.quoted.double.fstl
"#;

    #[test]
    fn loads_user_syntax_and_reports_broken_ones() {
        let d = std::env::temp_dir().join(format!("farsub-pkgs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("MyLang")).unwrap();
        std::fs::create_dir_all(d.join("Broken")).unwrap();
        std::fs::write(d.join("MyLang/MyLang.sublime-syntax"), SYNTAX).unwrap();
        std::fs::write(d.join("Broken/Bad.sublime-syntax"), "not: [valid").unwrap();
        let l = load(Some(&d));
        assert!(l.syntaxes.find_syntax_by_extension("fstl").is_some());
        assert!(
            l.syntaxes.find_syntax_by_extension("rs").is_some(),
            "built-ins kept"
        );
        assert_eq!(l.errors.len(), 1, "{:?}", l.errors);
        assert_eq!(l.packages, ["Broken", "MyLang"]);
        assert_eq!(
            name_from_url("https://github.com/dracula/sublime.git").as_deref(),
            Some("sublime")
        );
        assert_eq!(
            name_from_url("git@github.com:a/My-Pkg").as_deref(),
            Some("My-Pkg")
        );
    }
}
