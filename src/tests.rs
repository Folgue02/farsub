//! End-to-end tests: drive the App with key events, render to a TestBackend, talk to clangd.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::app::{App, AppEvent, Modal, View};
use crate::commands::Command;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("farsub-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn key(app: &mut App, code: KeyCode, m: KeyModifiers) {
    app.on_event(Event::Key(KeyEvent::new(code, m)));
}

fn typ(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn screen(app: &mut App) -> String {
    let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
    t.draw(|f| crate::ui::draw(f, app)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

fn pump(
    app: &mut App,
    rx: &mpsc::Receiver<AppEvent>,
    secs: u64,
    done: impl Fn(&App) -> bool,
) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if done(app) {
            return true;
        }
        if let Ok(ev) = rx.recv_timeout(Duration::from_millis(50)) {
            app.on_app_event(ev);
        }
        app.sync_lsp();
    }
    done(app)
}

fn setup(name: &str, files: &[(&str, &str)]) -> (App, mpsc::Receiver<AppEvent>, PathBuf) {
    let d = tmpdir(name);
    for (f, c) in files {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
    let (tx, rx) = mpsc::channel();
    (App::new(d.clone(), tx), rx, d)
}

#[test]
fn panels_render_and_navigate() {
    let (mut app, _rx, d) = setup("panels", &[("a.txt", "hi"), ("sub/b.txt", "yo")]);
    let s = screen(&mut app);
    assert!(
        s.contains("a.txt") && s.contains("sub") && s.contains("<DIR>"),
        "{s}"
    );
    assert!(s.contains("Quit") && s.contains("Copy"), "{s}");
    // Enter "sub" (entries: .., sub, a.txt).
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.panels[0].dir, d.join("sub"));
    // Back up with Backspace, selection returns to "sub".
    key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.panels[0].dir, d);
    assert_eq!(app.panels[0].current().unwrap().name, "sub");
    // F7 mkdir.
    key(&mut app, KeyCode::F(7), KeyModifiers::NONE);
    typ(&mut app, "newdir");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(d.join("newdir").is_dir());
    // F5 copy a.txt into sub.
    let i = app.panels[0]
        .entries
        .iter()
        .position(|e| e.name == "a.txt")
        .unwrap();
    app.panels[0].sel = i;
    app.panels[1].chdir(d.join("sub"), false);
    key(&mut app, KeyCode::F(5), KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(d.join("sub/a.txt").is_file());
    // F8 delete with confirm.
    key(&mut app, KeyCode::F(8), KeyModifiers::NONE);
    assert!(matches!(app.modal, Some(Modal::Confirm(_))));
    key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    assert!(!d.join("a.txt").exists());
}

#[test]
fn goto_anything_opens_file_and_edits() {
    let (mut app, rx, d) = setup(
        "palette",
        &[
            (
                "src/main.rs",
                "fn main() {\n    hello();\n}\n\nfn hello() {}\n",
            ),
            ("README.md", "# hi\n"),
        ],
    );
    std::fs::create_dir_all(d.join(".git")).unwrap();
    key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    assert!(pump(
        &mut app,
        &rx,
        5,
        |a| matches!(&a.modal, Some(Modal::Palette(p)) if !p.items.is_empty())
    ));
    typ(&mut app, "mainrs");
    let s = screen(&mut app);
    assert!(
        s.contains("Goto Anything") && s.contains("src/main.rs"),
        "{s}"
    );
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.view, View::Editor);
    let s = screen(&mut app);
    assert!(s.contains("fn main()") && s.contains("Rust"), "{s}");

    // Ctrl+R symbol palette (heuristic fallback without rust-analyzer) jumps to `hello`.
    key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    typ(&mut app, "hello");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().cursor.line, 4);

    // Command palette runs "Duplicate Line".
    key(
        &mut app,
        KeyCode::Char('p'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    typ(&mut app, "dupl");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().lines[5], "fn hello() {}");

    // Cmd+Z (SUPER) undoes it, Cmd+S saves.
    key(&mut app, KeyCode::Char('z'), KeyModifiers::SUPER);
    key(&mut app, KeyCode::Char('s'), KeyModifiers::SUPER);
    assert_eq!(
        std::fs::read_to_string(d.join("src/main.rs")).unwrap(),
        "fn main() {\n    hello();\n}\n\nfn hello() {}\n"
    );

    // Typing + comment toggle + save.
    key(&mut app, KeyCode::Char('/'), KeyModifiers::CONTROL);
    app.run(Command::Save);
    assert!(
        std::fs::read_to_string(d.join("src/main.rs"))
            .unwrap()
            .contains("// fn hello() {}")
    );

    // ":2" goto line inside palette.
    key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    typ(&mut app, ":2");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().cursor.line, 1);

    // Ctrl+O toggles back to the panels.
    key(&mut app, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(app.view, View::Panels);
}

