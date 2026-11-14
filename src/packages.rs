//! farsub packages: folders under `<config>/Packages/` that add data — `themes/*.toml`,
//! `queries/<grammar>/highlights.scm` (highlight overrides) and `icons.toml` (icon
//! packages). A package can be installed with `git clone` from a repository URL.

use std::path::{Path, PathBuf};

use crate::app::{AppEvent, EventTx};

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

/// Install a package from a folder or an archive (`.zip`, `.tar.gz`, `.tgz`, `.tar`) on
/// this computer into `dir` (Packages/). An installed package with the same name is replaced
/// (the old one goes to the Trash). Returns the package name.
pub fn install_from_path(dir: &Path, path: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(".from-file");
    let _ = std::fs::remove_dir_all(&tmp);
    let result = (|| {
        let (src, name) = if path.is_dir() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .ok_or("that folder has no name")?;
            (path.to_path_buf(), name)
        } else {
            let top = crate::grammar_install::unpack(path, &tmp)?;
            // Name: the archive's single top folder, else the archive name.
            let name = if top.file_name().is_some_and(|n| n != "unpacked") {
                top.file_name().unwrap().to_string_lossy().into_owned()
            } else {
                let f = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                [".tar.gz", ".tgz", ".zip", ".tar"]
                    .iter()
                    .find_map(|e| f.strip_suffix(e))
                    .unwrap_or(&f)
                    .to_string()
            };
            (top, name)
        };
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(format!("bad package name `{name}`"));
        }
        let dest = dir.join(&name);
        if dest.exists() {
            crate::fileops::trash(std::slice::from_ref(&dest))?;
        }
        crate::fileops::copy_recursive(&src, &dest).map_err(|e| e.to_string())?;
        Ok(name)
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
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

    #[test]
    fn lists_packages_and_names_urls() {
        let d = std::env::temp_dir().join(format!("farsub-pkgs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("My Theme/themes")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        std::fs::create_dir_all(d.join("Icons")).unwrap();
        assert_eq!(list(&d), ["Icons", "My Theme"]);
        assert_eq!(
            name_from_url("https://github.com/someone/farsub-dracula.git").as_deref(),
            Some("farsub-dracula")
        );
        assert_eq!(
            name_from_url("git@github.com:a/My-Pkg").as_deref(),
            Some("My-Pkg")
        );
    }

    #[test]
    fn installs_packages_from_folders_and_zips() {
        use std::io::Write;
        let d = std::env::temp_dir().join(format!("farsub-pkgfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let pkgs = d.join("Packages");
        // A folder.
        let src = d.join("My Theme");
        std::fs::create_dir_all(src.join("themes")).unwrap();
        std::fs::write(src.join("themes/t.toml"), "name = \"T\"\n").unwrap();
        assert_eq!(install_from_path(&pkgs, &src).unwrap(), "My Theme");
        assert!(pkgs.join("My Theme/themes/t.toml").is_file());
        // A zip with one top folder.
        let zpath = d.join("cool-icons.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zpath).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("cool-icons/icons.toml", o).unwrap();
            z.write_all(b"[[icons]]\nextensions = [\"x\"]\nlabel = \"x\"\ncolor = \"#FFFFFF\"\n")
                .unwrap();
            z.finish().unwrap();
        }
        assert_eq!(install_from_path(&pkgs, &zpath).unwrap(), "cool-icons");
        assert!(pkgs.join("cool-icons/icons.toml").is_file());
        // Installing again replaces it.
        assert_eq!(install_from_path(&pkgs, &zpath).unwrap(), "cool-icons");
        assert_eq!(list(&pkgs), ["My Theme", "cool-icons"]);
        // Not an archive.
        let txt = d.join("notes.txt");
        std::fs::write(&txt, "x").unwrap();
        assert!(install_from_path(&pkgs, &txt).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
