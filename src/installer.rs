//! A small mason.nvim: installs language servers into farsub's own data folder
//! (`<data>/servers/<name>/`) from npm, pip (a private venv), cargo, go, or a script, so
//! nothing touches the system. Output is streamed back as log lines.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::app::{AppEvent, EventTx};
use crate::config::{InstallSpec, ServerConfig};

pub enum InstallEvent {
    Log(String),
    Done(Result<PathBuf, String>),
}

const MARKER: &str = ".farsub-installed";

pub fn server_dir(data: &Path, name: &str) -> PathBuf {
    data.join("servers").join(name)
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Path of the server's executable if the manager installed it.
pub fn installed_bin(data: &Path, s: &ServerConfig) -> Option<PathBuf> {
    let marker = server_dir(data, &s.name).join(MARKER);
    let bin = PathBuf::from(std::fs::read_to_string(marker).ok()?.trim());
    bin.is_file().then_some(bin)
}

/// The executable to run: a managed install first, then PATH.
pub fn resolve(data: Option<&Path>, s: &ServerConfig) -> Option<PathBuf> {
    data.and_then(|d| installed_bin(d, s))
        .or_else(|| crate::lsp::find_server(&s.command))
}

pub fn uninstall(data: &Path, name: &str) -> std::io::Result<()> {
    let dir = server_dir(data, name);
    if dir.exists() {
        std::fs::remove_dir_all(dir)
    } else {
        Ok(())
    }
}

/// Install in the background; events arrive as `AppEvent::Install(name, ..)`.
pub fn install(data: PathBuf, s: ServerConfig, tx: EventTx) {
    std::thread::spawn(move || {
        let name = s.name.clone();
        let log = |m: String| {
            tx.send(AppEvent::Install(name.clone(), InstallEvent::Log(m)));
        };
        let res = run_install(&data, &s, &log);
        if let Err(e) = &res {
            log(format!("error: {e}"));
        }
        tx.send(AppEvent::Install(name.clone(), InstallEvent::Done(res)));
    });
}

fn run_install(data: &Path, s: &ServerConfig, log: &dyn Fn(String)) -> Result<PathBuf, String> {
    let Some(spec) = &s.install else {
        return Err(format!(
            "{} has no [install] section; install `{}` yourself",
            s.name, s.command
        ));
    };
    let dir = server_dir(data, &s.name);
    // Keep the previous install until the new one succeeds (installs happen in place
    // because Python venvs can't be moved after creation).
    let backup = dir.with_extension("previous");
    let _ = std::fs::remove_dir_all(&backup);
    if dir.exists() {
        std::fs::rename(&dir, &backup).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bin_name = spec.bin.clone().unwrap_or_else(|| s.command.clone());
    let rel_bin = match install_into(&dir, spec, &bin_name, log) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            if backup.exists() {
                let _ = std::fs::rename(&backup, &dir);
            }
            return Err(e);
        }
    };
    let _ = std::fs::remove_dir_all(&backup);
    let bin = dir.join(rel_bin);
    if !bin.is_file() {
        return Err(format!("install finished but {} is missing", bin.display()));
    }
    std::fs::write(dir.join(MARKER), bin.to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())?;
    log(format!("installed {}", bin.display()));
    Ok(bin)
}