#[test]
fn find_prompt_is_incremental() {
    let (mut app, _rx, d) = setup("find", &[("x.txt", "alpha\nbeta\ngamma beta\n")]);
    app.open_file(&d.join("x.txt"), true);
    key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
    typ(&mut app, "beta");
    assert_eq!(app.buf().unwrap().cursor.line, 1);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut app, KeyCode::F(3), KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().selection().unwrap().0.line, 2);
}

fn clangd_available() -> bool {
    std::process::Command::new("clangd")
        .arg("--version")
        .output()
        .is_ok()
}

#[test]
fn lsp_clangd_diagnostics_symbols_completion_definition() {
    if !clangd_available() {
        eprintln!("clangd not found; skipping");
        return;
    }
    let src = "struct Point { int x; int y; };\n\nint add(int a, int b) { return a + b; }\n\nint main() {\n    struct Point p;\n    p.;\n    int r = add(1, 2);\n    return r + undefined_thing;\n}\n";
    let (mut app, rx, d) = setup("clangd", &[("main.c", src)]);
    let path: &Path = &d.join("main.c");
    app.open_file(path, true);
    assert!(
        app.buf().unwrap().lsp_client.is_some(),
        "clangd should start"
    );

    // Diagnostics for the undefined identifier.
    assert!(
        pump(&mut app, &rx, 20, |a| a
            .diagnostics
            .get(path)
            .is_some_and(|d| d.iter().any(|x| x.severity == 1))),
        "no diagnostics"
    );
    let s = screen(&mut app);
    assert!(s.contains("clangd") && s.contains("E:"), "{s}");

    // Document symbols via Ctrl+R.
    key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(pump(
        &mut app,
        &rx,
        10,
        |a| matches!(&a.modal, Some(Modal::Palette(p)) if !p.loading
        && p.items.iter().any(|i| i.label.trim() == "add"))
    ));
    typ(&mut app, "add");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().cursor.line, 2);

    // Member completion after "p." on line 7.
    app.buffers[app.cur].goto(crate::editor::Pos::new(6, 6));
    app.run(Command::Completion);
    assert!(
        pump(&mut app, &rx, 10, |a| a.completion.as_ref().is_some_and(
            |c| c.items.iter().any(|i| i.label.contains('x'))
        )),
        "no completion"
    );
    let s = screen(&mut app);
    assert!(s.contains("field"), "{s}");
    typ(&mut app, "y");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().lines[6], "    p.y;");

    // Goto definition of `add` on line 8.
    app.buffers[app.cur].goto(crate::editor::Pos::new(7, 12));
    app.run(Command::GotoDefinition);
    let ok = pump(&mut app, &rx, 10, |a| a.buf().unwrap().cursor.line == 2);
    assert!(
        ok,
        "definition jump: cursor={:?} status={:?} modal={}",
        app.buf().unwrap().cursor,
        app.status,
        app.modal.is_some()
    );

    // Hover.
    app.buffers[app.cur].goto(crate::editor::Pos::new(2, 5));
    app.run(Command::Hover);
    assert!(pump(&mut app, &rx, 10, |a| a.hover.is_some()), "hover");
    app.lsp.shutdown();
}

/// Timing harness: `cargo test --release bench_ -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_hot_paths() {
    use crate::palette::{Action, Item, Mode, Palette};
    // 1. Palette filtering over 200k paths, typed one char at a time.
    let items: Vec<Item> = (0..200_000)
        .map(|i| Item {
            label: format!("crates/module_{}/src/sub_{}/file_{i}.rs", i % 97, i % 13),
            detail: String::new(),
            action: Action::None,
        })
        .collect();
    let mut p = Palette::new(Mode::Files, "");
    p.set_items(items);
    let t = Instant::now();
    for c in "mod5file12".chars() {
        p.query.push(c);
        p.refilter();
    }
    println!("palette: 10 keystrokes over 200k items: {:?}", t.elapsed());

    // 2. Highlighting a 20k-line Rust file + rendering frames.
    let src: String = (0..2000)
        .map(|i| format!("/// doc {i}\npub fn f{i}(x: &str) -> Result<u32, String> {{\n    let v = x.len() as u32 * {i};\n    if v > 10 {{ Ok(v) }} else {{ Err(format!(\"{{}}\", v)) }}\n}}\n\nstruct S{i} {{ a: i32 }}\nimpl S{i} {{ fn g(&self) {{}} }}\n// end\n\n"))
        .collect();
    let (mut app, _rx, d) = setup("bench", &[("big.rs", &src)]);
    app.open_file(&d.join("big.rs"), true);
    let t = Instant::now();
    screen(&mut app);
    println!("first frame (top of file): {:?}", t.elapsed());
    app.buffers[app.cur].goto(crate::editor::Pos::new(19_990, 0));
    let t = Instant::now();
    screen(&mut app);
    println!("jump to end, first frame: {:?}", t.elapsed());
    let mut frames = 1;
    while app.hl_pending {
        screen(&mut app);
        frames += 1;
    }
    println!(
        "jump to end, fully highlighted after {frames} frames: {:?}",
        t.elapsed()
    );
    let t = Instant::now();
    for _ in 0..50 {
        screen(&mut app);
    }
    println!("steady frame: {:?}", t.elapsed() / 50);
    // Typing at the top invalidates highlighting for the whole file.
    app.buffers[app.cur].goto(crate::editor::Pos::new(0, 0));
    let t = Instant::now();
    for _ in 0..20 {
        typ(&mut app, "x");
        screen(&mut app);
    }
    println!("keystroke+frame near top: {:?}", t.elapsed() / 20);
    let t = Instant::now();
    for _ in 0..20 {
        let _ = app.buffers[app.cur].text();
    }
    println!(
        "full-text join (LSP full sync payload, {} bytes): {:?}",
        src.len(),
        t.elapsed() / 20
    );
}

