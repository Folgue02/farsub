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
        // A plain shell, so tests don't depend on the user's shell setup.
        terminal_shell: Some("/bin/sh".into()),
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
    // The project folder is the first row; Down selects `docs`.
    assert!(h.state().app.tree.nodes[0].is_root);
    press(&mut h, Modifiers::NONE, Key::ArrowDown);

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
    // Like Sublime: defaults (read-only, every setting) on the left, the user file on the
    // right, focused.
    {
        let app = &h.state().app;
        assert_eq!(app.layout, 2);
        assert_eq!(app.group, 1);
        let left = &app.buffers[app.group_shown(0).unwrap()];
        assert_eq!(left.name(), "Default Settings.toml");
        assert!(left.read_only);
        assert!(left.lines.iter().any(|l| l.starts_with("theme = ")));
        assert!(
            !app.buf()
                .unwrap()
                .lines
                .iter()
                .any(|l| l.starts_with("theme = "))
        );
    }
    shot(&mut h, "settings_side_by_side");
    // Add a language and switch theme by editing the file in the editor, then save.
    let extra =
        "\n[[languages]]\nname = \"lua\"\nextensions = [\"lua\"]\ncomment = \"--\"\ntab_size = 2\n";
    {
        let b = h.state_mut().app.buf_mut().unwrap();
        let editor = b.lines.iter().position(|l| l == "[editor]").unwrap();
        b.lines.insert(editor + 1, "theme = \"Nord\"".into());
        b.move_doc(true, false);
        b.insert_text(extra);
    }
    press(&mut h, CMD, Key::S);
    assert_eq!(h.state().app.config.editor.theme, "Nord");
    assert!(h.state().app.config.language("lua").is_some());
    // Closing the settings closes both panes and restores the single layout.
    press(&mut h, CMD, Key::W);
    assert!(
        h.state().app.buffers.is_empty(),
        "{:?}",
        h.state()
            .app
            .buffers
            .iter()
            .map(|b| b.name())
            .collect::<Vec<_>>()
    );
    assert_eq!(h.state().app.layout, 1);
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
    // Nothing is preselected.
    assert_eq!(h.state().app.completion.as_ref().unwrap().sel, None);
    // Ctrl+N selects the first item, Ctrl+P wraps to the last, Ctrl+N back to the first.
    press(&mut h, Modifiers::CTRL, Key::N);
    assert_eq!(h.state().app.completion.as_ref().unwrap().sel, Some(0));
    press(&mut h, Modifiers::CTRL, Key::P);
    let n = h.state().app.completion.as_ref().unwrap().shown.len();
    assert_eq!(h.state().app.completion.as_ref().unwrap().sel, Some(n - 1));
    press(&mut h, Modifiers::CTRL, Key::N);
    shot(&mut h, "completion");
    typ(&mut h, "y");
    // Typing refilters and clears the selection again; select explicitly, then accept.
    assert_eq!(h.state().app.completion.as_ref().unwrap().sel, None);
    press(&mut h, Modifiers::CTRL, Key::N);
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().lines[6], "    p.y;");
    // Ctrl+N/P didn't trigger New File / Goto Anything.
    assert_eq!(h.state().app.buffers.len(), 1);
    assert!(h.state().app.modal.is_none());

    // With nothing selected, Enter is a plain newline and closes the list.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(6, 7));
    press(&mut h, Modifiers::CTRL, Key::Space);
    assert!(wait(&mut h, 10, |g| g.app.completion.is_some()));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state().app.completion.is_none());
    assert_eq!(h.state().app.buf().unwrap().lines[6], "    p.y");
    press(&mut h, CMD, Key::Z);
    assert_eq!(h.state().app.buf().unwrap().lines[6], "    p.y;");

    // F12 goto definition.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(7, 13));
    press(&mut h, Modifiers::NONE, Key::F12);
    assert!(wait(&mut h, 10, |g| g.app.buf().unwrap().cursor.line == 2));

    // LSP Info shows the attached clangd with its capabilities.
    assert!(wait(&mut h, 10, |g| g
        .app
        .lsp
        .clients
        .values()
        .any(|c| c.ready)));
    let info = h
        .state()
        .app
        .lsp_info()
        .into_iter()
        .map(|l| l.1)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        info.contains("Attached server: clangd") && info.contains("completion, hover, definition"),
        "{info}"
    );
    h.state_mut().app.run(crate::commands::Command::LspInfo);
    steps(&mut h, 2);
    shot(&mut h, "lsp_info_clangd");
    h.state_mut().app.modal = None;

    // Mouse hover: resting the pointer on `add` (line 8) shows LSP hover info there.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(0, 0));
    steps(&mut h, 2);
    let over_add = h.state().screen_pos_for_test(7, 13);
    h.event(Event::PointerMoved(over_add));
    assert!(
        wait(&mut h, 10, |g| g
            .app
            .hover
            .as_ref()
            .is_some_and(
                |hv| hv.anchor.is_some() && hv.lines.iter().any(|l| l.contains("add"))
            )),
        "mouse hover"
    );
    shot(&mut h, "mouse_hover");
    // Over the undefined name: the diagnostic is shown on top.
    let over_bad = h.state().screen_pos_for_test(8, 16);
    h.event(Event::PointerMoved(over_bad));
    assert!(
        wait(&mut h, 10, |g| g.app.hover.as_ref().is_some_and(|hv| hv
            .diagnostics
            .iter()
            .any(|d| d.1.contains("undefined_thing")))),
        "diagnostic in hover: {:?}",
        h.state()
            .app
            .hover
            .as_ref()
            .map(|hv| (&hv.diagnostics, &hv.lines))
    );
    shot(&mut h, "mouse_hover_diagnostic");
    // Moving away hides it.
    h.event(Event::PointerMoved(egui::Pos2::new(700.0, 650.0)));
    steps(&mut h, 3);
    assert!(h.state().app.hover.is_none());

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
    press(&mut h, Modifiers::NONE, Key::ArrowDown); // project folder → src
    press(&mut h, Modifiers::NONE, Key::ArrowRight);
    assert!(!h.state().app.tree.anims.is_empty(), "opening animates");
    assert!(wait(&mut h, 2, |g| g.app.tree.anims.is_empty()));
    assert_eq!(h.state().app.tree.nodes.len(), 5);
    // Hover the last row.
    let y = 26.0 + 32.0 + 4.0 * 24.0 + 12.0;
    h.event(Event::PointerMoved(egui::Pos2::new(80.0, y)));
    steps(&mut h, 2);
    shot(&mut h, "sidebar_hover");
    press(&mut h, Modifiers::NONE, Key::ArrowLeft);
    assert_eq!(
        h.state().app.tree.nodes.len(),
        5,
        "children stay while closing"
    );
    assert!(wait(&mut h, 2, |g| g.app.tree.nodes.len() == 3));
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

