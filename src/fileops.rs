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

/// Save `bytes` to `path` without ever leaving it half-written: write a temporary file next
/// to it, flush it to disk, then rename it over the original (keeping its permissions).
/// Falls back to writing in place where a rename would break something: a symlink (the link
/// would be replaced) or a file with several hard links (they'd stop sharing it), or when
/// the folder isn't writable. A failed write (disk full) leaves the original as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let meta = fs::symlink_metadata(path).ok();
    let in_place = meta.as_ref().is_some_and(|m| {
        if m.file_type().is_symlink() {
            return true;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            m.nlink() > 1
        }
        #[cfg(not(unix))]
        false
    });
    if in_place {
        return fs::write(path, bytes);
    }
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.farsub-{}.tmp", std::process::id()));
    let mut f = match fs::File::create(&tmp) {
        Ok(f) => f,
        // The folder isn't writable (the file may still be): write in place.
        Err(_) => return fs::write(path, bytes),
    };
    let mut write = || -> io::Result<()> {
        f.write_all(bytes)?;
        if let Some(m) = &meta {
            f.set_permissions(m.permissions())?;
        }
        f.sync_all()
    };
    // Closed before the rename (Windows can't rename an open file).
    let written = write().and_then(|()| {
        drop(f);
        fs::rename(&tmp, path)
    });
    if written.is_err() {
        // Disk full or similar: the original is untouched.
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// What a file looked like on disk (to notice it changing under an open buffer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskStamp {
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

impl DiskStamp {
    pub fn of(path: &Path) -> Option<DiskStamp> {
        let m = fs::metadata(path).ok()?;
        Some(DiskStamp {
            modified: m.modified().ok(),
            len: m.len(),
        })
    }
}

#[cfg(test)]
mod atomic_tests {
    use super::*;

    #[test]
    fn atomic_writes_keep_permissions_links_and_leave_no_temp_files() {
        let d = std::env::temp_dir().join(format!("farsub-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let f = d.join("a.sh");
        fs::write(&f, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&f, fs::Permissions::from_mode(0o751)).unwrap();
        }
        write_atomic(&f, b"new").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&f).unwrap().permissions().mode() & 0o777,
                0o751
            );
            // A symlink stays a symlink; the file it points to gets the text.
            let link = d.join("link");
            std::os::unix::fs::symlink(&f, &link).unwrap();
            write_atomic(&link, b"via link").unwrap();
            assert!(
                fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(fs::read_to_string(&f).unwrap(), "via link");
            // Hard links keep sharing the file.
            let hard = d.join("hard");
            fs::hard_link(&f, &hard).unwrap();
            write_atomic(&f, b"shared").unwrap();
            assert_eq!(fs::read_to_string(&hard).unwrap(), "shared");
        }
        // A new file works too, and no temporary files are left behind.
        write_atomic(&d.join("fresh.txt"), b"hi").unwrap();
        let names: Vec<String> = fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|n| n.ends_with(".tmp")), "{names:?}");
        let _ = fs::remove_dir_all(&d);
    }
}
