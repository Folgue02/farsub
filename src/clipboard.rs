//! System clipboard via platform tools (pbcopy/wl-copy/xclip), with an in-process fallback.

use std::io::Write;
use std::process::{Command, Stdio};

fn copy_cmds() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["-ib"]),
        ]
    }
}

fn paste_cmds() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "macos") {
        &[("pbpaste", &[])]
    } else {
        &[
            ("wl-paste", &["-n"]),
            ("xclip", &["-selection", "clipboard", "-o"]),
            ("xsel", &["-ob"]),
        ]
    }
}

pub fn set(text: &str) -> bool {
    for (cmd, args) in copy_cmds() {
        let child = Command::new(cmd)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}

pub fn get() -> Option<String> {
    for (cmd, args) in paste_cmds() {
        if let Ok(out) = Command::new(cmd).args(*args).stderr(Stdio::null()).output()
            && out.status.success()
        {
            return Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    None
}