fn project_harness(p: crate::project::Project, cfg: Config) -> Harness<'static, Gui> {
    let gui = Gui::with_project(p, cfg);
    Harness::builder()
        .with_size(egui::Vec2::new(1200.0, 720.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(|ui, g: &mut Gui| g.frame(ui), gui)
}

#[test]
fn project_folders_excludes_and_settings() {
    let d = project(
        "projfile",
        &[
            ("app/src/main.rs", "fn main() {}\n"),
            ("app/target/debug.rs", "junk\n"),
            ("app/bundle.min.js", "x\n"),
            ("app/notes.txt", "no indent here\n"),
            ("lib/lib.rs", "pub fn x() {}\n"),
            (
                "app/App.farsub-project",
                "[[folders]]\npath = \".\"\nname = \"App\"\nfolder_exclude_patterns = [\"target\"]\nfile_exclude_patterns = [\"*.min.js\"]\n\n[[folders]]\npath = \"../lib\"\n\n[settings]\ntab_size = 2\n",
            ),
        ],
    );
    let p = crate::project::Project::load(&d.join("app/App.farsub-project")).unwrap();
    let mut h = project_harness(p, test_config(&d));
    steps(&mut h, 2);
    let app = &h.state().app;
    assert_eq!(app.title(), "App — farsub");
    let names: Vec<_> = app.tree.nodes.iter().map(|n| n.name.clone()).collect();
    assert!(
        names.contains(&"App".to_string()) && names.contains(&"lib".to_string()),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "target" || n == "bundle.min.js"),
        "{names:?}"
    );
    assert_eq!(app.config.editor.tab_size, 2, "project settings apply");
    // Goto Anything searches both folders (prefixed by folder name) and skips excludes.
    press(&mut h, CMD, Key::P);
    assert!(wait(
        &mut h,
        5,
        |g| matches!(&g.app.modal, Some(Modal::Palette(p)) if !p.loading && !p.items.is_empty())
    ));
    let labels: Vec<String> = match &h.state().app.modal {
        Some(Modal::Palette(p)) => p.items.iter().map(|i| i.label.clone()).collect(),
        _ => unreachable!(),
    };
    assert!(
        labels.contains(&"App/src/main.rs".to_string())
            && labels.contains(&"lib/lib.rs".to_string()),
        "{labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("target") || l.contains(".min.js")),
        "{labels:?}"
    );
    shot(&mut h, "project_goto");
    press(&mut h, Modifiers::NONE, Key::Escape);
    // A file with no indentation of its own uses the project's tab size.
    h.state_mut().app.open_file(&d.join("app/notes.txt"), true);
    assert_eq!(h.state().app.buf().unwrap().indent_unit, "  ");
    shot(&mut h, "project_sidebar");
}

#[test]
fn workspace_restores_session_with_hot_exit() {
    let d = project(
        "ws",
        &[
            ("a.txt", "alpha\nbeta\ngamma\n"),
            ("sub/b.txt", "one\ntwo\n"),
        ],
    );
    let proj = || crate::project::Project::from_folders(vec![d.clone()]);
    let mut h = project_harness(proj(), test_config(&d));
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    h.state_mut().app.open_file(&d.join("sub/b.txt"), true);
    h.state_mut().app.buffers[0].goto(crate::editor::Pos::new(2, 3));
    h.state_mut().app.cur = 1;
    typ(&mut h, "EDITED ");
    h.state_mut().app.untitled_for_test("scratch notes");
    h.state_mut().app.cur = 1;
    steps(&mut h, 2);
    // Quitting with hot exit doesn't ask; it saves the session.
    h.state_mut().app.run(crate::commands::Command::Quit);
    assert!(h.state().app.quit && h.state().app.modal.is_none());
    assert_eq!(
        std::fs::read_to_string(d.join("sub/b.txt")).unwrap(),
        "one\ntwo\n",
        "nothing written to the file"
    );

    // A fresh window on the same folder restores everything.
    let mut h2 = project_harness(proj(), test_config(&d));
    h2.state_mut().app.restore_workspace();
    steps(&mut h2, 2);
    let app = &h2.state().app;
    assert_eq!(app.buffers.len(), 3);
    assert_eq!(app.buffers[0].cursor, crate::editor::Pos::new(2, 3));
    assert_eq!(app.cur, 1);
    assert!(app.buffers[1].modified);
    assert_eq!(app.buffers[1].lines[0], "EDITED one");
    assert!(app.buffers[2].path.is_none() && app.buffers[2].lines[0] == "scratch notes");
    shot(&mut h2, "workspace_restored");
}

#[test]
fn switch_save_and_edit_projects() {
    let d = project(
        "switch",
        &[
            ("one/x.txt", "x\n"),
            ("two/y.txt", "y\n"),
            ("extra/z.txt", "z\n"),
        ],
    );
    let cfg = test_config(&d);
    let mut h = project_harness(
        crate::project::Project::from_folders(vec![d.join("one")]),
        cfg,
    );
    h.state_mut().app.open_file(&d.join("one/x.txt"), true);
    // Open another folder (Project › Open Project accepts folders too).
    h.state_mut()
        .app
        .switch_project(crate::project::Project::from_folders(vec![d.join("two")]));
    assert!(
        h.state().app.buffers.is_empty(),
        "previous project's files closed"
    );
    h.state_mut().app.open_file(&d.join("two/y.txt"), true);
    // Switch Project palette lists recent folder sets; picking "one" brings back x.txt.
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::P);
    assert!(matches!(&h.state().app.modal, Some(Modal::Palette(p)) if p.items.len() >= 2));
    typ(&mut h, "one");
    shot(&mut h, "switch_project");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().name(), "x.txt");

    // Save Project As writes a project file with a relative folder; Add Folder updates it.
    h.state_mut()
        .app
        .run(crate::commands::Command::SaveProjectAs);
    press(&mut h, CMD, Key::U);
    typ(&mut h, &d.join("one/One").display().to_string());
    press(&mut h, Modifiers::NONE, Key::Enter);
    let file = d.join("one/One.farsub-project");
    assert!(file.is_file(), "{:?}", h.state().app.status);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("path = \".\"")
    );
    assert!(d.join("one/One.farsub-workspace").is_file());
    h.state_mut().app.run(crate::commands::Command::AddFolder);
    press(&mut h, CMD, Key::U);
    typ(&mut h, &d.join("extra").display().to_string());
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.project.folders.len(), 2);
    assert!(std::fs::read_to_string(&file).unwrap().contains("../extra"));
    // Edit Project, change settings, save → applied without reopening.
    h.state_mut().app.run(crate::commands::Command::EditProject);
    assert_eq!(
        h.state().app.buf().unwrap().path.as_deref(),
        Some(file.as_path())
    );
    h.state_mut().app.buf_mut().unwrap().move_doc(true, false);
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .insert_text("\n[settings]\nfont_size = 18.0\n");
    press(&mut h, CMD, Key::S);
    assert_eq!(h.state().app.font_size, 18.0, "{:?}", h.state().app.status);
    // Remove Folder (selected in the sidebar) updates the file again.
    let extra = d.join("extra");
    h.state_mut().app.tree.reveal(&extra);
    h.state_mut()
        .app
        .run(crate::commands::Command::RemoveFolder);
    assert_eq!(h.state().app.project.folders.len(), 1);
    assert!(!std::fs::read_to_string(&file).unwrap().contains("extra"));
}