/// Returns the executable's path relative to the install folder.
fn install_into(
    dir: &Path,
    spec: &InstallSpec,
    bin: &str,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let need = |tool: &str, hint: &str| {
        crate::lsp::find_server(tool).ok_or_else(|| format!("`{tool}` not found — {hint}"))
    };
    if !spec.npm.is_empty() {
        let npm = need(
            if cfg!(windows) { "npm.cmd" } else { "npm" },
            "install Node.js",
        )?;
        let mut c = Command::new(npm);
        c.arg("install").arg("--prefix").arg(dir);
        c.args(["--no-fund", "--no-audit", "--loglevel=error", "--no-save"])
            .args(&spec.npm);
        run(c, dir, log)?;
        let b = if cfg!(windows) {
            format!("{bin}.cmd")
        } else {
            bin.to_string()
        };
        return Ok(Path::new("node_modules").join(".bin").join(b));
    }
    if !spec.pip.is_empty() {
        let py = crate::lsp::find_server("python3")
            .or_else(|| crate::lsp::find_server("python"))
            .ok_or("`python3` not found — install Python 3")?;
        let mut c = Command::new(py);
        c.args(["-m", "venv"]).arg(dir.join("venv"));
        run(c, dir, log)?;
        let scripts = if cfg!(windows) { "Scripts" } else { "bin" };
        let vpy = dir.join("venv").join(scripts).join(exe("python"));
        let mut c = Command::new(vpy);
        c.args([
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--quiet",
        ])
        .args(&spec.pip);
        run(c, dir, log)?;
        return Ok(Path::new("venv").join(scripts).join(exe(bin)));
    }
    if let Some(krate) = &spec.cargo {
        let cargo = need("cargo", "install Rust (rustup.rs)")?;
        let mut c = Command::new(cargo);
        c.args(["install", "--root"]).arg(dir).arg(krate);
        run(c, dir, log)?;
        return Ok(Path::new("bin").join(exe(bin)));
    }
    if let Some(module) = &spec.go {
        let go = need("go", "install Go (go.dev)")?;
        let mut c = Command::new(go);
        c.args(["install", module]).env("GOBIN", dir.join("bin"));
        run(c, dir, log)?;
        return Ok(Path::new("bin").join(exe(bin)));
    }
    if let Some(script) = &spec.script {
        let (sh, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        let mut c = Command::new(sh);
        c.arg(flag).arg(script).env("FARSUB_INSTALL_DIR", dir);
        run(c, dir, log)?;
        return Ok(PathBuf::from(bin));
    }
    Err("the [install] section names no source (npm, pip, cargo, go or script)".into())
}

fn run(mut c: Command, cwd: &Path, log: &dyn Fn(String)) -> Result<(), String> {
    log(format!(
        "$ {} {}",
        c.get_program().to_string_lossy(),
        c.get_args()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    ));
    let mut child = c
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let err = child.stderr.take().unwrap();
    let (etx, erx) = std::sync::mpsc::channel();
    let t = std::thread::spawn(move || {
        for l in BufReader::new(err).lines().map_while(Result::ok) {
            let _ = etx.send(l);
        }
    });
    for l in BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok)
    {
        log(l);
        while let Ok(e) = erx.try_recv() {
            log(e);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = t.join();
    for e in erx.try_iter() {
        log(e);
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed ({status})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait_done(rx: &mpsc::Receiver<AppEvent>) -> (Vec<String>, Result<PathBuf, String>) {
        let mut log = vec![];
        loop {
            match rx
                .recv_timeout(std::time::Duration::from_secs(120))
                .expect("install timed out")
            {
                AppEvent::Install(_, InstallEvent::Log(l)) => log.push(l),
                AppEvent::Install(_, InstallEvent::Done(r)) => return (log, r),
                _ => {}
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn script_install_resolve_uninstall() {
        let data = std::env::temp_dir().join(format!("farsub-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let s = ServerConfig {
            name: "fake_ls".into(),
            command: "fake-ls".into(),
            install: Some(InstallSpec {
                script: Some(
                    "echo building; printf '#!/bin/sh\\necho hi\\n' > fake-ls; chmod +x fake-ls"
                        .into(),
                ),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        install(data.clone(), s.clone(), EventTx::new(tx));
        let (log, res) = wait_done(&rx);
        let bin = res.unwrap_or_else(|e| panic!("{e}\n{log:?}"));
        assert!(log.iter().any(|l| l == "building"), "{log:?}");
        assert_eq!(resolve(Some(&data), &s), Some(bin));
        // A failing install keeps the previous one.
        let mut bad = s.clone();
        bad.install.as_mut().unwrap().script = Some("exit 3".into());
        let (tx, rx) = mpsc::channel();
        install(data.clone(), bad, EventTx::new(tx));
        assert!(wait_done(&rx).1.is_err());
        assert!(installed_bin(&data, &s).is_some());
        uninstall(&data, "fake_ls").unwrap();
        assert!(installed_bin(&data, &s).is_none());
    }

    /// Real npm install of bash-language-server (network). Run with --ignored.
    #[test]
    #[ignore]
    fn npm_install_bashls() {
        let data = std::env::temp_dir().join(format!("farsub-npm-{}", std::process::id()));
        let c = crate::config::Config::builtin();
        let s = c.server("bashls").unwrap().clone();
        let (tx, rx) = mpsc::channel();
        install(data.clone(), s.clone(), EventTx::new(tx));
        let (log, res) = wait_done(&rx);
        let bin = res.unwrap_or_else(|e| panic!("{e}\n{log:?}"));
        assert!(
            bin.ends_with("node_modules/.bin/bash-language-server"),
            "{bin:?}"
        );
        let out = Command::new(&bin).arg("--version").output().unwrap();
        assert!(out.status.success());
        let _ = std::fs::remove_dir_all(&data);
    }

    /// Real pip install of python-lsp-server into a private venv (network). Run with --ignored.
    #[test]
    #[ignore]
    fn pip_install_pylsp() {
        let data = std::env::temp_dir().join(format!("farsub-pip-{}", std::process::id()));
        let c = crate::config::Config::builtin();
        let s = c.server("pylsp").unwrap().clone();
        let (tx, rx) = mpsc::channel();
        install(data.clone(), s.clone(), EventTx::new(tx));
        let (log, res) = wait_done(&rx);
        let bin = res.unwrap_or_else(|e| panic!("{e}\n{log:?}"));
        let out = Command::new(&bin).arg("--help").output().unwrap();
        assert!(out.status.success(), "{bin:?}");
        let _ = std::fs::remove_dir_all(&data);
    }
}
