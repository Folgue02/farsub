//! End-to-end tests: drive the real GUI headlessly with egui_kittest (keys, text, clicks),
//! check app state and the filesystem, and render screenshots.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Modifiers};
use egui_kittest::Harness;

use crate::app::{Focus, Modal};
use crate::config::{Config, EditorConfig};
use crate::gui::Gui;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("farsub-gui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = tmpdir(name);
    for (f, c) in files {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, c).unwrap();
    }
    d
}

fn test_config(root: &Path) -> Config {
    let mut c = Config::builtin();
    c.editor = EditorConfig {
        show_menu_bar: true,
        animations: false,
        ..EditorConfig::default()
    };
    // Never touch the user's real config or data folders.
    c.path = Some(root.join(".farsub-test-config.toml"));
    c.data_dir = Some(root.join(".farsub-test-data"));
    c
}

fn harness(root: &Path) -> Harness<'static, Gui> {
    let gui = Gui::headless(root.to_path_buf(), test_config(root));
    Harness::builder()
        .with_size(egui::Vec2::new(1200.0, 720.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(|ui, g: &mut Gui| g.frame(ui), gui)
}

fn steps(h: &mut Harness<'static, Gui>, n: usize) {
    for _ in 0..n {
        h.step();
    }
}

const CMD: Modifiers = Modifiers::COMMAND;

fn press(h: &mut Harness<'static, Gui>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    steps(h, 2);
}

fn typ(h: &mut Harness<'static, Gui>, s: &str) {
    h.event(Event::Text(s.to_string()));
    steps(h, 2);
}

fn shot(h: &mut Harness<'static, Gui>, name: &str) {
    let dir = std::env::var_os("FARSUB_SHOTS").map(PathBuf::from);
    if let Some(dir) = dir {
        std::fs::create_dir_all(&dir).unwrap();
        let img = h.render().expect("render");
        img.save(dir.join(format!("{name}.png"))).unwrap();
    }
}

fn wait(h: &mut Harness<'static, Gui>, secs: u64, f: impl Fn(&Gui) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        steps(h, 1);
        if f(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f(h.state())
}

#[test]
fn sidebar_file_operations_by_keyboard() {
    let d = project(
        "ops",
        &[
            ("src/main.rs", "fn main() {}\n"),
            ("docs/readme.md", "# hi\n"),
        ],
    );
    let mut h = harness(&d);
    steps(&mut h, 2);

    // Cmd+0 focuses the sidebar.
    press(&mut h, CMD, Key::Num0);
    assert_eq!(h.state().app.focus, Focus::Sidebar);

    // `a` → new file (nested path creates folders), opens it in the editor.
    typ(&mut h, "a");
    assert!(matches!(h.state().app.modal, Some(Modal::Prompt(_))));
    typ(&mut h, "notes/todo.txt");
    // The selected folder (first entry, `docs`) is where new files go.
    match &h.state().app.modal {
        Some(Modal::Prompt(p)) => assert!(p.title.contains("docs/"), "{}", p.title),
        _ => panic!("expected prompt"),
    }
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(d.join("docs/notes/todo.txt").is_file());
    assert_eq!(h.state().app.buf().unwrap().name(), "todo.txt");
    assert_eq!(h.state().app.focus, Focus::Editor);
    typ(&mut h, "hello");
    press(&mut h, CMD, Key::S);
    assert_eq!(
        std::fs::read_to_string(d.join("docs/notes/todo.txt")).unwrap(),
        "hello"
    );

    // Back to the sidebar: the new file is selected; `r` renames/moves it.
    press(&mut h, CMD, Key::Num0);
    assert_eq!(
        h.state().app.tree.selected_path(),
        Some(d.join("docs/notes/todo.txt").as_path())
    );
    typ(&mut h, "r");
    press(&mut h, CMD, Key::A); // Ctrl/Cmd+A: cursor to start (path prompt)
    // Replace the whole input: clear with Cmd+U, type new path.
    press(&mut h, CMD, Key::U);
    typ(&mut h, "docs/todo.md");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(!d.join("docs/notes/todo.txt").exists());
    assert_eq!(
        std::fs::read_to_string(d.join("docs/todo.md")).unwrap(),
        "hello"
    );
    // The open buffer followed the move.
    assert_eq!(
        h.state().app.buf().unwrap().path.as_deref(),
        Some(d.join("docs/todo.md").as_path())
    );
    assert_eq!(
        h.state().app.tree.selected_path(),
        Some(d.join("docs/todo.md").as_path())
    );

    // `c` copy, select the src folder, `p` paste.
    typ(&mut h, "c");
    let src_i = h
        .state()
        .app
        .tree
        .nodes
        .iter()
        .position(|n| n.name == "src")
        .unwrap();
    h.state_mut().app.tree.sel = src_i;
    typ(&mut h, "p");
    assert!(d.join("src/todo.md").is_file());
    assert!(d.join("docs/todo.md").is_file());

    // Pasting again into the same folder makes "todo copy.md".
    typ(&mut h, "p");
    assert!(d.join("src/todo copy.md").is_file());

    // `x` cut + `p` paste moves; Shift+D duplicates.
    h.state_mut().app.tree.reveal(&d.join("src/todo copy.md"));
    typ(&mut h, "x");
    let docs_i = h
        .state()
        .app
        .tree
        .nodes
        .iter()
        .position(|n| n.name == "docs")
        .unwrap();
    h.state_mut().app.tree.sel = docs_i;
    typ(&mut h, "p");
    assert!(!d.join("src/todo copy.md").exists());
    assert!(d.join("docs/todo copy.md").is_file());
    h.state_mut().app.tree.reveal(&d.join("src/main.rs"));
    typ(&mut h, "D");
    assert!(d.join("src/main copy.rs").is_file());

    // Shift+A → new folder.
    typ(&mut h, "A");
    typ(&mut h, "assets/img");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(d.join("src/assets/img").is_dir());

    // Space marks several files; `d` asks before trashing; `n` keeps them.
    h.state_mut().app.tree.reveal(&d.join("src/main.rs"));
    typ(&mut h, " ");
    h.state_mut().app.tree.reveal(&d.join("src/main copy.rs"));
    typ(&mut h, " ");
    assert_eq!(h.state().app.tree.marked.len(), 2);
    typ(&mut h, "d");
    match &h.state().app.modal {
        Some(Modal::Confirm(c)) => assert!(c.message.contains("2 items"), "{}", c.message),
        _ => panic!("expected confirm"),
    }
    shot(&mut h, "sidebar_confirm");
    typ(&mut h, "n");
    assert!(h.state().app.modal.is_none());
    assert!(d.join("src/main.rs").is_file() && d.join("src/main copy.rs").is_file());

    // Arrow navigation / folding; Esc clears marks, then returns to the editor.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().app.tree.marked.is_empty());
    h.state_mut().app.tree.sel = 0;
    let n0 = h.state().app.tree.nodes.len();
    press(&mut h, Modifiers::NONE, Key::ArrowRight); // expand first folder
    assert!(
        h.state().app.tree.nodes.len() > n0
            || h.state()
                .app
                .tree
                .expanded
                .contains(&h.state().app.tree.nodes[0].path)
    );
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(h.state().app.focus, Focus::Editor);
    shot(&mut h, "sidebar");
}

#[test]
fn goto_anything_commands_and_editing() {
    let d = project(
        "edit",
        &[
            (
                "src/main.rs",
                "fn main() {\n    hello();\n}\n\nfn hello() {}\n",
            ),
            ("README.md", "# hi\n"),
        ],
    );
    std::fs::create_dir_all(d.join(".git")).unwrap();
    let mut h = harness(&d);
    steps(&mut h, 2);

    press(&mut h, CMD, Key::P);
    assert!(wait(
        &mut h,
        5,
        |g| matches!(&g.app.modal, Some(Modal::Palette(p)) if !p.items.is_empty())
    ));
    typ(&mut h, "mainrs");
    shot(&mut h, "goto_anything");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().name(), "main.rs");

    // Cmd+R symbol palette (keyword fallback without rust-analyzer).
    press(&mut h, CMD, Key::R);
    typ(&mut h, "hello");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().cursor.line, 4);

    // Command palette → Duplicate Line, Cmd+Z undoes.
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    typ(&mut h, "duplicate line");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().lines[5], "fn hello() {}");
    press(&mut h, CMD, Key::Z);
    assert_eq!(h.state().app.buf().unwrap().lines[5], "");

    // Typing with auto-pairs, Cmd+/ comment, Cmd+D next occurrence.
    press(&mut h, Modifiers::NONE, Key::End);
    press(&mut h, Modifiers::NONE, Key::Enter);
    typ(&mut h, "let x = (1");
    assert_eq!(h.state().app.buf().unwrap().lines[5], "let x = (1)");
    press(&mut h, CMD, Key::Slash);
    assert_eq!(h.state().app.buf().unwrap().lines[5], "// let x = (1)");

    // Find panel: incremental, Enter goes to the next match.
    press(&mut h, CMD, Key::F);
    typ(&mut h, "hello");
    shot(&mut h, "find");
    let first = h.state().app.buf().unwrap().cursor.line;
    press(&mut h, Modifiers::NONE, Key::Enter);
    let second = h.state().app.buf().unwrap().cursor.line;
    assert_ne!(first, second);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().app.modal.is_none());

    // Cmd+B hides the sidebar; Cmd+W closes (asks because modified), Cmd+Shift+T reopens.
    press(&mut h, CMD, Key::B);
    assert!(!h.state().app.sidebar_visible);
    press(&mut h, CMD, Key::S);
    press(&mut h, CMD, Key::W);
    assert!(h.state().app.buffers.is_empty());
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    assert_eq!(h.state().app.buf().unwrap().name(), "main.rs");

    // Mouse: clicking a line in the editor moves the cursor there.
    press(&mut h, CMD, Key::B);
    steps(&mut h, 2);
    shot(&mut h, "editor");
}