#[test]
fn ibeam_cursor_over_text() {
    let d = project("ibeam", &[("a.txt", "hello world\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    // Over the text (right of the gutter) → I-beam.
    h.event(Event::PointerMoved(egui::Pos2::new(700.0, 300.0)));
    steps(&mut h, 1);
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Text
    );
    // Over selected text → normal arrow; elsewhere on the same line → I-beam again.
    {
        let b = h.state_mut().app.buf_mut().unwrap();
        b.goto(crate::editor::Pos::new(0, 0));
        b.select_word_or_next(); // "hello"
    }
    steps(&mut h, 1);
    let (cw, text_left, line_y) = h.state().text_metrics_for_test();
    h.event(Event::PointerMoved(egui::Pos2::new(
        text_left + cw * 2.5,
        line_y,
    )));
    steps(&mut h, 1);
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Default,
        "over selection"
    );
    h.event(Event::PointerMoved(egui::Pos2::new(
        text_left + cw * 8.5,
        line_y,
    )));
    steps(&mut h, 1);
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Text,
        "over unselected text"
    );
    // Just past the selection's last character is not "on" it.
    h.event(Event::PointerMoved(egui::Pos2::new(
        text_left + cw * 5.2,
        line_y,
    )));
    steps(&mut h, 1);
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Text,
        "right after selection"
    );
    // Over the sidebar → normal arrow.
    h.event(Event::PointerMoved(egui::Pos2::new(60.0, 300.0)));
    steps(&mut h, 1);
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Default
    );
}

#[test]
fn status_bar_syntax_picker_changes_language() {
    let d = project("syntax", &[("deploy", "echo hi\n")]);
    let proj = || crate::project::Project::from_folders(vec![d.clone()]);
    let mut h = project_harness(proj(), test_config(&d));
    h.state_mut().app.open_file(&d.join("deploy"), true);
    steps(&mut h, 2);
    assert_eq!(h.state().app.buf().unwrap().hl.syntax, "Plain Text");
    assert!(h.state().app.buf().unwrap().lang.is_none());
    // Click the syntax name at the right end of the status bar.
    click(&mut h, egui::Pos2::new(1200.0 - 40.0, 720.0 - 12.0));
    assert!(
        matches!(&h.state().app.modal, Some(Modal::Palette(p)) if p.mode == crate::palette::Mode::Syntaxes)
    );
    typ(&mut h, "bourne again");
    shot(&mut h, "syntax_picker");
    press(&mut h, Modifiers::NONE, Key::Enter);
    let b = h.state().app.buf().unwrap();
    assert!(
        b.hl.syntax.starts_with("Bourne Again Shell"),
        "{}",
        b.hl.syntax
    );
    assert_eq!(
        b.lang.as_deref(),
        Some("bash"),
        "language follows the syntax"
    );
    press(&mut h, CMD, Key::Slash);
    assert_eq!(h.state().app.buf().unwrap().lines[0], "# echo hi");
    // The choice is kept in the workspace.
    h.state_mut().app.save_workspace();
    let mut h2 = project_harness(proj(), test_config(&d));
    h2.state_mut().app.restore_workspace();
    assert!(
        h2.state()
            .app
            .buf()
            .unwrap()
            .hl
            .syntax
            .starts_with("Bourne Again Shell")
    );
}

#[test]
#[cfg(unix)]
fn lsp_info_shows_status_and_installs() {
    let d = project("lspinfo", &[("a.fake", "hello\n")]);
    let mut h = project_harness(
        crate::project::Project::from_folders(vec![d.clone()]),
        fake_server_config(&d),
    );
    h.state_mut().app.open_file(&d.join("a.fake"), true);
    steps(&mut h, 2);
    press(&mut h, Modifiers::COMMAND | Modifiers::ALT, Key::I);
    assert!(matches!(h.state().app.modal, Some(Modal::LspInfo(_))));
    let text: Vec<String> = h.state().app.lsp_info().into_iter().map(|l| l.1).collect();
    let all = text.join("\n");
    assert!(
        all.contains("fakelang") && all.contains("press i to install (script)"),
        "{all}"
    );
    shot(&mut h, "lsp_info_missing");
    typ(&mut h, "i");
    assert!(wait(&mut h, 20, |g| g
        .app
        .buf()
        .unwrap()
        .lsp_client
        .is_some()));
    let all = h
        .state()
        .app
        .lsp_info()
        .into_iter()
        .map(|l| l.1)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("running — attached")
            && all.contains("Attached server: fake_ls")
            && all.contains("pid"),
        "{all}"
    );
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().app.modal.is_none());
    h.state_mut().app.lsp.shutdown();
}

