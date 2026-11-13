//! File operations used by the sidebar.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() && dst.starts_with(src) {
        return Err(io::Error::other("cannot copy a folder into itself"));
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
    if src.is_dir() && dst.starts_with(src) {
        return Err(io::Error::other("cannot move a folder into itself"));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        // Cross-device: copy then remove.
        Err(_) => {
            copy_recursive(src, dst)?;
            if src.is_dir() {
                fs::remove_dir_all(src)
            } else {
                fs::remove_file(src)
            }
        }
    }
}

/// Move to the system trash (recoverable).
pub fn trash(paths: &[PathBuf]) -> Result<(), String> {
    trash::delete_all(paths).map_err(|e| e.to_string())
}

/// `dir/name`, or `dir/name copy.ext`, `dir/name copy 2.ext`, … if that already exists.
pub fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 1.. {
        let suffix = if n == 1 {
            " copy".to_string()
        } else {
            format!(" copy {n}")
        };
        let cand = dir.join(format!("{stem}{suffix}{ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_and_moves() {
        let d = std::env::temp_dir().join(format!("farsub-fileops-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("a")).unwrap();
        fs::write(d.join("x.rs"), "1").unwrap();
        assert_eq!(unique_dest(&d, "x.rs"), d.join("x copy.rs"));
        fs::write(d.join("x copy.rs"), "").unwrap();
        assert_eq!(unique_dest(&d, "x.rs"), d.join("x copy 2.rs"));
        assert_eq!(unique_dest(&d, "new.rs"), d.join("new.rs"));
        move_path(&d.join("x.rs"), &d.join("a/deep/y.rs")).unwrap();
        assert_eq!(fs::read_to_string(d.join("a/deep/y.rs")).unwrap(), "1");
        assert!(move_path(&d.join("a"), &d.join("a/inner")).is_err());
        copy_recursive(&d.join("a"), &d.join("b")).unwrap();
        assert!(d.join("b/deep/y.rs").is_file());
    }
}
