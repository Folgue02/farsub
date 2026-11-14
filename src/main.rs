//! farsub — a Sublime Text-style code editor with a keyboard-driven sidebar, projects,
//! syntax packages and LSP support.

mod app;
mod cli;
mod commands;
mod config;
mod editor;
mod fileops;
mod gui;
mod highlight;
mod installer;
mod keymap;
mod keys;
mod lineedit;
mod log;
mod lsp;
mod menu;
#[cfg(target_os = "macos")]
mod native_menu;
mod packages;
mod palette;
mod project;
mod terminal;
#[cfg(test)]
mod tests;
mod tree;
mod workspace;
mod wrap;

use std::io::Read;
use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let args = cli::Args::parse();
    // Panics end up in the console log (and the log file) as well as on stderr.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error("panic", info.to_string());
        default_hook(info);
    }));
    let mut config = match &args.config {
        Some(p) => config::Config::load_from(Some(p.clone())),
        None => config::Config::load(),
    };
    if let Some(d) = &args.data_dir {
        config.data_dir = Some(d.clone());
    }
    if let Some(code) = cli::run_headless(&args, &config) {
        return ExitCode::from(code as u8);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let recent = project::Recent::load(config.data_dir.as_deref());
    let plan = match cli::plan(
        &args,
        &cwd,
        recent.last.as_ref(),
        config.editor.restore_session,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("farsub: {e}");
            return ExitCode::from(2);
        }
    };
    let mut proj = match &plan.target {
        cli::Target::Project(f) => match project::Project::load(f) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("farsub: {e}");
                return ExitCode::from(2);
            }
        },
        cli::Target::Folders(fs) => project::Project::from_folders(fs.clone()),
    };
    let added = plan
        .add
        .iter()
        .fold(false, |any, f| proj.add_folder(f.clone()) || any);
    let stdin_text = match &args.stdin_file {
        Some(f) => {
            let t = std::fs::read_to_string(f).ok();
            let _ = std::fs::remove_file(f);
            t
        }
        None => plan.stdin.then(|| {
            let mut s = String::new();
            let _ = std::io::stdin().read_to_string(&mut s);
            s
        }),
    };

    // Don't block the shell (like `subl`): relaunch detached and return, unless --wait.
    if !args.wait && std::env::var_os("FARSUB_DETACHED").is_none() {
        return detach(stdin_text.as_deref());
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_title("farsub"),
        ..Default::default()
    };
    let res = eframe::run_native(
        "farsub",
        options,
        Box::new(move |cc| {
            let mut gui = gui::Gui::new(cc, proj, config);
            let app = &mut gui.app;
            if plan.restore {
                app.restore_workspace();
            }
            if added && app.project.file.is_some() {
                // `--add` on a saved project updates its file, like Project › Add Folder.
                if let Some(f) = app.project.file.clone() {
                    let _ = app.project.save_as(&f);
                }
            }
            for f in &plan.files {
                app.open_at(&f.path, f.line, f.col);
            }
            if let Some(t) = &stdin_text {
                app.open_untitled(t);
            }
            Ok(Box::new(gui))
        }),
    );
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("farsub: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Re-run this executable in the background with the same arguments and exit.
fn detach(stdin_text: Option<&str>) -> ExitCode {
    let exe = std::env::current_exe().unwrap_or_else(|_| "farsub".into());
    let mut args: Vec<std::ffi::OsString> = vec![];
    for a in std::env::args_os().skip(1) {
        if a == "-" {
            continue; // replaced by --stdin-file below
        }
        args.push(a);
    }
    if let Some(t) = stdin_text {
        let f = std::env::temp_dir().join(format!("farsub-stdin-{}.txt", std::process::id()));
        if std::fs::write(&f, t).is_ok() {
            args.push("--stdin-file".into());
            args.push(f.into());
        }
    }
    if std::env::var_os("FARSUB_DRY_RUN").is_some() {
        println!(
            "would launch: {} {}",
            exe.display(),
            args.iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        return ExitCode::SUCCESS;
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.args(&args)
        .env("FARSUB_DETACHED", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group: Ctrl+C in the shell or closing the terminal doesn't kill it.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match cmd.spawn() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!(
                "farsub: couldn't start in the background ({e}); use --wait to run in the foreground"
            );
            ExitCode::FAILURE
        }
    }
}