#[test]
fn lsp_incremental_sync_matches_buffer() {
    if !clangd_available() {
        return;
    }
    let (mut app, rx, d) = setup("incr", &[("m.c", "int main() {\n    return 0;\n}\n")]);
    let path = d.join("m.c");
    app.open_file(&path, true);
    let cid = app.buf().unwrap().lsp_client.unwrap();
    assert!(pump(&mut app, &rx, 10, |a| a.lsp.clients[&cid].ready));
    assert_eq!(
        app.lsp.clients[&cid].sync_kind, 2,
        "clangd supports incremental sync"
    );
    // Several kinds of edits, including multi-line and undo, then ask clangd for an error
    // that only exists if its copy of the document matches ours.
    let b = &mut app.buffers[app.cur];
    b.goto(crate::editor::Pos::new(1, 4));
    b.insert_text("int é = 1;\n    ");
    b.delete_line();
    b.insert_text("int zz = nope;\n    ");
    b.duplicate();
    b.undo();
    app.sync_lsp();
    let lines = &app.buf().unwrap().lines;
    let line = lines.iter().position(|l| l.contains("nope")).unwrap();
    let col = lines[line].chars().count() - "nope;".len();
    let want = crate::editor::Pos::new(line, col);
    assert!(
        pump(&mut app, &rx, 10, |a| a.diagnostics.get(&path).is_some_and(
            |d| d
                .iter()
                .any(|x| x.message.contains("nope") && x.start == want)
        )),
        "diagnostics should point at line 2: {:?} / {:?}",
        app.diagnostics.get(&path),
        app.buf().unwrap().lines
    );
    app.lsp.shutdown();
}

#[test]
fn hint_bar_clock_and_sidebar() {
    let (mut app, _rx, d) = setup(
        "sidebar",
        &[
            ("src/main.rs", "fn main() {}\n"),
            ("src/lib.rs", "pub fn x() {}\n"),
            ("notes.txt", "hi\n"),
        ],
    );
    std::fs::create_dir_all(d.join(".git")).unwrap();
    let s = screen(&mut app);
    let last = s.lines().last().unwrap();
    assert!(
        last.contains("Delete") && last.contains("Find file") && last.contains("Editor"),
        "{last}"
    );

    app.open_file(&d.join("src/main.rs"), true);
    let s = screen(&mut app);
    let first = s.lines().next().unwrap();
    let clock = chrono::Local::now().format("%H:%M").to_string();
    assert!(
        first.contains("main.rs") && first.trim_end().ends_with(&clock),
        "{first}"
    );
    let last = s.lines().last().unwrap();
    assert!(
        last.contains("Files") && last.contains("Save") && last.contains("Tree"),
        "{last}"
    );

    // Ctrl+B: show + focus the tree with the current file revealed.
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert!(app.tree_visible && app.focus_tree);
    assert_eq!(
        app.tree.selected_path(),
        Some(d.join("src/main.rs").as_path())
    );
    let s = screen(&mut app);
    assert!(
        s.contains("▾ FARSUB-TEST-SIDEBAR")
            && s.contains("▾ src")
            && s.contains("lib.rs")
            && s.contains("notes.txt"),
        "{s}"
    );
    assert!(s.lines().last().unwrap().contains("Preview"));

    // Up to lib.rs, Enter opens it and focuses the editor.
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buf().unwrap().name(), "lib.rs");
    assert!(!app.focus_tree && app.tree_visible);
    // Typing goes to the editor, not the tree.
    typ(&mut app, "z");
    assert!(app.buf().unwrap().lines[0].starts_with('z'));

    // Left on a file jumps to its folder; Left again collapses it.
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(app.tree.selected().unwrap().name, "src");
    key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(app.tree.nodes.len(), 2);
    // Type-to-jump.
    typ(&mut app, "n");
    assert_eq!(app.tree.selected().unwrap().name, "notes.txt");
    // Ctrl+B while focused hides it.
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert!(!app.tree_visible);

    // Palette shows its own hints.
    key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    let s = screen(&mut app);
    assert!(s.lines().last().unwrap().contains("Symbols"), "{s}");
}