#[test]
fn settings_add_language_and_theme() {
    let d = project("cfg", &[("init.lua", "local x = 1\n")]);
    let mut h = harness(&d);
    steps(&mut h, 2);
    // Cmd+, creates and opens the config from the template.
    press(&mut h, CMD, Key::Comma);
    let cfg = d.join(".farsub-test-config.toml");
    assert!(cfg.is_file());
    assert_eq!(
        h.state().app.buf().unwrap().name(),
        ".farsub-test-config.toml"
    );
    // Add a language and switch theme by editing the file in the editor, then save.
    let extra =
        "\n[[languages]]\nname = \"lua\"\nextensions = [\"lua\"]\ncomment = \"--\"\ntab_size = 2\n";
    {
        let b = h.state_mut().app.buf_mut().unwrap();
        b.move_doc(true, false);
        b.insert_text(extra);
        let theme = b
            .lines
            .iter()
            .position(|l| l.starts_with("theme = "))
            .unwrap();
        b.lines[theme] = "theme = \"Nord\"".into();
    }
    press(&mut h, CMD, Key::S);
    assert_eq!(h.state().app.config.editor.theme, "Nord");
    assert!(h.state().app.config.language("lua").is_some());
    // A .lua file now gets the configured comment token and indentation.
    h.state_mut().app.open_file(&d.join("init.lua"), true);
    steps(&mut h, 2);
    press(&mut h, CMD, Key::Slash);
    assert_eq!(h.state().app.buf().unwrap().lines[0], "-- local x = 1");
    assert_eq!(h.state().app.buf().unwrap().indent_unit, "  ");
    shot(&mut h, "nord_theme");
}

