//! Runs the real binary for the headless CLI commands.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_farsub"))
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("farsub-clitest-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn version_and_help() {
    let out = bin().arg("--version").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("farsub "));
    let out = bin().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    for want in [
        "--project",
        "--new",
        "--add",
        "--install-server",
        "--check-config",
        "FILE[:LINE[:COL]]",
        "Examples:",
    ] {
        assert!(help.contains(want), "{want} missing from help:\n{help}");
    }
}

#[test]
fn check_config_reports_errors() {
    let d = tmp("check");
    let cfg = d.join("config.toml");
    std::fs::write(&cfg, "[editor]\ntab_size = 2\n").unwrap();
    let out = bin()
        .arg("--config")
        .arg(&cfg)
        .arg("--check-config")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("languages"));

    std::fs::write(
        d.join("keymap.toml"),
        "[[keys]]\nkeys = \"Cmd+K\"\ncommand = \"nope\"\n",
    )
    .unwrap();
    std::fs::write(
        d.join("P.farsub-project"),
        "[[folders]]\npath = \"missing\"\n[settings]\nbogus = 1\n",
    )
    .unwrap();
    let out = bin()
        .arg("--config")
        .arg(&cfg)
        .args(["--check-config", "--project"])
        .arg(d.join("P.farsub-project"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("unknown command \"nope\"")
            && err.contains("doesn't exist")
            && err.contains("bogus"),
        "{err}"
    );
}

#[test]
fn list_and_install_servers() {
    let d = tmp("servers");
    let cfg = d.join("config.toml");
    std::fs::create_dir_all(d.join("servers")).unwrap();
    std::fs::write(
        d.join("servers/fake_ls.toml"),
        "command = \"fake-ls\"\n[install]\nscript = \"printf '#!/bin/sh\\n' > fake-ls; chmod +x fake-ls\"\nbin = \"fake-ls\"\n",
    )
    .unwrap();
    let data = d.join("data");
    let run = |args: &[&str]| {
        bin()
            .arg("--config")
            .arg(&cfg)
            .arg("--data-dir")
            .arg(&data)
            .args(args)
            .output()
            .unwrap()
    };
    let out = run(&["--list-servers"]);
    let list = String::from_utf8_lossy(&out.stdout);
    assert!(
        list.contains("pyright") && list.contains("bashls") && list.contains("fake_ls"),
        "{list}"
    );
    if cfg!(unix) {
        let out = run(&["--install-server", "fake_ls"]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let list = String::from_utf8_lossy(&run(&["--list-servers"]).stdout).to_string();
        let line = list
            .lines()
            .find(|l| l.starts_with("fake_ls"))
            .unwrap()
            .to_string();
        assert!(line.contains("installed"), "{line}");
        assert!(run(&["--uninstall-server", "fake_ls"]).status.success());
    }
    assert_eq!(
        run(&["--install-server", "no_such_server"]).status.code(),
        Some(1)
    );
}

#[test]
fn bad_arguments_fail_cleanly() {
    let out = bin()
        .arg("missing.farsub-project")
        .arg("--data-dir")
        .arg(tmp("bad"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no such project"));
}

#[test]
fn launching_hands_off_to_background_unless_wait() {
    let d = tmp("detach");
    let out = bin()
        .env("FARSUB_DRY_RUN", "1")
        .arg("--data-dir")
        .arg(&d)
        .args(["-n", "."])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.starts_with("would launch:") && s.contains("-n"), "{s}");
    // Bad arguments are still reported in the shell before handing off.
    let out = bin()
        .env("FARSUB_DRY_RUN", "1")
        .arg("--data-dir")
        .arg(&d)
        .arg("nope.farsub-project")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    // stdin is captured before detaching and passed along as a file.
    let mut child = bin()
        .env("FARSUB_DRY_RUN", "1")
        .arg("--data-dir")
        .arg(&d)
        .args(["-n", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"piped text")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let file = s.split("--stdin-file ").nth(1).unwrap().trim().to_string();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "piped text");
    let _ = std::fs::remove_file(file);
}
