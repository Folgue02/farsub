//! Running windows find each other, so `farsub FILE` opens the file in the window the
//! user last used instead of starting a new one.
//!
//! Each window (its own process) listens on a localhost port and lists itself in
//! `<data>/windows/<pid>.json` with that port, a secret token and when it was last
//! focused. The command line sends the files to the most recently focused window that
//! answers; a listing whose window doesn't answer is removed.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// What the command line asks a window to open.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Request {
    /// (path, line, column)
    pub files: Vec<(PathBuf, Option<usize>, Option<usize>)>,
    /// Text read from stdin (`farsub -`), opened as a new untitled file.
    pub stdin: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Listing {
    port: u16,
    token: String,
    /// Milliseconds since the epoch when the window was last focused.
    focused: u64,
}

#[derive(Serialize, Deserialize)]
struct Message {
    token: String,
    #[serde(flatten)]
    request: Request,
}

fn dir(data: &Path) -> PathBuf {
    data.join("windows")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A random token (std's hasher keys are random per process and per instance).
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::hash::RandomState::new().build_hasher();
        h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
        h.write_u32(std::process::id());
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// This window's listing; removed when the window closes.
pub struct Instance {
    file: PathBuf,
    port: u16,
    token: String,
}

impl Instance {
    /// Start listening for requests; `wake` is called after each one is queued.
    pub fn listen(
        data: &Path,
        wake: impl Fn() + Send + 'static,
    ) -> Option<(Instance, Receiver<Request>)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        let me = Instance {
            file: dir(data).join(format!("{}-{port}.json", std::process::id())),
            port,
            token: token(),
        };
        std::fs::create_dir_all(dir(data)).ok()?;
        me.focused();
        let (tx, rx) = channel();
        let token = me.token.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                serve(conn, &token, &tx);
                wake();
            }
        });
        Some((me, rx))
    }

    /// The user focused this window: it's now the one files go to.
    pub fn focused(&self) {
        let l = Listing {
            port: self.port,
            token: self.token.clone(),
            focused: now_ms(),
        };
        let tmp = self.file.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&l).unwrap_or_default()).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
            }
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

fn serve(conn: TcpStream, token: &str, tx: &Sender<Request>) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    let mut r = BufReader::new(&conn);
    if r.read_line(&mut line).is_err() {
        return;
    }
    let Ok(m) = serde_json::from_str::<Message>(&line) else {
        return;
    };
    if m.token != token {
        return;
    }
    if tx.send(m.request).is_ok() {
        let _ = (&conn).write_all(b"ok\n");
    }
}

/// Hand the request to the most recently focused window. False if none took it.
pub fn send(data: &Path, request: &Request) -> bool {
    let Ok(entries) = std::fs::read_dir(dir(data)) else {
        return false;
    };
    let mut windows: Vec<(PathBuf, Listing)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let l = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
            Some((p, l))
        })
        .collect();
    windows.sort_by_key(|(_, l)| std::cmp::Reverse(l.focused));
    for (path, l) in windows {
        if try_send(&l, request) {
            return true;
        }
        // Its window is gone (it crashed or was killed).
        let _ = std::fs::remove_file(path);
    }
    false
}

fn try_send(l: &Listing, request: &Request) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, l.port));
    let Ok(mut c) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    let m = Message {
        token: l.token.clone(),
        request: request.clone(),
    };
    let Ok(mut bytes) = serde_json::to_vec(&m) else {
        return false;
    };
    bytes.push(b'\n');
    let _ = c.set_read_timeout(Some(Duration::from_secs(3)));
    if c.write_all(&bytes).is_err() {
        return false;
    }
    // Let the window bring itself to the front (Windows only lets the foreground
    // process hand that over).
    #[cfg(windows)]
    allow_foreground();
    let mut reply = String::new();
    BufReader::new(&c).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

#[cfg(windows)]
fn allow_foreground() {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn AllowSetForegroundWindow(process_id: u32) -> i32;
    }
    const ASFW_ANY: u32 = u32::MAX;
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farsub-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn req(f: &str) -> Request {
        Request {
            files: vec![(PathBuf::from(f), Some(3), None)],
            stdin: None,
        }
    }

    #[test]
    fn files_go_to_the_last_focused_window() {
        let d = tmp("focus");
        assert!(!send(&d, &req("a")), "no windows yet");
        let (a, ra) = Instance::listen(&d, || {}).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        let (b, rb) = Instance::listen(&d, || {}).unwrap();
        // b was focused last.
        assert!(send(&d, &req("one")));
        assert_eq!(rb.recv_timeout(Duration::from_secs(2)).unwrap(), req("one"));
        assert!(ra.try_recv().is_err());
        // Now a is.
        std::thread::sleep(Duration::from_millis(5));
        a.focused();
        assert!(send(&d, &req("two")));
        assert_eq!(ra.recv_timeout(Duration::from_secs(2)).unwrap(), req("two"));
        // b closes: its listing goes; a still gets files.
        drop(b);
        drop(rb);
        assert_eq!(std::fs::read_dir(dir(&d)).unwrap().count(), 1);
        assert!(send(&d, &req("three")));
        assert_eq!(ra.recv_timeout(Duration::from_secs(2)).unwrap(), req("three"));
        drop(a);
        assert!(!send(&d, &req("four")));
    }

    #[test]
    fn stale_listings_and_wrong_tokens() {
        let d = tmp("stale");
        let (a, ra) = Instance::listen(&d, || {}).unwrap();
        // A window that died without cleaning up, focused more recently.
        let dead = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = dead.local_addr().unwrap().port();
        drop(dead);
        let stale = dir(&d).join("1-1.json");
        let l = Listing { port, token: "x".into(), focused: now_ms() + 10_000 };
        std::fs::write(&stale, serde_json::to_vec(&l).unwrap()).unwrap();
        assert!(send(&d, &req("f")));
        assert!(!stale.exists());
        assert_eq!(ra.recv_timeout(Duration::from_secs(2)).unwrap(), req("f"));
        // A request without the token is ignored.
        let bad = Listing { port: a.port, token: "wrong".into(), focused: 0 };
        assert!(!try_send(&bad, &req("g")));
        assert!(ra.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
