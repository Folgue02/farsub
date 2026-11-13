//! Far-Manager style twin file panels and file operations.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_link: bool,
    pub exec: bool,
    pub size: u64,
    pub mtime: Option<SystemTime>,
}

pub struct Panel {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub sel: usize,
    pub scroll: usize,
    pub marked: HashSet<PathBuf>,
    pub error: Option<String>,
}

impl Panel {
    pub fn new(dir: PathBuf, show_hidden: bool) -> Self {
        let mut p = Panel {
            dir,
            entries: vec![],
            sel: 0,
            scroll: 0,
            marked: HashSet::new(),
            error: None,
        };
        p.reload(show_hidden);
        p
    }

    pub fn reload(&mut self, show_hidden: bool) {
        let keep = self.current().map(|e| e.name.clone());
        self.load(show_hidden);
        if let Some(name) = keep {
            self.select_name(&name);
        }
    }

    fn load(&mut self, show_hidden: bool) {
        self.entries.clear();
        self.error = None;
        if let Some(parent) = self.dir.parent() {
            self.entries.push(Entry {
                name: "..".into(),
                path: parent.to_path_buf(),
                is_dir: true,
                is_link: false,
                exec: false,
                size: 0,
                mtime: None,
            });
        }
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let mut items: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if !show_hidden && name.starts_with('.') {
                    return None;
                }
                let path = e.path();
                let lmeta = e.metadata().ok();
                let is_link = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
                let meta = if is_link {
                    fs::metadata(&path).ok().or(lmeta)
                } else {
                    lmeta
                };
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                #[cfg(unix)]
                let exec = {
                    use std::os::unix::fs::PermissionsExt;
                    !is_dir
                        && meta
                            .as_ref()
                            .is_some_and(|m| m.permissions().mode() & 0o111 != 0)
                };
                #[cfg(not(unix))]
                let exec = false;
                Some(Entry {
                    name,
                    path,
                    is_dir,
                    is_link,
                    exec,
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    mtime: meta.and_then(|m| m.modified().ok()),
                })
            })
            .collect();
        items.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.entries.extend(items);
        self.marked.retain(|p| p.exists());
        self.sel = self.sel.min(self.entries.len().saturating_sub(1));
    }

    pub fn select_name(&mut self, name: &str) {
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.sel = i;
        }
    }

    pub fn chdir(&mut self, dir: PathBuf, show_hidden: bool) {
        let from = self
            .dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned());
        let going_up = self.dir.parent() == Some(dir.as_path());
        self.dir = dir;
        self.sel = 0;
        self.scroll = 0;
        self.marked.clear();
        self.load(show_hidden);
        if going_up && let Some(n) = from {
            self.select_name(&n);
        }
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.sel)
    }

    /// Marked files, or the file under the cursor.
    pub fn targets(&self) -> Vec<PathBuf> {
        if !self.marked.is_empty() {
            let mut v: Vec<PathBuf> = self.marked.iter().cloned().collect();
            v.sort();
            return v;
        }
        match self.current() {
            Some(e) if e.name != ".." => vec![e.path.clone()],
            _ => vec![],
        }
    }

    pub fn toggle_mark(&mut self) {
        if let Some(e) = self.current() {
            if e.name == ".." {
                return;
            }
            let p = e.path.clone();
            if !self.marked.remove(&p) {
                self.marked.insert(p);
            }
        }
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let n = self.entries.len() as isize;
        self.sel = (self.sel as isize + delta).clamp(0, n - 1) as usize;
    }
}

pub fn human_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n}")
    } else {
        format!("{v:.1}{}", U[i])
    }
}

pub fn fmt_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
        None => String::new(),
    }
}

/// Resolve a copy/move destination: into a directory, or to an explicit new name.
pub fn dest_for(src: &Path, dest: &Path, multiple: bool) -> PathBuf {
    if dest.is_dir() || multiple {
        dest.join(src.file_name().unwrap_or_default())
    } else {
        dest.to_path_buf()
    }
}

pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.starts_with(src) && src.is_dir() {
        return Err(io::Error::other("cannot copy a directory into itself"));
    }
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_recursive(&e.path(), &dst.join(e.file_name()))?;
        }
    } else {
        fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_recursive(src, dst)?;
            delete_path(src)
        }
    }
}

pub fn delete_path(p: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(p)?;
    if meta.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}
