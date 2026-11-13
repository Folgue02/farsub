//! farsub — a Sublime Text-style code editor with a keyboard-driven sidebar and LSP support.

mod app;
mod commands;
mod config;
mod editor;
mod fileops;
mod gui;
mod highlight;
mod installer;
mod keymap;
mod keys;
mod lsp;
mod menu;
#[cfg(target_os = "macos")]
mod native_menu;
mod packages;
mod palette;
#[cfg(test)]
mod tests;
mod tree;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let target = arg
        .map(|a| a.canonicalize().unwrap_or(a))
        .unwrap_or(cwd.clone());
    // A file argument opens its project (git root / language root) in the sidebar.
    let root = if target.is_dir() {
        target.clone()
    } else {
        let parent = target
            .parent()
            .map(|p| p.to_path_buf())
            .filter(|p| p.is_dir())
            .unwrap_or(cwd);
        lsp::find_root(&parent, &[])
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_title("farsub"),
        ..Default::default()
    };
    eframe::run_native(
        "farsub",
        options,
        Box::new(move |cc| {
            let mut gui = gui::Gui::new(cc, root, config::Config::load());
            if target.is_file() {
                gui.app.open_file(&target, true);
            } else if !target.exists() {
                gui.app.open_new_path(target.clone());
            }
            Ok(Box::new(gui))
        }),
    )
}