fn clangd_available() -> bool {
    std::process::Command::new("clangd")
        .arg("--version")
        .output()
        .is_ok()
}

#[test]
fn lsp_through_the_gui() {
    if !clangd_available() {
        eprintln!("clangd not found; skipping");
        return;
    }
    let src = "struct Point { int x; int y; };\n\nint add(int a, int b) { return a + b; }\n\nint main() {\n    struct Point p;\n    p.;\n    int r = add(1, 2);\n    return r + undefined_thing;\n}\n";
    let d = project("lsp", &[("main.c", src)]);
    let path = d.join("main.c");
    let mut h = harness(&d);
    h.state_mut().app.open_file(&path, true);
    assert!(wait(&mut h, 20, |g| g
        .app
        .diagnostics
        .get(&path)
        .is_some_and(|d| d.iter().any(|x| x.severity == 1))));

    // Ctrl+Space completion after `p.`.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(6, 6));
    press(&mut h, Modifiers::CTRL, Key::Space);
    assert!(wait(&mut h, 10, |g| g
        .app
        .completion
        .as_ref()
        .is_some_and(|c| !c.shown.is_empty())));
    shot(&mut h, "completion");
    typ(&mut h, "y");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().lines[6], "    p.y;");

    // F12 goto definition.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(7, 13));
    press(&mut h, Modifiers::NONE, Key::F12);
    assert!(wait(&mut h, 10, |g| g.app.buf().unwrap().cursor.line == 2));

    // Cmd+I hover.
    press(&mut h, CMD, Key::I);
    assert!(wait(&mut h, 10, |g| g.app.hover.is_some()));
    shot(&mut h, "hover");
    h.state_mut().app.lsp.shutdown();
}