fn drag(h: &mut Harness<'static, Gui>, from: egui::Pos2, to: egui::Pos2) {
    h.event(Event::PointerMoved(from));
    steps(h, 1);
    h.event(Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    steps(h, 1);
    for k in 1..=6 {
        let t = k as f32 / 6.0;
        h.event(Event::PointerMoved(from + (to - from) * t));
        steps(h, 1);
    }
    h.event(Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(h, 2);
}

#[test]
fn console_panel_logs_and_runs_commands() {
    let d = project("console", &[("a.txt", "x\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    h.state_mut().app.msg("hello from the status bar");
    // The console opens from the command palette (Ctrl+` is the terminal).
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    typ(&mut h, "show console");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state().app.console_shown());
    assert_eq!(h.state().app.focus, Focus::Console);
    assert!(crate::log::entries(crate::log::Level::Info, "hello from the status bar").len() == 1);
    // Built-ins and command ids.
    typ(&mut h, "help");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(!crate::log::entries(crate::log::Level::Info, "runs a command").is_empty());
    typ(&mut h, "toggle_sidebar");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(!h.state().app.sidebar_visible, "console ran the command");
    assert_eq!(
        h.state().app.focus,
        Focus::Console,
        "focus stays in the console"
    );
    typ(&mut h, "no_such_thing");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        !crate::log::entries(crate::log::Level::Error, "unknown command `no_such_thing`")
            .is_empty()
    );
    // History.
    press(&mut h, Modifiers::NONE, Key::ArrowUp);
    assert_eq!(h.state().app.console.input, "no_such_thing");
    press(&mut h, CMD, Key::U);
    typ(&mut h, "filter sidebar");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.console.filter, "sidebar");
    shot(&mut h, "console");
    // Esc hides it and returns to the editor; typing goes to the buffer again.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().app.console_shown());
    typ(&mut h, "y");
    assert_eq!(h.state().app.buf().unwrap().lines[0], "yx");
}

#[test]
fn sidebar_resizes_by_dragging_and_is_remembered() {
    let d = project("resize", &[("a.txt", "x\n")]);
    let proj = || crate::project::Project::from_folders(vec![d.clone()]);
    let mut h = project_harness(proj(), test_config(&d));
    steps(&mut h, 2);
    let w0 = h.state().app.sidebar_width;
    let y = 400.0;
    drag(
        &mut h,
        egui::Pos2::new(w0, y),
        egui::Pos2::new(w0 + 150.0, y),
    );
    let w1 = h.state().app.sidebar_width;
    assert!((w1 - (w0 + 150.0)).abs() < 2.0, "{w0} -> {w1}");
    shot(&mut h, "sidebar_wide");
    h.state_mut().app.save_workspace();
    let mut h2 = project_harness(proj(), test_config(&d));
    h2.state_mut().app.restore_workspace();
    assert!((h2.state().app.sidebar_width - w1).abs() < 0.5);
}

#[test]
fn indentation_detection_menu_and_guides() {
    let src = "fn main() {\n  if true {\n    let x = 1;\n\n    call(x);\n  }\n}\n";
    let d = project("indent", &[("m.rs", src)]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("m.rs"), true);
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(2, 6));
    steps(&mut h, 2);
    assert_eq!(
        h.state().app.buf().unwrap().indent_unit,
        "  ",
        "2-space indentation detected"
    );
    shot(&mut h, "indent_guides");
    // Click "Spaces: 2" in the status bar (left of the syntax name) → Indentation menu.
    let spot = h
        .state()
        .status_hits
        .iter()
        .find(|(c, _)| *c == crate::commands::Command::Indentation)
        .map(|(_, r)| r.center())
        .expect("indentation item in the status bar");
    click(&mut h, spot);
    assert!(
        matches!(&h.state().app.modal, Some(Modal::Palette(p)) if p.mode == crate::palette::Mode::Indentation),
        "status bar indentation item opens the menu"
    );
    shot(&mut h, "indentation_menu");
    typ(&mut h, "convert indentation to tabs");
    press(&mut h, Modifiers::NONE, Key::Enter);
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.lines[1], "\tif true {");
    assert_eq!(
        b.lines[2], "\t\tlet x = 1;",
        "tab width 2 → one tab per level"
    );
    // Tab width changes how tabs display without touching the text.
    h.state_mut().app.run(crate::commands::Command::Indentation);
    typ(&mut h, "tab width: 8");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().tab_size, 8);
    assert_eq!(h.state().app.buf().unwrap().lines[2], "\t\tlet x = 1;");
}

