//! farsub — a Far Manager style twin-panel file manager with a Sublime-flavoured editor,
//! syntax highlighting and LSP support.

mod app;
mod clipboard;
mod commands;
mod editor;
mod highlight;
mod keys;
mod lsp;
mod palette;
mod panels;
#[cfg(test)]
mod tests;
mod tree;
mod ui;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;

type Term = Terminal<CrosstermBackend<io::Stdout>>;

fn enter_tui(enhanced: bool) -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    if enhanced {
        // Lets kitty/WezTerm/Ghostty/iTerm2 report Cmd (SUPER) and Ctrl+Shift combos.
        execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    Ok(())
}

fn leave_tui(enhanced: bool) -> io::Result<()> {
    if enhanced {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
    execute!(
        io::stdout(),
        DisableBracketedPaste,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    disable_raw_mode()
}

fn run_shell(term: &mut Term, enhanced: bool, cmd: &str, dir: &Path) -> io::Result<()> {
    leave_tui(enhanced)?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    println!("{}> {cmd}", dir.display());
    match std::process::Command::new(&shell)
        .arg("-c")
        .arg(cmd)
        .current_dir(dir)
        .status()
    {
        Ok(s) if !s.success() => println!("\n[exit status: {}]", s.code().unwrap_or(-1)),
        Err(e) => println!("\n[failed to run: {e}]"),
        _ => {}
    }
    print!("\n[Press Enter to return] ");
    io::stdout().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    enter_tui(enhanced)?;
    term.clear()?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let cwd = std::env::current_dir()?;
    let target = arg
        .map(|a| a.canonicalize().unwrap_or(a))
        .unwrap_or(cwd.clone());
    let start_dir = if target.is_dir() {
        target.clone()
    } else {
        target
            .parent()
            .map(|p| p.to_path_buf())
            .filter(|p| p.is_dir())
            .unwrap_or(cwd)
    };

    let (tx, rx) = mpsc::channel();
    let mut app = App::new(start_dir, tx);
    if target.is_file() {
        app.open_file(&target, true);
    } else if !target.exists() {
        // `farsub new.rs` opens an empty buffer that is created on first save.
        app.open_new_path(target.clone());
    }

    enable_raw_mode()?;
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    disable_raw_mode()?;
    enter_tui(enhanced)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = leave_tui(enhanced);
        hook(info);
    }));

    let mut term = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let res = event_loop(&mut term, &mut app, &rx, enhanced);
    app.lsp.shutdown();
    leave_tui(enhanced)?;
    res
}

fn event_loop(
    term: &mut Term,
    app: &mut App,
    rx: &mpsc::Receiver<app::AppEvent>,
    enhanced: bool,
) -> anyhow::Result<()> {
    let mut redraw = true;
    let mut last_draw = Instant::now();
    loop {
        if redraw || last_draw.elapsed() > Duration::from_millis(500) {
            term.draw(|f| ui::draw(f, app))?;
            last_draw = Instant::now();
            redraw = false;
        }
        let wait = if app.hl_pending { 0 } else { 30 };
        if event::poll(Duration::from_millis(wait))? {
            loop {
                app.on_event(event::read()?);
                if app.quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
            redraw = true;
        }
        while let Ok(ev) = rx.try_recv() {
            app.on_app_event(ev);
            redraw = true;
        }
        app.sync_lsp();
        redraw |= app.tick() || app.hl_pending;
        if let Some((cmd, dir)) = app.pending_shell.take() {
            run_shell(term, enhanced, &cmd, &dir)?;
            app.reload_panels();
            redraw = true;
        }
        if app.quit {
            return Ok(());
        }
    }
}