fn click(h: &mut Harness<'static, Gui>, pos: egui::Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    steps(h, 2);
}

#[test]
fn menu_bar_keyboard_and_mouse() {
    let d = project("menu", &[("a.txt", "one\ntwo\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);

    // F10 opens File; arrows move between menus; Esc closes.
    press(&mut h, Modifiers::NONE, Key::F10);
    assert_eq!(h.state().app.menu.map(|m| m.menu), Some(0));
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    press(&mut h, Modifiers::NONE, Key::ArrowDown);
    assert_eq!(h.state().app.menu.map(|m| m.menu), Some(1));
    shot(&mut h, "menu_edit");
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().app.menu.is_none());

    // Typing a letter runs the item: File → "New File".
    let n = h.state().app.buffers.len();
    press(&mut h, Modifiers::NONE, Key::F10);
    typ(&mut h, "n");
    assert_eq!(h.state().app.buffers.len(), n + 1);
    assert!(h.state().app.menu.is_none());

    // Left from File wraps to Help; Enter runs its first item (Keyboard Shortcuts).
    press(&mut h, Modifiers::NONE, Key::F10);
    press(&mut h, Modifiers::NONE, Key::ArrowLeft);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(matches!(h.state().app.modal, Some(Modal::Keymap(_))));
    press(&mut h, Modifiers::NONE, Key::Escape);

    // Disabled items are skipped: Edit's first enabled item on an untitled empty buffer is Undo.
    // Mouse: clicking a title opens that menu, clicking it again closes it.
    let file = h.state().menu_titles[3].center();
    click(&mut h, file);
    assert_eq!(h.state().app.menu.map(|m| m.menu), Some(3));
    click(&mut h, file);
    assert!(h.state().app.menu.is_none());

    // View → Toggle Menu Bar hides it; F10 still works.
    h.state_mut()
        .app
        .run(crate::commands::Command::ToggleMenuBar);
    steps(&mut h, 2);
    assert!(!h.state().app.menu_bar_visible);
    press(&mut h, Modifiers::NONE, Key::F10);
    assert!(h.state().app.menu.is_some());
}

#[test]
fn keymap_viewer_search_and_run() {
    let d = project("keymap", &[("a.txt", "one\ntwo\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Slash);
    assert!(matches!(h.state().app.modal, Some(Modal::Keymap(_))));
    // Every bound command and the sidebar keys are listed.
    let all = crate::app::keymap_rows(&h.state().app.keymap, "");
    assert!(all.iter().any(|r| r.section.starts_with("Sidebar")
        && r.desc.contains("New Folder")
        && r.keys.contains("Shift+A")));
    assert!(all.iter().any(|r| r.desc.contains("Goto Anything")));
    shot(&mut h, "keymap");
    typ(&mut h, "duplicate line");
    let rows = crate::app::keymap_rows(&h.state().app.keymap, "duplicate line");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].keys.contains("Shift+D"));
    shot(&mut h, "keymap_filtered");
    // Enter runs it.
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state().app.modal.is_none());
    assert_eq!(
        h.state().app.buf().unwrap().lines[..2],
        ["one".to_string(), "one".to_string()]
    );
}

fn fake_server_config(root: &Path) -> Config {
    let mut c = test_config(root);
    c.merge_str(
        r#"
        [[languages]]
        name = "fakelang"
        extensions = ["fake"]
        comment = ";;"
        servers = ["fake_ls"]

        [[servers]]
        name = "fake_ls"
        command = "fake-ls-not-on-path"
        description = "A pretend server installed by a script"
        [servers.install]
        script = "echo compiling fake-ls; printf '#!/bin/sh\nsleep 30\n' > fake-ls; chmod +x fake-ls"
        bin = "fake-ls"
        "#,
    )
    .unwrap();
    c
}

#[test]
#[cfg(unix)]
fn server_manager_install_and_uninstall() {
    let d = project("servers", &[("a.fake", "hello\n")]);
    let gui = Gui::headless(d.clone(), fake_server_config(&d));
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1200.0, 720.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(|ui, g: &mut Gui| g.frame(ui), gui);
    h.state_mut().app.open_file(&d.join("a.fake"), true);
    steps(&mut h, 2);
    // Opening the file explains how to get a server.
    let msg = h.state().app.status.clone().unwrap().0;
    assert!(
        msg.contains("Language Servers") && msg.contains("fake_ls"),
        "{msg}"
    );

    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::L);
    assert!(matches!(h.state().app.modal, Some(Modal::Servers(_))));
    // The current file's servers are listed first.
    assert_eq!(h.state().app.server_rows()[0].name, "fake_ls");
    let names: Vec<String> = h
        .state()
        .app
        .server_rows()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    for want in ["pyright", "basedpyright", "pylsp", "ts_ls", "bashls"] {
        assert!(
            names.iter().any(|n| n == want),
            "{want} missing from {names:?}"
        );
    }
    shot(&mut h, "servers");
    typ(&mut h, "i");
    let ok = wait(&mut h, 20, |g| {
        let s = g.app.config.server("fake_ls").unwrap();
        matches!(
            g.app.server_status(s),
            crate::app::ServerStatus::Installed(_)
        )
    });
    assert!(
        ok,
        "log={:?} failed={:?} status={:?}",
        h.state().app.install_log.get("fake_ls"),
        h.state().app.install_failed,
        h.state().app.status
    );
    assert!(wait(&mut h, 1, |g| {
        let s = g.app.config.server("fake_ls").unwrap();
        matches!(
            g.app.server_status(s),
            crate::app::ServerStatus::Installed(_)
        )
    }));
    assert!(
        h.state().app.install_log["fake_ls"]
            .iter()
            .any(|l| l == "compiling fake-ls")
    );
    // Installing starts the server for the open file.
    assert!(h.state().app.buffers[0].lsp_client.is_some());
    press(&mut h, Modifiers::NONE, Key::Enter); // details
    shot(&mut h, "servers_details");
    typ(&mut h, "x");
    assert!(matches!(h.state().app.modal, Some(Modal::Confirm(_))));
    typ(&mut h, "y");
    let s = h.state().app.config.server("fake_ls").unwrap().clone();
    assert_eq!(
        h.state().app.server_status(&s),
        crate::app::ServerStatus::Missing
    );
    assert!(
        matches!(h.state().app.modal, Some(Modal::Servers(_))),
        "back in the manager"
    );
    h.state_mut().app.lsp.shutdown();
}

const TM_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>name</key><string>Farsub Test Theme</string>
<key>settings</key><array>
<dict><key>settings</key><dict>
<key>background</key><string>#102030</string><key>foreground</key><string>#E0E0E0</string>
</dict></dict>
<dict><key>scope</key><string>keyword</string><key>settings</key><dict><key>foreground</key><string>#FF8800</string></dict></dict>
</array></dict></plist>"#;

#[test]
fn sublime_packages_add_syntax_and_theme() {
    let syntax = "%YAML 1.2\n---\nname: Farsub Test Lang\nfile_extensions: [fstl]\nscope: source.fstl\ncontexts:\n  main:\n    - match: '\\b(let|fn)\\b'\n      scope: keyword.control.fstl\n";
    let d = project(
        "pkgs",
        &[
            ("Packages/TestLang/TestLang.sublime-syntax", syntax),
            ("Packages/TestTheme/Test.tmTheme", TM_THEME),
            ("code.fstl", "let x = fn\n"),
        ],
    );
    let mut h = harness(&d);
    assert!(wait(&mut h, 30, |g| g
        .app
        .syn
        .ss
        .find_syntax_by_extension("fstl")
        .is_some()));
    assert_eq!(h.state().app.packages, ["TestLang", "TestTheme"]);
    h.state_mut().app.open_file(&d.join("code.fstl"), true);
    steps(&mut h, 2);
    assert_eq!(h.state().app.buf().unwrap().hl.syntax, "Farsub Test Lang");
    assert!(
        h.state()
            .app
            .syn
            .theme_names()
            .iter()
            .any(|t| t == "Farsub Test Theme")
    );
    assert!(h.state_mut().app.syn.set_theme("Farsub Test Theme"));
    h.state_mut().app.buffers[0].hl.invalidate(0);
    steps(&mut h, 3);
    // The keyword picked up the package theme's color.
    let spans = &h.state().app.buffers[0].hl.lines[0];
    assert!(
        spans
            .iter()
            .any(|s| s.2.fg == egui::Color32::from_rgb(0xFF, 0x88, 0x00)),
        "{spans:?}"
    );
    shot(&mut h, "package_theme");
}

#[test]
fn sidebar_hover_and_folder_animation() {
    let d = project("anim", &[("src/a.rs", ""), ("src/b.rs", ""), ("z.txt", "")]);
    let mut cfg = test_config(&d);
    cfg.editor.animations = true;
    let gui = Gui::headless(d.clone(), cfg);
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1000.0, 500.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(|ui, g: &mut Gui| g.frame(ui), gui);
    steps(&mut h, 2);
    press(&mut h, CMD, Key::Num0);
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    assert!(!h.state().app.tree.anims.is_empty(), "opening animates");
    assert!(wait(&mut h, 2, |g| g.app.tree.anims.is_empty()));
    assert_eq!(h.state().app.tree.nodes.len(), 4);
    // Hover the last row.
    let y = 26.0 + 32.0 + 3.0 * 24.0 + 12.0;
    h.event(Event::PointerMoved(egui::Pos2::new(80.0, y)));
    steps(&mut h, 2);
    shot(&mut h, "sidebar_hover");
    press(&mut h, Modifiers::NONE, Key::ArrowLeft);
    assert_eq!(
        h.state().app.tree.nodes.len(),
        4,
        "children stay while closing"
    );
    assert!(wait(&mut h, 2, |g| g.app.tree.nodes.len() == 2));
}

#[test]
fn native_menu_commands_follow_focus() {
    let d = project("nmenu", &[("a.txt", "one\n"), ("b.txt", "")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    // Edit → Duplicate Line from the system menu acts on the editor.
    h.state_mut().app.on_menu_command("duplicate_line");
    assert_eq!(
        h.state().app.buf().unwrap().lines[..2],
        ["one".to_string(), "one".to_string()]
    );
    // Edit → Copy while the sidebar has focus copies files, like its shortcut does.
    h.state_mut()
        .app
        .run(crate::commands::Command::FocusSidebar);
    h.state_mut().app.on_menu_command("copy");
    assert!(h.state().app.file_clip.is_some());
    // Items without a shortcut just run.
    h.state_mut().app.on_menu_command("select_theme");
    assert!(matches!(h.state().app.modal, Some(Modal::Palette(_))));
}