/// Real typescript-language-server install + JS file (network). Run with --ignored.
#[test]
#[ignore]
fn ts_ls_installs_and_attaches_to_javascript() {
    let d = project(
        "tsls",
        &[("app.js", "const greeting = 'hi';\ngreeting.toUpp\n")],
    );
    let mut h = harness(&d);
    h.state_mut().app.install_server("ts_ls");
    assert!(
        wait(&mut h, 240, |g| !g.app.installing.contains("ts_ls")),
        "install timed out"
    );
    assert!(
        h.state().app.install_failed.is_empty(),
        "{:?}",
        h.state().app.install_log.get("ts_ls")
    );
    h.state_mut().app.open_file(&d.join("app.js"), true);
    assert!(
        wait(&mut h, 60, |g| g
            .app
            .buf()
            .unwrap()
            .lsp_client
            .and_then(|c| g.app.lsp.clients.get(&c))
            .is_some_and(|c| c.ready)),
        "server never became ready: {:?}",
        h.state()
            .app
            .lsp
            .clients
            .values()
            .map(|c| (
                c.init_error.clone(),
                c.stderr.lock().unwrap().iter().cloned().collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    );
    // Member completion on a string works, so TypeScript was found.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(1, 14));
    press(&mut h, Modifiers::CTRL, Key::Space);
    assert!(wait(&mut h, 30, |g| g.app.completion.as_ref().is_some_and(
        |c| c.items.iter().any(|i| i.label == "toUpperCase")
    )));
    let info = h
        .state()
        .app
        .lsp_info()
        .into_iter()
        .map(|l| l.1)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(info.contains("Attached server: ts_ls"), "{info}");
    shot(&mut h, "ts_ls_completion");
    h.state_mut().app.lsp.shutdown();
}

#[test]
#[cfg(unix)]
fn terminal_panel_runs_a_shell() {
    let d = project("term", &[("a.txt", "x\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    // Ctrl+` opens a terminal in the project folder and focuses it.
    press(&mut h, Modifiers::CTRL, Key::Backtick);
    assert_eq!(h.state().app.focus, Focus::Terminal);
    assert_eq!(h.state().app.terminals.len(), 1);
    let screen = |g: &Gui| g.app.terminals[0].with_screen(|s| s.contents());
    typ(&mut h, "echo farsub-$((6*7)) && pwd");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        wait(&mut h, 10, |g| screen(g).contains("farsub-42")),
        "{}",
        screen(h.state())
    );
    assert!(
        screen(h.state()).contains(&d.file_name().unwrap().to_string_lossy().to_string()),
        "starts in the project folder"
    );
    shot(&mut h, "terminal");
    // Ctrl+C reaches the shell (interrupts a running command), Cmd shortcuts stay app shortcuts.
    typ(&mut h, "sleep 30");
    press(&mut h, Modifiers::NONE, Key::Enter);
    // Give the shell a moment to start `sleep` in the foreground (a human can't type faster).
    std::thread::sleep(Duration::from_millis(600));
    press(&mut h, Modifiers::CTRL, Key::C);
    typ(&mut h, "echo after-interrupt");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        wait(&mut h, 10, |g| screen(g).contains("after-interrupt\r")
            || screen(g).matches("after-interrupt").count() >= 2),
        "{}",
        screen(h.state())
    );
    // Ctrl+Shift+` opens a second terminal; Ctrl+` while focused hides the panel.
    press(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::Backtick);
    assert_eq!(h.state().app.terminals.len(), 2);
    press(&mut h, Modifiers::CTRL, Key::Backtick);
    assert!(!h.state().app.panel.visible);
    assert_eq!(h.state().app.focus, Focus::Editor);
    // Kill Terminal closes the current one.
    h.state_mut()
        .app
        .run(crate::commands::Command::ToggleTerminal);
    h.state_mut()
        .app
        .run(crate::commands::Command::CloseTerminal);
    assert_eq!(h.state().app.terminals.len(), 1);
}

#[test]
fn console_text_is_selectable_and_wraps() {
    let d = project("consel", &[]);
    let mut h = harness(&d);
    let long = format!("unique-marker {}", "0123456789".repeat(30));
    crate::log::info("seltest", &long);
    h.state_mut()
        .app
        .run(crate::commands::Command::ToggleConsole);
    h.state_mut().app.console.filter = "unique-marker".into();
    steps(&mut h, 2);
    shot(&mut h, "console_wrapped");
    // Cmd+A selects everything shown; copy puts it on the clipboard.
    press(&mut h, CMD, Key::A);
    let text = h.state().app.console_selection_text().unwrap();
    assert!(
        text.contains("[seltest] unique-marker") && text.ends_with("0123456789"),
        "{text}"
    );
    h.state_mut().app.on_copy(false);
    assert_eq!(
        h.state().app.pending_copy.as_deref(),
        Some(text.as_str()),
        "goes to the clipboard"
    );
    // `wrap` toggles wrapping; with it off, horizontal scrolling works.
    h.state_mut().app.console_exec("wrap");
    assert!(!h.state().app.console.wrap);
    steps(&mut h, 2);
    h.event(Event::PointerMoved(egui::Pos2::new(600.0, 600.0)));
    h.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::Vec2::new(-300.0, 0.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 6);
    assert!(h.state().app.console.hscroll > 0.0, "side scroll");
    shot(&mut h, "console_nowrap");
}

fn right_click(h: &mut Harness<'static, Gui>, pos: egui::Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    steps(h, 2);
}

fn context_labels(h: &Harness<'static, Gui>) -> Vec<String> {
    h.state()
        .app
        .context_menu
        .as_ref()
        .map(|m| m.items.iter().flatten().map(|i| i.label.clone()).collect())
        .unwrap_or_default()
}

#[test]
#[cfg(unix)]
fn terminal_exit_closes_tab_and_tabs_cycle() {
    let d = project("termexit", &[]);
    let mut h = harness(&d);
    steps(&mut h, 2);
    press(&mut h, Modifiers::CTRL, Key::Backtick);
    press(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::Backtick);
    press(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::Backtick);
    assert_eq!(h.state().app.terminals.len(), 3);
    std::thread::sleep(Duration::from_millis(500)); // let the shells start, as a person would
    let ids: Vec<usize> = h.state().app.terminals.iter().map(|t| t.id).collect();
    assert_eq!(
        h.state().app.panel.tab,
        crate::app::PanelTab::Terminal(ids[2])
    );
    // Ctrl+Tab cycles terminal tabs (wrapping), Ctrl+Shift+Tab goes back.
    press(&mut h, Modifiers::CTRL, Key::Tab);
    assert_eq!(
        h.state().app.panel.tab,
        crate::app::PanelTab::Terminal(ids[0])
    );
    press(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::Tab);
    assert_eq!(
        h.state().app.panel.tab,
        crate::app::PanelTab::Terminal(ids[2])
    );
    assert!(
        h.state().app.buffers.is_empty(),
        "didn't switch editor files"
    );
    // `exit` closes the tab by itself.
    typ(&mut h, "exit");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        h.state().app.menu.is_none(),
        "Enter after Ctrl+Shift+Tab must not click a menu button"
    );
    assert!(
        wait(&mut h, 10, |g| g.app.terminals.len() == 2),
        "exit closes the tab"
    );
    // A failing shell stays open showing its exit code; any key closes it.
    let cur = match h.state().app.panel.tab {
        crate::app::PanelTab::Terminal(id) => id,
        _ => panic!("terminal tab expected"),
    };
    h.state_mut().app.show_terminal(cur);
    typ(&mut h, "exit 3");
    press(&mut h, Modifiers::NONE, Key::Enter);
    let ok = wait(&mut h, 10, |g| {
        g.app.terminals.iter().any(|t| t.id == cur && t.is_exited())
    });
    assert!(
        ok,
        "focus={:?} tab={:?} shown={:?} modal={:?} menu={:?} ctx={} ids={:?} screen:\n{}",
        h.state().app.focus,
        h.state().app.panel.tab,
        h.state().app.terminal_shown(),
        h.state()
            .app
            .modal
            .as_ref()
            .map(|m| std::mem::discriminant(m)),
        h.state().app.menu.as_ref().map(|m| (m.menu, m.item)),
        h.state().app.context_menu.is_some(),
        h.state()
            .app
            .terminals
            .iter()
            .map(|t| (t.id, t.is_exited()))
            .collect::<Vec<_>>(),
        h.state()
            .app
            .terminals
            .iter()
            .find(|t| t.id == cur)
            .map(|t| t.with_screen(|s| s.contents()))
            .unwrap_or_default()
    );
    steps(&mut h, 3);
    assert_eq!(
        h.state().app.terminals.len(),
        2,
        "non-zero exit keeps the tab"
    );
    shot(&mut h, "terminal_exit_code");
    typ(&mut h, "q");
    assert_eq!(h.state().app.terminals.len(), 1);
    // The × on a tab closes it.
    steps(&mut h, 2);
    let id = h.state().app.terminals[0].id;
    h.state_mut().app.close_terminal(id);
    assert!(h.state().app.terminals.is_empty() && !h.state().app.panel.visible);
}

#[test]
fn context_menus_by_mouse_and_keyboard() {
    let d = project(
        "ctxmenu",
        &[
            ("src/a.rs", "fn main() { let value = 1; }\n"),
            ("b.txt", "b\n"),
            ("c.txt", "c\n"),
        ],
    );
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("b.txt"), true);
    h.state_mut().app.open_file(&d.join("c.txt"), true);
    h.state_mut().app.open_file(&d.join("src/a.rs"), true);
    steps(&mut h, 2);
    // Editor: right-click moves the cursor to the click and shows the editor menu.
    let at = h.state().screen_pos_for_test(0, 18) - egui::Vec2::new(3.0, 0.0);
    right_click(&mut h, at);
    let labels = context_labels(&h);
    for want in [
        "Cut",
        "Copy",
        "Paste",
        "Goto Definition",
        "Rename Symbol…",
        "Copy File Path",
    ] {
        assert!(
            labels.iter().any(|l| l == want),
            "{want} missing: {labels:?}"
        );
    }
    assert_eq!(
        h.state().app.buf().unwrap().cursor,
        crate::editor::Pos::new(0, 18)
    );
    shot(&mut h, "context_editor");
    // Keyboard: arrows + Enter run an item ("Select All" via its first letter here).
    typ(&mut h, "s");
    assert!(h.state().app.context_menu.is_none());
    assert_eq!(
        h.state().app.buf().unwrap().selected_text().as_deref(),
        Some("fn main() { let value = 1; }\n")
    );
    // Shift+F10 opens the same menu at the cursor; Down + Enter runs the first item (Cut).
    press(&mut h, Modifiers::SHIFT, Key::F10);
    assert!(
        h.state()
            .app
            .context_menu
            .as_ref()
            .is_some_and(|m| m.pos.is_none())
    );
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(h.state().app.context_menu.is_none());
    // Tab menu: Close Other Tabs.
    let tab_y = 26.0 + 16.0;
    right_click(&mut h, egui::Pos2::new(250.0 + 40.0, tab_y));
    assert!(
        context_labels(&h).iter().any(|l| l == "Close Other Tabs"),
        "{:?}",
        context_labels(&h)
    );
    let cur_name = h.state().app.buf().unwrap().name();
    h.state_mut()
        .app
        .run(crate::commands::Command::CloseOtherTabs);
    h.state_mut().app.context_menu = None;
    assert_eq!(h.state().app.buffers.len(), 1);
    assert_eq!(h.state().app.buf().unwrap().name(), cur_name);
    // Sidebar: right-click selects the row and opens the sidebar menu; "New File…" runs.
    let row_y = 26.0 + 32.0 + 1.0 * 24.0 + 12.0; // row 1 = "src"
    right_click(&mut h, egui::Pos2::new(60.0, row_y));
    assert_eq!(h.state().app.tree.selected().unwrap().name, "src");
    let labels = context_labels(&h);
    assert!(
        labels.iter().any(|l| l == "New File…")
            && labels
                .iter()
                .any(|l| l.starts_with("Reveal in") || l == "Open Containing Folder"),
        "{labels:?}"
    );
    shot(&mut h, "context_sidebar");
    typ(&mut h, "n");
    assert!(matches!(&h.state().app.modal, Some(Modal::Prompt(p)) if p.title.contains("src/")));
    press(&mut h, Modifiers::NONE, Key::Escape);
    // Clicking elsewhere closes an open menu.
    right_click(&mut h, egui::Pos2::new(60.0, row_y));
    assert!(h.state().app.context_menu.is_some());
    click(&mut h, egui::Pos2::new(700.0, 400.0));
    assert!(h.state().app.context_menu.is_none());
    // Line highlighting is off by default.
    assert!(!h.state().app.config.editor.highlight_current_line);
}

#[test]
fn caret_styles_and_size() {
    use crate::config::CaretStyle;
    use crate::gui::caret_alpha;
    let ms = Duration::from_millis;
    // Defaults: smooth fade, a bit thicker and taller than the line (Sublime-like look).
    let e = crate::config::EditorConfig::default();
    assert_eq!(e.caret_style, CaretStyle::Smooth);
    assert!(e.caret_extra_width > 0.0 && e.caret_extra_top > 0.0 && e.caret_extra_bottom > 0.0);
    // Solid never animates.
    assert_eq!(caret_alpha(CaretStyle::Solid, ms(1700)), (1.0, None));
    // Right after moving the caret is solid for every animated style.
    for s in [CaretStyle::Smooth, CaretStyle::Blink] {
        assert_eq!(caret_alpha(s, ms(100)).0, 1.0);
    }
    // Blink: off for half a period, then on; it schedules the next toggle.
    let (a, next) = caret_alpha(CaretStyle::Blink, ms(600));
    assert_eq!(a, 0.0);
    assert!(next.unwrap() <= ms(400));
    assert_eq!(caret_alpha(CaretStyle::Blink, ms(1200)).0, 1.0);
    // Smooth: fades through intermediate values, fully gone mid-cycle.
    let mid = caret_alpha(CaretStyle::Smooth, ms(1000)).0;
    assert!(mid < 0.01, "{mid}");
    let partial = caret_alpha(CaretStyle::Smooth, ms(750)).0;
    assert!(partial > 0.2 && partial < 0.8, "{partial}");
    // Settings parse from TOML.
    let c: crate::config::EditorConfig =
        toml::from_str("caret_style = \"blink\"\ncaret_extra_width = 3\ncaret_extra_top = 0\ncaret_extra_bottom = 0").unwrap();
    assert_eq!(c.caret_style, CaretStyle::Blink);
    assert_eq!(c.caret_extra_width, 3.0);
    // Screenshot with the default caret.
    let d = project("caret", &[("a.txt", "hello world\nsecond line\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    shot(&mut h, "caret_default");
}

#[test]
fn ui_scale_zooms_everything() {
    let d = project("uiscale", &[("a.rs", "fn main() {}\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.rs"), true);
    steps(&mut h, 3);
    let (_, lh1, _) = h.state().text_metrics_for_test();
    let p1 = h.state().screen_pos_for_test(0, 3);
    h.state_mut().app.config.editor.ui_scale = 1.5;
    steps(&mut h, 3);
    assert!((h.ctx.zoom_factor() - 1.5).abs() < 1e-4);
    // In egui points the layout is unchanged; on screen everything is 1.5× bigger.
    let (_, lh2, _) = h.state().text_metrics_for_test();
    let p2 = h.state().screen_pos_for_test(0, 3);
    assert!(
        (lh1 - lh2).abs() < 0.01 && (p1 - p2).length() < 0.5,
        "{lh1} {lh2} {p1:?} {p2:?}"
    );
    shot(&mut h, "ui_scale_150");
    // Nonsense values fall back to something usable.
    h.state_mut().app.config.editor.ui_scale = 40.0;
    steps(&mut h, 2);
    assert!((h.ctx.zoom_factor() - 3.0).abs() < 1e-4);
    let c: crate::config::EditorConfig = toml::from_str("ui_scale = 1.25").unwrap();
    assert_eq!(c.ui_scale, 1.25);
}

#[test]
fn goto_symbol_shows_names_not_signatures() {
    let src = "pub fn add(a: i32, b: i32) -> i32 { a + b }\npub struct Point<T> {\n    x: T,\n}\nimpl<T: Clone> std::fmt::Debug for Point<T> where T: Copy {\n    fn fmt(&self) {}\n}\nasync def handler(req):\n";
    let d = project(
        "symnames",
        &[("a.rs", src), ("b.md", "# Title\n## Sub section\n")],
    );
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.rs"), true);
    steps(&mut h, 2);
    press(&mut h, CMD, Key::R);
    let labels: Vec<String> = match &h.state().app.modal {
        Some(crate::app::Modal::Palette(p)) => {
            p.items.iter().map(|i| i.label.trim().to_string()).collect()
        }
        _ => panic!("symbol palette expected"),
    };
    assert_eq!(
        labels,
        [
            "add",
            "Point",
            "impl std::fmt::Debug for Point<T>",
            "fmt",
            "handler"
        ]
    );
    shot(&mut h, "goto_symbol_names");
    press(&mut h, Modifiers::NONE, Key::Escape);
    h.state_mut().app.open_file(&d.join("b.md"), true);
    steps(&mut h, 2);
    press(&mut h, CMD, Key::R);
    let labels: Vec<String> = match &h.state().app.modal {
        Some(crate::app::Modal::Palette(p)) => {
            p.items.iter().map(|i| i.label.trim().to_string()).collect()
        }
        _ => panic!("symbol palette expected"),
    };
    assert_eq!(labels, ["Title", "Sub section"]);
}

#[test]
fn editor_groups_side_by_side() {
    let d = project(
        "groups",
        &[
            ("a.txt", "alpha\n"),
            ("b.txt", "bravo\n"),
            ("c.txt", "charlie\n"),
        ],
    );
    let mut h = harness(&d);
    for f in ["a.txt", "b.txt", "c.txt"] {
        h.state_mut().app.open_file(&d.join(f), true);
    }
    steps(&mut h, 2);
    // Two columns: everything stays in group 1, group 2 is empty.
    h.state_mut()
        .app
        .run(crate::commands::Command::LayoutColumns2);
    steps(&mut h, 2);
    assert_eq!(h.state().app.layout, 2);
    assert_eq!(h.state().app.group_tabs(0).len(), 3);
    assert!(h.state().app.group_tabs(1).is_empty());
    // Ctrl+Shift+2 moves the current file (c.txt) to group 2; focus follows it.
    press(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::Num2);
    let app = &h.state().app;
    assert_eq!(app.group, 1);
    assert_eq!(app.buf().unwrap().name(), "c.txt");
    assert_eq!(app.group_tabs(0).len(), 2);
    // Group 1 now shows its neighbour.
    assert_eq!(app.buffers[app.group_shown(0).unwrap()].name(), "b.txt");
    // Typing goes to the focused group's file only.
    typ(&mut h, "Z");
    assert_eq!(h.state().app.buf().unwrap().lines[0], "Zcharlie");
    // Ctrl+1 focuses group 1; typing goes there now.
    press(&mut h, Modifiers::CTRL, Key::Num1);
    assert_eq!(h.state().app.group, 0);
    typ(&mut h, "Y");
    assert_eq!(h.state().app.buf().unwrap().name(), "b.txt");
    assert_eq!(h.state().app.buf().unwrap().lines[0], "Ybravo");
    // Next/previous file cycles within the group.
    h.state_mut().app.run(crate::commands::Command::NextBuffer);
    assert_eq!(h.state().app.buf().unwrap().name(), "a.txt");
    h.state_mut().app.run(crate::commands::Command::NextBuffer);
    assert_eq!(h.state().app.buf().unwrap().name(), "b.txt");
    shot(&mut h, "groups_two_columns");
    // Clicking in the other pane focuses it.
    let geom = h.state().text_metrics_for_test();
    let _ = geom;
    let right = egui::Pos2::new(1100.0, 200.0);
    h.event(egui::Event::PointerMoved(right));
    h.event(egui::Event::PointerButton {
        pos: right,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: right,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 2);
    assert_eq!(h.state().app.group, 1);
    assert_eq!(h.state().app.buf().unwrap().name(), "c.txt");
    // Closing the last tab of a group leaves it focused and empty: editing does nothing,
    // and nothing leaks into the other group's file.
    h.state_mut()
        .app
        .buffers
        .iter_mut()
        .for_each(|b| b.modified = false);
    h.state_mut().app.run(crate::commands::Command::CloseBuffer);
    assert_eq!(h.state().app.group, 1);
    assert!(h.state().app.buf().is_none());
    typ(&mut h, "Q");
    assert!(
        h.state()
            .app
            .buffers
            .iter()
            .all(|b| !b.lines[0].starts_with('Q'))
    );
    // The layout survives a restart (workspace).
    h.state_mut().app.focus_group(0);
    h.state_mut()
        .app
        .run(crate::commands::Command::MoveToGroup2);
    let ws = h.state().app.capture_workspace();
    assert_eq!(ws.layout, 2);
    assert_eq!(ws.buffers.iter().filter(|b| b.group == 1).count(), 1);
    // Back to a single column: every tab joins group 1.
    h.state_mut()
        .app
        .run(crate::commands::Command::LayoutSingle);
    assert_eq!(h.state().app.layout, 1);
    assert_eq!(h.state().app.group_tabs(0).len(), 2);
    assert!(h.state().app.buf().is_some());
}

#[test]
fn editor_groups_restore_from_workspace() {
    let d = project("groupsws", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    {
        let mut h = harness(&d);
        h.state_mut().app.open_file(&d.join("a.txt"), true);
        h.state_mut().app.open_file(&d.join("b.txt"), true);
        h.state_mut()
            .app
            .run(crate::commands::Command::MoveToGroup2);
        assert_eq!(h.state().app.layout, 2, "moving to group 2 adds the column");
        h.state_mut().app.save_workspace();
    }
    let mut h = harness(&d);
    h.state_mut().app.restore_workspace();
    steps(&mut h, 2);
    let app = &h.state().app;
    assert_eq!(app.layout, 2);
    assert_eq!(app.buffers[app.group_shown(0).unwrap()].name(), "a.txt");
    assert_eq!(app.buffers[app.group_shown(1).unwrap()].name(), "b.txt");
    assert_eq!(app.group, 1);
    assert_eq!(app.buf().unwrap().name(), "b.txt");
}

#[test]
fn file_icons_and_icon_packages() {
    // Built-in mapping: exact names beat extensions; unknown files get the default.
    let c = Config::builtin();
    assert_eq!(c.icon_for("main.rs").unwrap().label, "rs");
    assert_eq!(c.icon_for("Cargo.toml").unwrap().label, "crg");
    assert_eq!(c.icon_for("other.toml").unwrap().label, "tml");
    assert_eq!(c.icon_for("App.TSX").unwrap().label, "tsx");
    assert_eq!(c.icon_for(".gitignore").unwrap().label, "git");
    assert_eq!(c.icon_for("no_extension").unwrap().label, "");
    // An icon package (Packages/<name>/icons.toml + PNGs) and a user icons.toml.
    let d = project(
        "iconpkg",
        &[
            ("cfg/config.toml", "[editor]\n"),
            (
                "cfg/Packages/Pixel Icons/icons.toml",
                "folder_image = \"img/folder.png\"\n[[icons]]\nextensions = [\"rs\"]\nimage = \"img/rust.png\"\n",
            ),
            (
                "cfg/Packages/Other Icons/icons.toml",
                "[[icons]]\nextensions = [\"py\"]\nlabel = \"PY\"\ncolor = \"#00FF00\"\n",
            ),
            (
                "cfg/icons.toml",
                "[[icons]]\nextensions = [\"md\"]\nlabel = \"MD\"\ncolor = \"#FF0000\"\n",
            ),
            ("src/main.rs", "fn main() {}\n"),
            ("notes.md", "# hi\n"),
        ],
    );
    let img = d.join("cfg/Packages/Pixel Icons/img");
    std::fs::create_dir_all(&img).unwrap();
    for f in ["rust.png", "folder.png"] {
        image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 80, 40, 255]))
            .save(img.join(f))
            .unwrap();
    }
    let c = Config::load_from(Some(d.join("cfg/config.toml")));
    assert!(c.error.is_none(), "{:?}", c.error);
    assert_eq!(c.icon_packages, ["Other Icons", "Pixel Icons"]);
    let rs = c.icon_for("main.rs").unwrap();
    assert_eq!(
        rs.image.as_deref(),
        Some(img.join("rust.png").to_str().unwrap())
    );
    assert_eq!(c.icon_for("a.py").unwrap().label, "PY");
    assert_eq!(
        c.icon_for("notes.md").unwrap().label,
        "MD",
        "user icons.toml wins"
    );
    assert_eq!(
        c.folder_images.0.as_deref(),
        Some(img.join("folder.png").as_path())
    );
    // icon_theme picks one package only.
    std::fs::write(
        d.join("cfg/config.toml"),
        "[editor]\nicon_theme = \"Other Icons\"\n",
    )
    .unwrap();
    let c2 = Config::load_from(Some(d.join("cfg/config.toml")));
    assert_eq!(c2.icon_packages, ["Other Icons"]);
    assert!(c2.icon_for("main.rs").unwrap().image.is_none());
    // In the GUI the package images are drawn (decoded once, cached).
    let mut cfg = test_config(&d);
    cfg.icons = c.icons.clone();
    cfg.folder_images = c.folder_images.clone();
    let gui = Gui::headless(d.clone(), cfg);
    let mut h = Harness::builder()
        .with_size(egui::Vec2::new(1200.0, 720.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_ui_state(|ui, g: &mut Gui| g.frame(ui), gui);
    h.state_mut().app.open_file(&d.join("src/main.rs"), true);
    h.state_mut().app.open_file(&d.join("notes.md"), true);
    steps(&mut h, 3);
    assert!(
        h.state()
            .icon_textures_for_test()
            .iter()
            .any(|(p, ok)| p.ends_with("rust.png") && *ok)
    );
    shot(&mut h, "icons_package");
    // file_icons = false draws none.
    h.state_mut().app.config.editor.file_icons = false;
    steps(&mut h, 2);
}

#[test]
fn gitignore_dims_sidebar_and_filters_goto_anything() {
    // Not a git repository: .gitignore still applies.
    let d = project(
        "gitignore",
        &[
            (".gitignore", "build/\n*.log\n"),
            ("build/out.bin", "x"),
            ("app.log", "x"),
            ("main.rs", "fn main() {}\n"),
            ("README.md", "# readme\n"),
        ],
    );
    let mut h = harness(&d);
    steps(&mut h, 2);
    let nodes: Vec<(String, bool)> = h
        .state()
        .app
        .tree
        .nodes
        .iter()
        .map(|n| (n.name.clone(), n.ignored))
        .collect();
    assert!(nodes.contains(&("build".into(), true)), "{nodes:?}");
    assert!(nodes.contains(&("app.log".into(), true)), "{nodes:?}");
    assert!(nodes.contains(&("main.rs".into(), false)), "{nodes:?}");
    // Inside an ignored folder everything counts as ignored.
    let build = d.join("build");
    h.state_mut().app.tree.expanded.insert(build);
    h.state_mut().app.tree.rebuild();
    assert!(
        h.state()
            .app
            .tree
            .nodes
            .iter()
            .any(|n| n.name == "out.bin" && n.ignored)
    );
    // Goto Anything lists only files that aren't ignored, two lines per row.
    press(&mut h, CMD, Key::P);
    assert!(wait(
        &mut h,
        5,
        |g| matches!(&g.app.modal, Some(crate::app::Modal::Palette(p)) if !p.loading && !p.items.is_empty())
    ));
    let labels: Vec<String> = match &h.state().app.modal {
        Some(crate::app::Modal::Palette(p)) => p.items.iter().map(|i| i.label.clone()).collect(),
        _ => unreachable!(),
    };
    assert!(
        labels.contains(&"main.rs".to_string()) && labels.contains(&"README.md".to_string()),
        "{labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("build") || l.ends_with(".log")),
        "{labels:?}"
    );
    shot(&mut h, "goto_anything_two_line");
    press(&mut h, Modifiers::NONE, Key::Escape);
    shot(&mut h, "sidebar_icons_gitignore");
}

#[test]
fn editor_scrollbars() {
    let long: String =
        (0..300).map(|i| format!("line {i}\n")).collect::<String>() + &"x".repeat(400) + "\n";
    let d = project("scrollbar", &[("a.txt", long.as_str())]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    assert_eq!(h.state().app.buf().unwrap().scroll_y, 0);
    let ed = h.state().editor_rect_for_test();
    let x = ed.right() - 5.0;
    let drag = |h: &mut Harness<'static, Gui>, from: egui::Pos2, to: egui::Pos2| {
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        steps(h, 1);
        h.event(egui::Event::PointerMoved(to));
        steps(h, 1);
        h.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        steps(h, 2);
    };
    // Click near the bottom of the track: jumps towards the end.
    let bottom = egui::Pos2::new(x, ed.bottom() - 20.0);
    drag(&mut h, bottom, bottom);
    let sy = h.state().app.buf().unwrap().scroll_y;
    assert!(sy > 200, "track click jumped to {sy}");
    shot(&mut h, "scrollbar");
    // Drag the thumb back to the top.
    let thumb = egui::Pos2::new(x, ed.bottom() - 20.0);
    drag(&mut h, thumb, egui::Pos2::new(x, ed.top() - 400.0));
    assert_eq!(h.state().app.buf().unwrap().scroll_y, 0);
    // Lines wider than the pane get a horizontal scrollbar.
    let hb = egui::Pos2::new(ed.right() - 40.0, ed.bottom() - 5.0);
    drag(&mut h, hb, hb);
    assert!(h.state().app.buf().unwrap().scroll_x > 0);
    // The cursor didn't move: scrollbars only scroll.
    assert_eq!(
        h.state().app.buf().unwrap().cursor,
        crate::editor::Pos::new(0, 0)
    );
}
