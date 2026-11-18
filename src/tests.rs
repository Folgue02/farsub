//! End-to-end tests: drive the real GUI headlessly with egui_kittest (keys, text, clicks),
//! check app state and the filesystem, and render screenshots.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Event, Key, Modifiers};
use egui_kittest::Harness;

use crate::app::{Focus, Modal};
use crate::config::{Config, EditorConfig};
use crate::editor::Pos;
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
        // Most tests check columns on unwrapped lines; wrapping has its own tests.
        word_wrap: false,
        // Tests never download or compile anything unless they ask to.
        auto_install: false,
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

/// The platform's command key as a real keyboard sends it (Command on macOS, Ctrl elsewhere).
const CMD: Modifiers = if cfg!(target_os = "macos") {
    Modifiers::MAC_CMD.plus(Modifiers::COMMAND)
} else {
    Modifiers::COMMAND
};

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
    // Replace the whole input: Select All, then type (as in any text field).
    press(&mut h, CMD, Key::A);
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
    press(&mut h, CMD | Modifiers::SHIFT, Key::P);
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
    press(&mut h, CMD | Modifiers::SHIFT, Key::T);
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
    // Cmd+, opens the settings in a window of their own (another farsub process)...
    press(&mut h, CMD, Key::Comma);
    let spawned = h.state().app.spawned.clone();
    assert_eq!(spawned.len(), 1);
    assert_eq!(spawned[0][0], "--settings");
    assert!(
        spawned[0].iter().any(|a| a == "--config"),
        "same config: {spawned:?}"
    );
    assert!(
        h.state().app.buffers.is_empty(),
        "this window stays as it was"
    );
    // ...which shows them (this test plays that window now). It creates the config from
    // the template.
    h.state_mut().app.settings_window = true;
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

    // Enter runs the selected item (File's first: "New File"). A letter shared by several
    // items ("New File", "New Window") moves between them instead of running one.
    let n = h.state().app.buffers.len();
    press(&mut h, Modifiers::NONE, Key::F10);
    typ(&mut h, "n");
    assert_eq!(
        h.state().app.buffers.len(),
        n,
        "two items start with N: it selects, doesn't run"
    );
    typ(&mut h, "n");
    press(&mut h, Modifiers::NONE, Key::Enter);
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
    press(&mut h, CMD | Modifiers::SHIFT, Key::Slash);
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

    press(&mut h, CMD | Modifiers::ALT, Key::L);
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

#[test]
fn packages_add_themes_and_highlight_queries() {
    let theme = "name = \"Farsub Test Theme\"\n[editor]\nbackground = \"#102030\"\nforeground = \"#E0E0E0\"\n[syntax]\nkeyword = \"#FF8800\"\nstring = \"#00FF88\"\n";
    let d = project(
        "pkgs",
        &[
            ("Packages/TestTheme/themes/test.toml", theme),
            ("code.rs", "fn main() { let s = \"hi\"; }\n"),
        ],
    );
    let mut h = harness(&d);
    steps(&mut h, 2);
    assert_eq!(h.state().app.packages, ["TestTheme"]);
    assert!(
        h.state()
            .app
            .syn
            .theme_names()
            .iter()
            .any(|t| t == "Farsub Test Theme")
    );
    assert!(h.state_mut().app.syn.set_theme("Farsub Test Theme"));
    h.state_mut().app.open_file(&d.join("code.rs"), true);
    steps(&mut h, 3);
    let b = h.state().app.buf().unwrap();
    assert_eq!(
        (b.hl.syntax.as_str(), b.hl.grammar.as_deref()),
        ("rust", Some("rust"))
    );
    let orange = egui::Color32::from_rgb(0xFF, 0x88, 0x00);
    let spans = b.hl.line(0).unwrap().clone();
    assert!(
        spans.iter().any(|s| s.0 == 0 && s.2.fg == orange),
        "fn is a keyword: {spans:?}"
    );
    shot(&mut h, "package_theme");
    // A package can override a grammar's highlight query (here: everything is a string).
    let q = d.join("Packages/TestTheme/queries/rust");
    std::fs::create_dir_all(&q).unwrap();
    std::fs::write(q.join("highlights.scm"), "(identifier) @string\n").unwrap();
    h.state_mut()
        .app
        .run(crate::commands::Command::ReloadPackages);
    h.state_mut().app.syn.set_theme("Farsub Test Theme");
    steps(&mut h, 3);
    let spans = h.state().app.buf().unwrap().hl.line(0).unwrap().clone();
    let green = egui::Color32::from_rgb(0x00, 0xFF, 0x88);
    assert!(
        spans.iter().any(|s| s.2.fg == green) && !spans.iter().any(|s| s.2.fg == orange),
        "{spans:?}"
    );
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
    press(&mut h, CMD | Modifiers::ALT, Key::P);
    assert!(matches!(&h.state().app.modal, Some(Modal::Palette(p)) if p.items.len() >= 2));
    typ(&mut h, "one");
    shot(&mut h, "switch_project");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().name(), "x.txt");

    // Save Project As writes a project file with a relative folder; Add Folder updates it.
    // (The system's save dialog is answered by the test hook.)
    h.state_mut().app.test_pick = Some(d.join("one/One"));
    h.state_mut()
        .app
        .run(crate::commands::Command::SaveProjectAs);
    steps(&mut h, 2);
    let file = d.join("one/One.farsub-project");
    assert!(file.is_file(), "{:?}", h.state().app.status);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("path = \".\"")
    );
    assert!(d.join("one/One.farsub-workspace").is_file());
    h.state_mut().app.test_pick = Some(d.join("extra"));
    h.state_mut().app.run(crate::commands::Command::AddFolder);
    steps(&mut h, 2);
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
        b.add_next_occurrence(); // "hello"
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
    typ(&mut h, "bash");
    shot(&mut h, "syntax_picker");
    press(&mut h, Modifiers::NONE, Key::Enter);
    let b = h.state().app.buf().unwrap();
    assert_eq!(
        (b.hl.syntax.as_str(), b.hl.grammar.as_deref()),
        ("bash", Some("bash"))
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
    assert_eq!(h2.state().app.buf().unwrap().hl.syntax, "bash");
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
    press(&mut h, CMD | Modifiers::ALT, Key::I);
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
    press(&mut h, CMD | Modifiers::SHIFT, Key::P);
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
    assert_eq!(h.state().app.console.input.text(), "no_such_thing");
    press(&mut h, CMD, Key::A); // select the input; typing replaces it
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

#[test]
#[ignore]
/// Frame-time benchmark (run: cargo test --release -- --ignored bench_frame_cost --nocapture).
fn bench_frame_cost() {
    // 200k lines of valid Rust: 10,000 functions of 20 lines.
    let big: String = (0..10_000)
        .map(|f| {
            let body: String = (0..18)
                .map(|i| format!("    let value_{i} = compute({i}, \"some string\");\n"))
                .collect();
            format!("fn f{f}() {{\n{body}}}\n")
        })
        .collect();
    let d = project("perfprobe", &[("big.rs", big.as_str())]);
    // A real ~6k-line Rust file: the app modules glued together.
    let mut real = String::new();
    let mut parts: Vec<_> = std::fs::read_dir("src/app")
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    parts.sort();
    for p in parts {
        real += &std::fs::read_to_string(p).unwrap();
    }
    std::fs::write(d.join("app.rs"), real).unwrap();
    let t = Instant::now();
    let mut h = harness(&d);
    h.step();
    eprintln!(
        "startup (app + first frame): {:.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = Instant::now();
    h.state_mut().app.open_file(&d.join("big.rs"), true);
    eprintln!(
        "open big.rs (200k lines): {:.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let t = Instant::now();
    h.step();
    eprintln!(
        "  its first frame: {:.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    h.state_mut().app.run(crate::commands::Command::CloseBuffer);
    h.step();
    for (name, f) in [
        ("app.rs (6.4k lines)", "app.rs"),
        ("big.rs (200k lines)", "big.rs"),
    ] {
        h.state_mut().app.open_file(&d.join(f), true);
        // let highlighting of the visible part settle
        for _ in 0..30 {
            h.step();
        }
        let t = Instant::now();
        let n = 100;
        for _ in 0..n {
            h.step();
        }
        eprintln!(
            "{name}: idle frame {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.0 / n as f64
        );
        let t = Instant::now();
        for _ in 0..n {
            h.event(egui::Event::Text("x".into()));
            h.step();
        }
        eprintln!(
            "{name}: typing frame {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.0 / n as f64
        );
        let t = Instant::now();
        for _ in 0..n {
            h.key_press(Key::PageDown);
            h.step();
        }
        eprintln!(
            "{name}: page-down frame {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.0 / n as f64
        );
    }
    let time = |h: &mut Harness<'static, Gui>,
                name: &str,
                n: usize,
                f: &dyn Fn(&mut Harness<'static, Gui>)| {
        let t = Instant::now();
        for _ in 0..n {
            f(h);
            h.step();
        }
        eprintln!(
            "{name}: {:.2} ms",
            t.elapsed().as_secs_f64() * 1000.0 / n as f64
        );
    };
    // Word wrap on (the default) on the big file.
    h.state_mut().app.buf_mut().unwrap().word_wrap = Some(true);
    for _ in 0..10 {
        h.step();
    }
    time(&mut h, "big.rs wrapped: idle frame", 100, &|_| {});
    time(&mut h, "big.rs wrapped: typing frame", 100, &|h| {
        h.event(egui::Event::Text("x".into()))
    });
    time(&mut h, "big.rs wrapped: page-down frame", 100, &|h| {
        h.key_press(Key::PageDown)
    });
    // The find panel open, with matches on every line.
    h.state_mut().app.run(crate::commands::Command::Find);
    h.event(egui::Event::Text("value".into()));
    for _ in 0..5 {
        h.step();
    }
    time(&mut h, "find panel open: idle frame", 100, &|_| {});
    time(&mut h, "find panel open: Enter (next match)", 100, &|h| {
        h.key_press(Key::Enter)
    });
    h.key_press(Key::Escape);
    h.state_mut().app.run(crate::commands::Command::Find);
    h.step();
    h.key_press(Key::Escape);
    // Typing in the file while the panel stays open (matches recounted).
    h.state_mut().app.run(crate::commands::Command::Find);
    h.step();
    h.state_mut().app.focus = crate::app::Focus::Editor;
    time(&mut h, "typing with the find panel open", 50, &|h| {
        h.event(egui::Event::Text("x".into()))
    });
    h.key_press(Key::Escape);
    // A sidebar with thousands of files, all folders open.
    let many = project("perfsidebar", &[]);
    for a in 0..40 {
        for b in 0..100 {
            let f = many.join(format!("dir{a:02}/file{b:03}.rs"));
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "").unwrap();
        }
    }
    let mut hs = harness(&many);
    for a in 0..40 {
        hs.state_mut()
            .app
            .tree
            .expanded
            .insert(many.join(format!("dir{a:02}")));
    }
    hs.state_mut().app.refresh_tree_for_test();
    for _ in 0..10 {
        hs.step();
    }
    eprintln!("sidebar rows: {}", hs.state().app.tree.nodes.len());
    time(&mut hs, "sidebar 4k files: idle frame", 100, &|_| {});
    hs.state_mut().app.focus = crate::app::Focus::Sidebar;
    time(&mut hs, "sidebar 4k files: arrow-down frame", 100, &|h| {
        h.key_press(Key::ArrowDown)
    });
    h.state_mut()
        .app
        .run(crate::commands::Command::ManageServers);
    h.step();
    let t = Instant::now();
    for _ in 0..20 {
        h.step();
    }
    eprintln!(
        "server manager ({} servers): frame {:.2} ms",
        h.state().app.config.servers.len(),
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );
}

#[test]
fn word_wrap_rows_movement_and_toggle() {
    let long = format!(
        "    {}\nshort\n",
        (0..60)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let d = project("wrap", &[("a.txt", long.as_str())]);
    let mut cfg = test_config(&d);
    cfg.editor.word_wrap = true;
    let mut h = project_harness(crate::project::Project::from_folders(vec![d.clone()]), cfg);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    let rows = h.state().rows_for_test();
    let line0: Vec<_> = rows.iter().filter(|r| r.0 == 0).collect();
    assert!(line0.len() >= 3, "long line wraps into rows: {rows:?}");
    assert_eq!(rows.iter().filter(|r| r.0 == 1).count(), 1);
    let width = h.state().app.wrap_cols.expect("wrapping");
    assert!(
        line0.iter().all(|r| r.2 - r.1 <= width),
        "{line0:?} width {width}"
    );
    shot(&mut h, "word_wrap");
    // Down moves one visual row within the long line, then Up comes back.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(0, 8));
    steps(&mut h, 1);
    press(&mut h, Modifiers::NONE, Key::ArrowDown);
    let c = h.state().app.buf().unwrap().cursor;
    assert_eq!(c.line, 0, "still on the wrapped line");
    assert!(
        c.col > line0[1].1.saturating_sub(4) && c.col < line0[1].2,
        "on its second row: {c:?} {line0:?}"
    );
    press(&mut h, Modifiers::NONE, Key::ArrowUp);
    assert_eq!(
        h.state().app.buf().unwrap().cursor,
        crate::editor::Pos::new(0, 8)
    );
    // Clicking the second row puts the caret on that row.
    let (_, start, end, y) = *line0[1];
    let (cw, _, _) = h.state().text_metrics_for_test();
    let ed = h.state().editor_rect_for_test();
    let x = ed.left() + (ed.width() * 0.3).min(400.0);
    h.event(egui::Event::PointerMoved(egui::Pos2::new(x, y + 5.0)));
    h.event(egui::Event::PointerButton {
        pos: egui::Pos2::new(x, y + 5.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: egui::Pos2::new(x, y + 5.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    steps(&mut h, 2);
    let c = h.state().app.buf().unwrap().cursor;
    assert!(
        c.line == 0 && c.col >= start && c.col < end,
        "click on row 2 → {c:?} (row {start}..{end}, cw {cw})"
    );
    // Alt+Z turns wrapping off for this file: one row per line, horizontal scrolling back.
    press(&mut h, Modifiers::ALT, Key::Z);
    steps(&mut h, 2);
    assert_eq!(h.state().app.buf().unwrap().word_wrap, Some(false));
    assert!(h.state().app.wrap_cols.is_none());
    assert_eq!(
        h.state()
            .rows_for_test()
            .iter()
            .filter(|r| r.0 == 0)
            .count(),
        1
    );
    // ...and it's remembered in the workspace.
    let ws = h.state().app.capture_workspace();
    assert_eq!(ws.buffers[0].word_wrap, Some(false));
}

#[test]
fn input_boxes_edit_like_the_editor() {
    let d = project("inputs", &[("a.txt", "some text\n")]);
    let mut h = harness(&d);
    // Word movement is Option on macOS, Ctrl elsewhere (as in the editor).
    let word = if cfg!(target_os = "macos") {
        Modifiers::ALT
    } else {
        Modifiers::CTRL
    };
    let prompt_text = |h: &Harness<'static, Gui>| {
        let app = &h.state().app;
        if app.find.open && app.focus == Focus::Find {
            return app.find.find.text().to_string();
        }
        match &app.modal {
            Some(Modal::Prompt(p)) => p.input.text().to_string(),
            Some(Modal::Palette(p)) => p.query.text().to_string(),
            _ => panic!("no input open"),
        }
    };
    // Goto Anything with no file open: Select All works (from the key and from the menu).
    press(&mut h, CMD, Key::P);
    typ(&mut h, "abc");
    assert!(h.state().app.enabled(crate::commands::Command::SelectAll));
    press(&mut h, CMD, Key::A);
    typ(&mut h, "z");
    assert_eq!(prompt_text(&h), "z");
    h.state_mut().app.on_menu_command("select_all");
    typ(&mut h, "q");
    assert_eq!(
        prompt_text(&h),
        "q",
        "the menu's Select All also targets the input"
    );
    press(&mut h, Modifiers::NONE, Key::Escape);
    // The find panel: word movement, delete word, select all, undo — editor bindings.
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    press(&mut h, CMD, Key::F);
    typ(&mut h, "hello big world");
    press(&mut h, word, Key::ArrowLeft);
    assert_eq!(h.state().app.find.find.cursor(), 10);
    press(&mut h, word, Key::Backspace);
    assert_eq!(prompt_text(&h), "hello world");
    press(&mut h, Modifiers::SHIFT, Key::End);
    press(&mut h, CMD, Key::C);
    press(&mut h, CMD, Key::Z);
    assert_eq!(prompt_text(&h), "hello big world", "undo in the input");
    press(&mut h, CMD, Key::A);
    typ(&mut h, "text");
    assert_eq!(prompt_text(&h), "text");
    // The editor's own text and selection weren't touched by any of this.
    assert_eq!(h.state().app.buf().unwrap().lines[0], "some text");
    shot(&mut h, "find_panel_input");
    press(&mut h, Modifiers::NONE, Key::Escape);
    // The console line too.
    h.state_mut()
        .app
        .run(crate::commands::Command::ToggleConsole);
    steps(&mut h, 2);
    typ(&mut h, "filter one two");
    press(&mut h, word, Key::Backspace);
    assert_eq!(h.state().app.console.input.text(), "filter one ");
}

#[test]
fn smooth_scrolling_by_pixels() {
    let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let d = project("smooth", &[("a.txt", text.as_str())]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    let ed = h.state().editor_rect_for_test();
    let over = egui::Pos2::new(ed.center().x, ed.center().y);
    let wheel = |h: &mut Harness<'static, Gui>, dy: f32| {
        h.event(egui::Event::PointerMoved(over));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::Vec2::new(0.0, dy),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
        });
        for _ in 0..30 {
            h.step();
        }
    };
    let (_, lh, _) = h.state().text_metrics_for_test();
    // A small trackpad nudge scrolls a few pixels, not a whole line.
    wheel(&mut h, -6.0);
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.scroll_y, 0);
    assert!(
        b.scroll_px > 0.5 && b.scroll_px < lh,
        "px {} lh {lh}",
        b.scroll_px
    );
    let first_row_y = h.state().rows_for_test()[0].3;
    assert!(
        first_row_y < ed.top(),
        "rows shift by the pixel offset: {first_row_y} vs {}",
        ed.top()
    );
    // A bigger scroll carries whole lines into the line position.
    wheel(&mut h, -10.0 * lh);
    let b = h.state().app.buf().unwrap();
    assert!(b.scroll_y >= 8, "scrolled {} lines", b.scroll_y);
    // Scrolling back up past the top stops cleanly at 0.
    wheel(&mut h, 100.0 * lh);
    let b = h.state().app.buf().unwrap();
    assert_eq!((b.scroll_y, b.scroll_px), (0, 0.0));
    // Past the bottom it stops at the last line, with no partial row to bounce back from.
    wheel(&mut h, -300.0 * lh);
    let b = h.state().app.buf().unwrap();
    assert_eq!((b.scroll_y, b.scroll_px), (b.lines.len() - 1, 0.0));
    for _ in 0..3 {
        wheel(&mut h, -0.4 * lh);
        let b = h.state().app.buf().unwrap();
        assert_eq!((b.scroll_y, b.scroll_px), (b.lines.len() - 1, 0.0));
    }
    wheel(&mut h, 100.0 * lh);
    // Moving the cursor snaps to whole lines again.
    wheel(&mut h, -5.0);
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(150, 0));
    steps(&mut h, 2);
    assert_eq!(h.state().app.buf().unwrap().scroll_px, 0.0);
}

#[test]
fn horizontal_wheel_scrolling_keeps_fractions_and_stops_at_the_widest_line() {
    let text = format!("{}\nshort\n", "x".repeat(300));
    let d = project("hwheel", &[("a.txt", text.as_str())]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    let ed = h.state().editor_rect_for_test();
    let (cw, _, _) = h.state().text_metrics_for_test();
    let wheel = |h: &mut Harness<'static, Gui>, dx: f32| {
        h.event(egui::Event::PointerMoved(ed.center()));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::Vec2::new(dx, 0.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
        });
        for _ in 0..30 {
            h.step();
        }
    };
    // Slow swipes (a fraction of a column per frame) still add up.
    for _ in 0..8 {
        wheel(&mut h, -0.5 * cw);
    }
    let sx = h.state().app.buf().unwrap().scroll_x;
    assert!((2..=6).contains(&sx), "scrolled {sx} columns");
    // A huge swipe stops with the end of the widest line in view.
    wheel(&mut h, -1000.0 * cw);
    let sx = h.state().app.buf().unwrap().scroll_x;
    assert!(sx > 100 && sx < 300, "stopped at {sx}");
    wheel(&mut h, -100.0 * cw);
    assert_eq!(h.state().app.buf().unwrap().scroll_x, sx);
}

#[test]
fn install_a_grammar_from_the_palette() {
    // Compile Prisma's grammar from the local cargo registry (no network); skip if absent.
    let home = std::env::var("HOME").unwrap_or_default();
    let reg = PathBuf::from(&home).join(".cargo/registry/src");
    let src = std::fs::read_dir(&reg)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("tree-sitter-prisma-0.2.2"))
        .find(|p| p.is_dir());
    let Some(src) = src else {
        eprintln!("tree-sitter-prisma source not in the cargo registry; skipping");
        return;
    };
    let d = project(
        "grammarinst",
        &[("schema.prisma", "model User {\n  id Int @id\n}\n")],
    );
    let mut cfg = test_config(&d);
    let mut r = cfg.grammar_recipe("prisma").unwrap().clone();
    r.krate = None;
    r.version = None;
    r.source = Some(src.display().to_string());
    cfg.grammars.retain(|g| g.name != "prisma");
    cfg.grammars.push(r);
    let mut h = project_harness(crate::project::Project::from_folders(vec![d.clone()]), cfg);
    h.state_mut().app.open_file(&d.join("schema.prisma"), true);
    steps(&mut h, 2);
    let b = h.state().app.buf().unwrap();
    assert_eq!(
        (b.hl.syntax.as_str(), b.hl.grammar.as_deref()),
        ("prisma", None),
        "not installed yet"
    );
    assert!(
        !crate::log::entries(crate::log::Level::Info, "needs its grammar").is_empty(),
        "the console says how to get the grammar"
    );
    // Grammars: Install Grammar… → pick Prisma.
    h.state_mut()
        .app
        .run(crate::commands::Command::InstallGrammar);
    typ(&mut h, "prisma");
    shot(&mut h, "install_grammar_palette");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        wait(&mut h, 60, |g| g.app.buf().is_some_and(|b| b
            .hl
            .grammar
            .as_deref()
            == Some("prisma"))),
        "installed and attached: {:?}",
        h.state().app.status
    );
    steps(&mut h, 3);
    let spans = h
        .state()
        .app
        .buf()
        .unwrap()
        .hl
        .line(0)
        .cloned()
        .unwrap_or_default();
    assert!(!spans.is_empty(), "highlighted after install");
    shot(&mut h, "installed_grammar");
    // Remove it again: back to plain text.
    h.state_mut()
        .app
        .run(crate::commands::Command::RemoveGrammar);
    press(&mut h, Modifiers::NONE, Key::Enter);
    steps(&mut h, 2);
    assert_eq!(h.state().app.buf().unwrap().hl.grammar, None);
}

#[test]
fn opening_a_file_installs_its_grammar_automatically() {
    let home = std::env::var("HOME").unwrap_or_default();
    let src = std::fs::read_dir(PathBuf::from(&home).join(".cargo/registry/src"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("tree-sitter-prisma-0.2.2"))
        .find(|p| p.is_dir());
    let Some(src) = src else {
        eprintln!("tree-sitter-prisma source not in the cargo registry; skipping");
        return;
    };
    let d = project(
        "autogrammar",
        &[("schema.prisma", "model Post {\n  id Int @id\n}\n")],
    );
    let mut cfg = test_config(&d);
    cfg.editor.auto_install = true;
    // Language servers would be fetched from the network: keep prisma's out of this test.
    if let Some(l) = cfg.languages.iter_mut().find(|l| l.name == "prisma") {
        l.servers.clear();
    }
    let mut r = cfg.grammar_recipe("prisma").unwrap().clone();
    (r.krate, r.version, r.source) = (None, None, Some(src.display().to_string()));
    cfg.grammars.retain(|g| g.name != "prisma");
    cfg.grammars.push(r);
    let mut h = project_harness(crate::project::Project::from_folders(vec![d.clone()]), cfg);
    // Just open the file: nothing else.
    h.state_mut().app.open_file(&d.join("schema.prisma"), true);
    assert!(
        wait(&mut h, 60, |g| g.app.buf().is_some_and(|b| b
            .hl
            .grammar
            .as_deref()
            == Some("prisma"))),
        "grammar installed on open: {:?}",
        h.state().app.status
    );
    steps(&mut h, 3);
    assert!(
        !h.state()
            .app
            .buf()
            .unwrap()
            .hl
            .line(0)
            .cloned()
            .unwrap_or_default()
            .is_empty()
    );
}

#[test]
fn install_a_grammar_from_a_folder() {
    let home = std::env::var("HOME").unwrap_or_default();
    let src = std::fs::read_dir(PathBuf::from(&home).join(".cargo/registry/src"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("tree-sitter-vue-0.0.3"))
        .find(|p| p.is_dir());
    let Some(src) = src else {
        eprintln!("tree-sitter-vue source not in the cargo registry; skipping");
        return;
    };
    let d = project(
        "grammarfolder",
        &[("App.vue", "<template><p>{{ msg }}</p></template>\n")],
    );
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("App.vue"), true);
    steps(&mut h, 2);
    assert_eq!(h.state().app.buf().unwrap().hl.grammar, None);
    // What "Grammars: Install Grammar from Folder…" does with the picked folder.
    h.state_mut().app.install_grammar_from(src);
    assert!(
        wait(&mut h, 60, |g| g.app.buf().is_some_and(|b| b
            .hl
            .grammar
            .as_deref()
            == Some("vue"))),
        "{:?}",
        h.state().app.status
    );
    assert!(
        h.state()
            .app
            .status
            .as_ref()
            .is_some_and(|s| s.0.contains("Installed the vue grammar"))
    );
}

#[test]
fn deleted_files_are_flagged_and_closing_warns() {
    let d = project("deleted", &[("a.txt", "keep me\n"), ("b.txt", "other\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("b.txt"), true);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    std::fs::remove_file(d.join("a.txt")).unwrap();
    assert!(
        wait(&mut h, 5, |g| g.app.buf().is_some_and(|b| b.deleted)),
        "noticed"
    );
    let app = &h.state().app;
    let b = app.buf().unwrap();
    assert!(b.modified, "counts as unsaved (hot exit keeps it)");
    assert!(app.title().contains("a.txt (deleted)"), "{}", app.title());
    assert_eq!(b.lines[0], "keep me", "contents kept");
    assert!(
        !app.tree.nodes.iter().any(|n| n.name == "a.txt"),
        "the sidebar no longer lists it"
    );
    steps(&mut h, 2);
    shot(&mut h, "deleted_file");
    // Closing warns that the contents will be lost; "n" keeps it open.
    h.state_mut().app.run(crate::commands::Command::CloseBuffer);
    match &h.state().app.modal {
        Some(Modal::Confirm(c)) => assert!(
            c.message.contains("no longer exists") && c.message.contains("lose"),
            "{}",
            c.message
        ),
        _ => panic!("expected the close warning"),
    }
    typ(&mut h, "n");
    assert_eq!(h.state().app.buffers.len(), 2);
    // Saving recreates the file and clears the flag.
    press(&mut h, CMD, Key::S);
    assert_eq!(
        std::fs::read_to_string(d.join("a.txt")).unwrap(),
        "keep me\n"
    );
    assert!(!h.state().app.buf().unwrap().deleted);
    // Delete again, close and confirm: gone.
    std::fs::remove_file(d.join("a.txt")).unwrap();
    assert!(wait(&mut h, 5, |g| g.app.buf().is_some_and(|b| b.deleted)));
    h.state_mut().app.run(crate::commands::Command::CloseBuffer);
    typ(&mut h, "y");
    assert_eq!(h.state().app.buffers.len(), 1);
    assert_eq!(h.state().app.buf().unwrap().name(), "b.txt");
}

#[test]
fn files_changed_on_disk_reload_or_ask() {
    let d = project("changed", &[("a.txt", "one\ntwo\nthree\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(2, 3));
    steps(&mut h, 2);
    // Unedited: reloads in place, the cursor stays, and undo brings the old text back.
    std::fs::write(d.join("a.txt"), "one\nTWO, changed elsewhere\nthree\n").unwrap();
    assert!(
        wait(&mut h, 5, |g| g
            .app
            .buf()
            .is_some_and(|b| b.lines[1].starts_with("TWO"))),
        "reloaded"
    );
    let b = h.state().app.buf().unwrap();
    assert!(!b.modified);
    assert_eq!(b.cursor, crate::editor::Pos::new(2, 3), "cursor kept");
    h.state_mut().app.buf_mut().unwrap().undo();
    assert_eq!(h.state().app.buf().unwrap().lines[1], "two");
    h.state_mut().app.buf_mut().unwrap().redo();
    // Edited here too: asks first; "n" keeps our edits.
    h.state_mut().app.buf_mut().unwrap().insert_text("mine ");
    std::fs::write(d.join("a.txt"), "theirs\n").unwrap();
    assert!(wait(&mut h, 5, |g| matches!(
        g.app.modal,
        Some(Modal::Confirm(_))
    )));
    match &h.state().app.modal {
        Some(Modal::Confirm(c)) => assert!(c.message.contains("changed on disk"), "{}", c.message),
        _ => unreachable!(),
    }
    typ(&mut h, "n");
    steps(&mut h, 80);
    assert!(h.state().app.modal.is_none(), "asked once per change");
    assert!(h.state().app.buf().unwrap().lines[2].contains("mine "));
    // Another outside change, then Save: asks before overwriting it.
    std::fs::write(d.join("a.txt"), "theirs again, longer\n").unwrap();
    press(&mut h, CMD, Key::S);
    match &h.state().app.modal {
        Some(Modal::Confirm(c)) => assert!(c.message.contains("Overwrite"), "{}", c.message),
        _ => panic!("expected the overwrite question"),
    }
    typ(&mut h, "n");
    assert_eq!(
        std::fs::read_to_string(d.join("a.txt")).unwrap(),
        "theirs again, longer\n",
        "not overwritten"
    );
    // Revert File takes the disk version.
    h.state_mut().app.run(crate::commands::Command::RevertFile);
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.lines[0], "theirs again, longer");
    assert!(!b.modified);
    // Our own saves don't count as outside changes.
    h.state_mut().app.buf_mut().unwrap().insert_text("x");
    press(&mut h, CMD, Key::S);
    assert!(h.state().app.modal.is_none());
    steps(&mut h, 80);
    assert!(h.state().app.modal.is_none());
}

#[test]
fn legacy_encoded_files_keep_their_encoding() {
    let d = tmpdir("latin1");
    let raw = b"caf\xE9 cr\xE8me br\xFBl\xE9e, \xE0 la fran\xE7aise\n";
    std::fs::write(d.join("menu.txt"), raw).unwrap();
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("menu.txt"), true);
    steps(&mut h, 2);
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.lines[0], "café crème brûlée, à la française");
    assert!(!b.encoding.is_utf8());
    // An edit the encoding can hold saves byte for byte in it.
    h.state_mut()
        .app
        .buf_mut()
        .unwrap()
        .goto(crate::editor::Pos::new(0, 0));
    typ(&mut h, "é");
    press(&mut h, CMD, Key::S);
    let mut want = b"\xE9".to_vec();
    want.extend_from_slice(raw);
    assert_eq!(std::fs::read(d.join("menu.txt")).unwrap(), want);
    // One it can't hold is refused, and the file is left alone.
    typ(&mut h, "Ω");
    press(&mut h, CMD, Key::S);
    assert_eq!(std::fs::read(d.join("menu.txt")).unwrap(), want);
    let status = h.state().app.status.clone().unwrap().0;
    assert!(status.contains("Convert to UTF-8"), "{status}");
    assert!(h.state().app.buf().unwrap().modified);
    // Converting saves it as UTF-8.
    h.state_mut()
        .app
        .run(crate::commands::Command::ConvertToUtf8);
    press(&mut h, CMD, Key::S);
    assert_eq!(
        std::fs::read_to_string(d.join("menu.txt")).unwrap(),
        "éΩcafé crème brûlée, à la française\n"
    );
}

#[test]
fn new_window_is_a_clean_window() {
    let d = project("newwin", &[("a.txt", "x\n")]);
    let h = harness(&d);
    let args: Vec<String> = h
        .state()
        .app
        .new_window_args()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "--empty");
    assert!(
        args.windows(2).any(|w| w[0] == "--config"),
        "same config as this window: {args:?}"
    );
    assert!(
        h.state()
            .app
            .keymap
            .keys_for(crate::commands::Command::NewWindow)
            .iter()
            .any(|k| k.contains('N'))
    );
    // What the new window looks like: no folder, no session.
    let mut e = project_harness(
        crate::project::Project::from_folders(vec![]),
        test_config(&d),
    );
    steps(&mut e, 2);
    assert!(e.state().app.project.folders.is_empty() && e.state().app.buffers.is_empty());
    shot(&mut e, "empty_window");
}

#[test]
fn find_panel_options_count_and_replace() {
    use crate::commands::Command;
    let d = project("findpanel", &[("a.txt", "foo Foo foo\nbar foo_x foo\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    press(&mut h, CMD, Key::F);
    assert!(h.state().app.find.open);
    assert_eq!(h.state().app.focus, Focus::Find);
    typ(&mut h, "foo");
    // Searching as you type selects the first match; "1 of 5" (case-insensitive).
    let sel = |h: &Harness<'static, Gui>| h.state().app.buf().unwrap().selection();
    assert_eq!(sel(&h), Some((Pos::new(0, 0), Pos::new(0, 3))));
    assert_eq!(h.state_mut().app.find_count(), Some((Some(0), 5)));
    h.state_mut().app.run(Command::ToggleFindCase);
    assert_eq!(h.state_mut().app.find_count(), Some((Some(0), 4)));
    h.state_mut().app.run(Command::ToggleFindWord);
    assert_eq!(
        h.state_mut().app.find_count(),
        Some((Some(0), 3)),
        "foo_x isn't a whole word"
    );
    // Enter / Shift+Enter step through the matches, wrapping around.
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(sel(&h), Some((Pos::new(0, 8), Pos::new(0, 11))));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(sel(&h), Some((Pos::new(1, 10), Pos::new(1, 13))));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(sel(&h), Some((Pos::new(0, 0), Pos::new(0, 3))), "wrapped");
    press(&mut h, Modifiers::SHIFT, Key::Enter);
    assert_eq!(sel(&h), Some((Pos::new(1, 10), Pos::new(1, 13))));
    shot(&mut h, "find_panel");
    // Replace: Tab to the Replace field, Enter replaces the selected match and moves on.
    h.state_mut().app.run(Command::Replace);
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Tab);
    typ(&mut h, "X");
    press(&mut h, Modifiers::NONE, Key::Enter);
    let lines = |h: &Harness<'static, Gui>| h.state().app.buf().unwrap().lines.clone();
    assert_eq!(
        lines(&h)[1],
        "bar foo_x X",
        "the selected match was replaced"
    );
    assert_eq!(
        sel(&h),
        Some((Pos::new(0, 0), Pos::new(0, 3))),
        "and the next one selected"
    );
    h.state_mut().app.run(Command::ReplaceAll);
    assert_eq!(lines(&h)[..2], ["X Foo X", "bar foo_x X"]);
    steps(&mut h, 2);
    shot(&mut h, "replace_panel");
    // A bad regex is reported in the panel, not applied.
    h.state_mut().app.run(Command::ToggleFindRegex);
    press(&mut h, Modifiers::SHIFT, Key::Tab);
    press(&mut h, Modifiers::NONE, Key::End);
    typ(&mut h, "\\");
    assert!(
        h.state()
            .app
            .find
            .error
            .as_ref()
            .is_some_and(|e| e.1.starts_with("Invalid regex")),
        "{:?}",
        h.state().app.find.error
    );
    // Esc closes the panel; the editor has focus again and nothing else changed.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!h.state().app.find.open);
    assert_eq!(h.state().app.focus, Focus::Editor);
    // The options are kept in the workspace.
    let ws = h.state().app.capture_workspace();
    assert!(ws.find_options.case && ws.find_options.word && ws.find_options.regex);
    assert_eq!(ws.find_replace, "X");
}

#[test]
fn find_in_files_results_open_and_replace() {
    use crate::commands::Command;
    let d = project(
        "fif",
        &[
            ("src/a.rs", "fn needle() {}\n// a needle\n"),
            ("src/b.txt", "nothing here\n"),
            ("skip/c.rs", "needle\n"),
            (".gitignore", "skip/\n"),
        ],
    );
    let mut h = harness(&d);
    steps(&mut h, 2);
    // No file open: Find in Files still works.
    press(&mut h, CMD.plus(Modifiers::SHIFT), Key::F);
    assert_eq!(h.state().app.find.mode, crate::app::FindMode::Files);
    typ(&mut h, "needle");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(wait(&mut h, 10, |g| g.app.find_in_files_done()));
    steps(&mut h, 2);
    let text = h.state().app.buf().unwrap().text();
    assert_eq!(h.state().app.buf().unwrap().name(), "Find Results");
    assert!(
        text.contains(&format!("{}:", d.join("src/a.rs").display())),
        "{text}"
    );
    assert!(!text.contains("c.rs"), "gitignored: {text}");
    assert!(text.contains("2 matches across 1 file"), "{text}");
    assert!(
        !text.contains("\n\n\n"),
        "one blank line between blocks: {text}"
    );
    assert!(
        !h.state().app.buf().unwrap().modified,
        "results aren't unsaved work"
    );
    shot(&mut h, "find_results");
    // F4 opens the first match with it selected; Enter on a results line opens that one.
    press(&mut h, Modifiers::NONE, Key::F4);
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.path.as_deref(), Some(d.join("src/a.rs").as_path()));
    assert_eq!(b.selection(), Some((Pos::new(0, 3), Pos::new(0, 9))));
    let results = h
        .state()
        .app
        .buffers
        .iter()
        .position(|b| b.results.is_some())
        .unwrap();
    h.state_mut().app.set_cur(results);
    let line = h
        .state()
        .app
        .buf()
        .unwrap()
        .lines
        .iter()
        .position(|l| l.contains("// a needle"))
        .unwrap();
    h.state_mut().app.buffers[results].goto(Pos::new(line, 0));
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(h.state().app.buf().unwrap().cursor.line, 1);
    // Opening from a file-name line goes to its first match (what Enter and double-click
    // on a results line do).
    h.state_mut().app.set_cur(results);
    let header = h
        .state()
        .app
        .buf()
        .unwrap()
        .lines
        .iter()
        .position(|l| l.ends_with("a.rs:"))
        .unwrap();
    assert!(h.state_mut().app.open_result_line(header));
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.path.as_deref(), Some(d.join("src/a.rs").as_path()));
    assert_eq!(b.cursor.line, 0);
    // Replace in files: asks first, then leaves the files open with the changes unsaved.
    h.state_mut().app.run(Command::FindInFiles);
    steps(&mut h, 2);
    press(&mut h, Modifiers::NONE, Key::Tab);
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(h.state().app.find.field, crate::app::FindField::Replace);
    typ(&mut h, "pin");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(wait(&mut h, 10, |g| matches!(
        g.app.modal,
        Some(Modal::Confirm(_))
    )));
    press(&mut h, Modifiers::NONE, Key::Enter);
    let a = h
        .state()
        .app
        .buffers
        .iter()
        .find(|b| b.path.as_deref() == Some(d.join("src/a.rs").as_path()))
        .unwrap();
    assert_eq!(a.lines[..2], ["fn pin() {}", "// a pin"]);
    assert!(a.modified);
    assert_eq!(
        std::fs::read_to_string(d.join("src/a.rs")).unwrap(),
        "fn needle() {}\n// a needle\n",
        "nothing written until saved"
    );
}

#[test]
fn powershell_without_its_grammar_still_gets_colors() {
    // As on a machine that can't compile grammars (no C compiler): the regex fallback.
    let d = project(
        "ps1",
        &[(
            "a.ps1",
            "# comment\nfunction Get-Thing { param($Name) Write-Output \"hi $Name\" }\n",
        )],
    );
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.ps1"), true);
    assert!(wait(&mut h, 5, |g| g
        .app
        .buf()
        .unwrap()
        .hl
        .line(1)
        .is_some_and(|s| s.len() > 2)));
    let b = h.state().app.buf().unwrap();
    assert_eq!(b.hl.syntax, "powershell");
    let colors: std::collections::HashSet<_> =
        b.hl.line(1).unwrap().iter().map(|s| s.2.fg).collect();
    assert!(colors.len() >= 3, "several colors: {colors:?}");
}

#[test]
fn open_and_save_go_through_the_system_file_picker() {
    let d = project("picker", &[("a.txt", "hello\n")]);
    let mut h = harness(&d);
    steps(&mut h, 2);
    // Cmd+O asks the system picker (answered by the test hook) and opens the file.
    h.state_mut().app.test_pick = Some(d.join("a.txt"));
    press(&mut h, CMD, Key::O);
    assert_eq!(h.state().app.buf().unwrap().name(), "a.txt");
    assert!(h.state().app.modal.is_none(), "no typed prompt");
    // Saving an untitled file asks where, then writes it there.
    h.state_mut().app.untitled_for_test("new text");
    h.state_mut().app.test_pick = Some(d.join("sub/new.txt"));
    press(&mut h, CMD, Key::S);
    assert_eq!(
        std::fs::read_to_string(d.join("sub/new.txt")).unwrap(),
        "new text"
    );
    assert_eq!(h.state().app.buf().unwrap().name(), "new.txt");
    // Cancelling the dialog changes nothing.
    h.state_mut().app.untitled_for_test("x");
    press(&mut h, CMD, Key::S);
    assert!(h.state().app.buf().unwrap().path.is_none());
}

#[test]
fn settings_saved_in_another_window_apply_here() {
    let d = project("cfgwatch", &[("a.txt", "x\n")]);
    let mut h = harness(&d);
    steps(&mut h, 2);
    // The settings window (another process) saves the file; this window notices.
    std::fs::write(
        d.join(".farsub-test-config.toml"),
        "[editor]\ntab_size = 7\nword_wrap = false\n",
    )
    .unwrap();
    assert!(wait(&mut h, 5, |g| g.app.config.editor.tab_size == 7));
}

#[test]
fn select_font_previews_and_keeps_it_in_the_settings() {
    use crate::palette::Action;
    let d = project("font", &[("a.txt", "some text\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    let width = |h: &Harness<'static, Gui>| h.state().text_metrics_for_test().0;
    let before = width(&h);
    h.state_mut().app.run(crate::commands::Command::SelectFont);
    steps(&mut h, 2);
    // Installed monospaced fonts are listed first. Pick one this machine has.
    let (mono, pick) = {
        let Some(Modal::Palette(p)) = &h.state().app.modal else {
            panic!("font list")
        };
        let mono: Vec<String> = p
            .items
            .iter()
            .filter(|i| i.detail == "monospace")
            .map(|i| i.label.clone())
            .collect();
        let pick = [
            "Courier New",
            "Menlo",
            "Consolas",
            "DejaVu Sans Mono",
            "Courier",
        ]
        .into_iter()
        .find(|f| {
            p.items
                .iter()
                .any(|i| matches!(&i.action, Action::Font(n) if n == f))
        });
        (mono, pick.map(String::from))
    };
    assert!(mono.len() >= 2, "{mono:?}");
    let Some(pick) = pick else {
        eprintln!("no known monospaced font installed; skipping the rest");
        return;
    };
    typ(&mut h, &pick);
    steps(&mut h, 3);
    assert_eq!(h.state().app.config.editor.font_face, pick, "previewed");
    assert_ne!(width(&h), before, "the editor text uses it");
    steps(&mut h, 10);
    shot(&mut h, "select_font");
    // Esc puts the old font back; Enter keeps the new one in the settings file.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(h.state().app.config.editor.font_face, "");
    h.state_mut().app.run(crate::commands::Command::SelectFont);
    typ(&mut h, &pick);
    press(&mut h, Modifiers::NONE, Key::Enter);
    let cfg = std::fs::read_to_string(d.join(".farsub-test-config.toml")).unwrap();
    assert!(cfg.contains(&format!("font_face = \"{pick}\"")), "{cfg}");
    steps(&mut h, 3);
    assert_eq!(h.state().app.config.editor.font_face, pick);
    shot(&mut h, "font_applied");
}

#[test]
fn tabs_sidebar_rows_and_menus_react_on_press() {
    let d = project("press", &[("src/a.txt", "a\n"), ("b.txt", "b\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("b.txt"), true);
    h.state_mut().app.open_file(&d.join("src/a.txt"), true);
    steps(&mut h, 3);
    // Press only (no release yet): the first tab is already active.
    let down = |h: &mut Harness<'static, Gui>, pos: egui::Pos2| {
        h.event(Event::PointerMoved(pos));
        steps(h, 1);
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        steps(h, 1);
    };
    let up = |h: &mut Harness<'static, Gui>, pos: egui::Pos2| {
        h.event(Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        steps(h, 1);
    };
    let tab = egui::Pos2::new(250.0 + 40.0, 26.0 + 16.0);
    down(&mut h, tab);
    assert_eq!(
        h.state().app.buf().unwrap().name(),
        "b.txt",
        "switched on press"
    );
    up(&mut h, tab);
    assert_eq!(h.state().app.buf().unwrap().name(), "b.txt");
    // A sidebar folder opens on press.
    let src_row = egui::Pos2::new(60.0, 26.0 + 32.0 + 24.0 + 12.0);
    let open_before = h.state().app.tree.expanded.contains(&d.join("src"));
    down(&mut h, src_row);
    assert_ne!(
        h.state().app.tree.expanded.contains(&d.join("src")),
        open_before,
        "toggled on press"
    );
    up(&mut h, src_row);
    // A menu-bar title opens its menu on press.
    let title = h.state().menu_titles[0].center();
    down(&mut h, title);
    assert_eq!(h.state().app.menu.map(|m| m.menu), Some(0));
    up(&mut h, title);
    assert_eq!(
        h.state().app.menu.map(|m| m.menu),
        Some(0),
        "still open after release"
    );
}

#[test]
fn multiple_cursors_by_keyboard_and_mouse() {
    let d = project("cursors", &[("a.txt", "foo x foo\nfoo y\nabcdef\nabcdef\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    let text = |h: &Harness<'static, Gui>| h.state().app.buf().unwrap().text();
    let count = |h: &Harness<'static, Gui>| h.state().app.buf().unwrap().sels().len();
    // Cmd+D three times: "foo" and its next two occurrences; typing replaces all three.
    for _ in 0..3 {
        press(&mut h, CMD, Key::D);
    }
    assert_eq!(count(&h), 3);
    typ(&mut h, "Z");
    assert_eq!(text(&h), "Z x Z\nZ y\nabcdef\nabcdef\n");
    // Copy three pieces, paste them back one per cursor.
    press(&mut h, CMD, Key::Z);
    assert_eq!(count(&h), 3);
    h.state_mut().app.run(crate::commands::Command::Copy);
    typ(&mut h, "-");
    h.state_mut().app.run(crate::commands::Command::Paste);
    assert_eq!(text(&h), "-foo x -foo\n-foo y\nabcdef\nabcdef\n");
    // Esc: back to one cursor.
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(count(&h), 1);
    // Cmd+Click adds a caret; typing goes to both.
    let mods = |h: &mut Harness<'static, Gui>, m: Modifiers| {
        h.event(Event::ModifiersChanged(m));
    };
    let a = h.state().pos_px_for_test(2, 1);
    let b = h.state().pos_px_for_test(3, 4);
    click(&mut h, a);
    mods(&mut h, CMD);
    h.event(Event::PointerMoved(b));
    steps(&mut h, 1);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: b,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: CMD,
        });
        steps(&mut h, 1);
    }
    mods(&mut h, Modifiers::NONE);
    steps(&mut h, 1);
    assert_eq!(count(&h), 2, "Cmd+Click added a caret");
    typ(&mut h, "_");
    assert_eq!(text(&h), "-foo x -foo\n-foo y\na_bcdef\nabcd_ef\n");
    // Alt+drag: a column from line 2 to 3, columns 1..3.
    press(&mut h, Modifiers::NONE, Key::Escape);
    let from = h.state().pos_px_for_test(2, 1);
    let to = h.state().pos_px_for_test(3, 3);
    mods(&mut h, Modifiers::ALT);
    h.event(Event::PointerMoved(from));
    steps(&mut h, 1);
    h.event(Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::ALT,
    });
    steps(&mut h, 1);
    for k in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
        steps(&mut h, 1);
    }
    h.event(Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::ALT,
    });
    mods(&mut h, Modifiers::NONE);
    steps(&mut h, 2);
    assert_eq!(
        h.state().app.buf().unwrap().selected_texts(),
        vec!["_b".to_string(), "bc".to_string()]
    );
    // Find All from the find panel: a cursor on every "foo".
    press(&mut h, CMD, Key::F);
    typ(&mut h, "foo");
    press(&mut h, Modifiers::ALT, Key::Enter);
    assert_eq!(count(&h), 3);
    assert_eq!(h.state().app.focus, Focus::Editor);
}

#[test]
fn lsp_code_actions_and_format_on_save() {
    if !clangd_available() {
        eprintln!("clangd not found; skipping");
        return;
    }
    let src = "int main() {\n    int x = 1\n    return x;\n}\n";
    let d = project("codeaction", &[("main.c", src)]);
    let path = d.join("main.c");
    let mut h = harness(&d);
    h.state_mut().app.open_file(&path, true);
    assert!(wait(&mut h, 20, |g| g
        .app
        .diagnostics
        .get(&path)
        .is_some_and(|d| d.iter().any(|x| x.severity == 1))));
    // Cmd+. on the error (clangd puts it on the next token): it offers to insert the ';'.
    h.state_mut().app.buf_mut().unwrap().goto(Pos::new(2, 6));
    press(&mut h, CMD, Key::Period);
    assert!(
        wait(&mut h, 10, |g| matches!(&g.app.modal,
            Some(Modal::Palette(p)) if p.mode == crate::palette::Mode::CodeActions
                && !p.shown.is_empty())),
        "code actions listed"
    );
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(wait(&mut h, 5, |g| g.app.buf().unwrap().lines[1] == "    int x = 1;"));
    // Cmd+. also offers the editor's actions for the symbol (after the server's own):
    // rename, go to definition, … Going to the definition of the `x` in `return x;`.
    h.state_mut().app.buf_mut().unwrap().goto(Pos::new(2, 11));
    press(&mut h, CMD, Key::Period);
    assert!(wait(&mut h, 10, |g| matches!(&g.app.modal,
        Some(Modal::Palette(p)) if p.mode == crate::palette::Mode::CodeActions)));
    let labels: Vec<String> = match &h.state().app.modal {
        Some(Modal::Palette(p)) => p.shown.iter().map(|(i, _)| p.items[*i].label.clone()).collect(),
        _ => unreachable!(),
    };
    for want in ["Rename Symbol", "Goto Definition", "Find References"] {
        assert!(labels.iter().any(|l| l == want), "{want} missing: {labels:?}");
    }
    let k = labels.iter().position(|l| l == "Goto Definition").unwrap();
    for _ in 0..k {
        press(&mut h, Modifiers::NONE, Key::ArrowDown);
    }
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        wait(&mut h, 10, |g| g.app.buf().unwrap().cursor.line == 1),
        "went to the definition: {:?}",
        h.state().app.buf().unwrap().cursor
    );
    // Format on save: the file is formatted, then saved.
    h.state_mut().app.config.editor.format_on_save = true;
    {
        let b = h.state_mut().app.buf_mut().unwrap();
        b.goto(Pos::new(2, 4));
        b.insert_text("      ");
    }
    press(&mut h, CMD, Key::S);
    assert!(
        wait(&mut h, 10, |g| !g.app.buf().unwrap().modified),
        "saved after formatting"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "int main() {\n  int x = 1;\n  return x;\n}\n",
        "clangd's default style (2 spaces)"
    );
}

#[test]
fn licenses_page_lists_what_is_in_the_build() {
    let d = project("licenses", &[]);
    let mut h = harness(&d);
    h.state_mut().app.run(crate::commands::Command::ShowLicenses);
    steps(&mut h, 3);
    // It's Markdown: the title gets the heading color once it's parsed.
    let heading = h.state().app.syn.theme.style_for("markup.heading");
    assert!(wait(&mut h, 10, |g| g.app.buf().unwrap().hl.line(0).is_some_and(|l| l
        .iter()
        .any(|sp| sp.0 <= 3 && 3 < sp.1 && Some(sp.2) == heading))));
    shot(&mut h, "licenses_page");
    let n = h.state().app.buffers.len();
    {
        let b = h.state().app.buf().unwrap();
        assert_eq!(b.title.as_deref(), Some("Licenses and Attribution"));
        assert!(b.read_only && b.path.is_none());
        let t = b.text();
        // Crates linked in, bundled data, and the license texts.
        for s in [
            "| egui | ",
            "| tree-sitter | ",
            "| two-face | ",
            "Color theme \"Tsoding\" — MIT",
            "Copyright 2023 JoaoAJMatos",
            "SIL Open Font License",
            "END OF TERMS AND CONDITIONS",
        ] {
            assert!(t.contains(s), "missing {s:?}");
        }
        // Not in the binary: compile-time proc-macros and dev-dependencies.
        for s in ["| serde_derive | ", "| egui_kittest | "] {
            assert!(!t.contains(s), "shouldn't list {s:?}");
        }
        // Every bracketed reference points at a license text that exists.
        let texts = t.matches("\n### [").count();
        let listing = t.split("\n## License texts").next().unwrap();
        for r in listing.split('[').skip(1).filter_map(|s| s.split(']').next()?.parse::<usize>().ok()) {
            assert!(r >= 1 && r <= texts, "dangling reference [{r}]");
        }
    }
    // Read-only: typing changes nothing; opening it again reuses the tab.
    typ(&mut h, "x");
    assert!(!h.state().app.buf().unwrap().text().starts_with('x'));
    h.state_mut().app.run(crate::commands::Command::ShowLicenses);
    assert_eq!(h.state().app.buffers.len(), n);
    shot(&mut h, "licenses");
}

#[test]
fn diagnostic_under_the_cursor_shows_in_the_status_bar() {
    let d = project("diag-status", &[("a.rs", "let x = 1;\nlet y = 2;\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.rs"), true);
    steps(&mut h, 2);
    let diag = |l: usize, a: usize, b: usize, sev: u8, m: &str| crate::app::Diag {
        start: Pos { line: l, col: a },
        end: Pos { line: l, col: b },
        severity: sev,
        message: m.into(),
        raw: serde_json::Value::Null,
    };
    h.state_mut().app.diagnostics.insert(
        d.join("a.rs"),
        vec![
            diag(0, 4, 5, 2, "unused variable: `x`\nhelp: prefix it with _"),
            diag(0, 8, 9, 1, "mismatched types"),
        ],
    );
    let at = |h: &mut Harness<'static, Gui>, line: usize, col: usize| {
        h.state_mut().app.buf_mut().unwrap().set_cursor(Pos { line, col }, false);
        h.state().app.diagnostic_at_cursor()
    };
    // Under the caret wins over a more severe one elsewhere on the line; one line of text.
    assert_eq!(
        at(&mut h, 0, 4),
        Some((2, "warning: unused variable: `x` help: prefix it with _".into()))
    );
    assert_eq!(at(&mut h, 0, 9), Some((1, "error: mismatched types".into())));
    // Elsewhere on the line: the most severe on the line; another line: nothing.
    assert_eq!(at(&mut h, 0, 0).map(|d| d.0), Some(1));
    assert_eq!(at(&mut h, 1, 0), None);
}

#[test]
fn clock_pomodoro_and_alarm_widgets() {
    use crate::app::TimerKind;
    use crate::commands::Command;
    let d = project("timers", &[("a.txt", "hello\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 3);
    let kind = |h: &Harness<'static, Gui>| match &h.state().app.modal {
        Some(Modal::Timers(d)) => Some(d.kind),
        _ => None,
    };
    // The clock sits at the right end of the tab strip: clicking it opens its dialog.
    let ed = h.state().editor_rect_for_test();
    let at = egui::Pos2::new(ed.right() - 20.0, ed.top() - 16.0);
    h.event(Event::PointerMoved(at));
    steps(&mut h, 1);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        steps(&mut h, 1);
    }
    steps(&mut h, 1);
    assert_eq!(kind(&h), Some(TimerKind::Clock));
    // Space flips "24-hour clock"; Enter applies and closes.
    typ(&mut h, " ");
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(kind(&h), None);
    assert!(!h.state().app.timers.clock.hour24);

    // Pomodoro: set work to 1 minute, then Start (the 6th row).
    h.state_mut().app.run(Command::Pomodoro);
    steps(&mut h, 1);
    assert_eq!(kind(&h), Some(TimerKind::Pomodoro));
    press(&mut h, CMD, Key::A);
    typ(&mut h, "1");
    for _ in 0..5 {
        press(&mut h, Modifiers::NONE, Key::ArrowDown);
    }
    press(&mut h, Modifiers::NONE, Key::Enter);
    {
        let p = &h.state().app.timers.pomodoro;
        assert_eq!(p.work, 1);
        assert!(p.is_running());
        assert_eq!(p.remaining(crate::timers::now()).map(|s| s <= 60), Some(true));
    }
    // A bad number is refused with a message; the dialog stays.
    press(&mut h, Modifiers::NONE, Key::ArrowUp); // back to "auto" toggle
    for _ in 0..4 {
        press(&mut h, Modifiers::NONE, Key::ArrowUp);
    }
    press(&mut h, CMD, Key::A);
    typ(&mut h, "0");
    press(&mut h, Modifiers::NONE, Key::Enter);
    match &h.state().app.modal {
        Some(Modal::Timers(d)) => assert!(d.error.as_deref().unwrap().starts_with("Work")),
        _ => panic!("dialog closed on a bad value"),
    }
    shot(&mut h, "timers_pomodoro_dialog");
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(h.state().app.timers.pomodoro.work, 1);

    // Alarm: type a time, then "Set alarm" (4th row).
    h.state_mut().app.run(Command::Alarm);
    steps(&mut h, 1);
    typ(&mut h, "18:30");
    for _ in 0..3 {
        press(&mut h, Modifiers::NONE, Key::ArrowDown);
    }
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(h.state().app.timers.alarm.at.is_some());
    assert_eq!(h.state().app.timers.alarm.time, "18:30");
    press(&mut h, Modifiers::NONE, Key::Escape);
    shot(&mut h, "timers_widgets");

    // A phase ends: the pomodoro flashes; clicking it only stops the flash.
    let n = crate::timers::now();
    h.state_mut().app.timers.pomodoro.run = crate::timers::Run::Running { ends: n - 1 };
    h.state_mut().app.tick();
    assert!(h.state().app.timers.flash.0);
    assert_eq!(
        h.state().app.timers.pomodoro.phase,
        crate::timers::Phase::ShortBreak
    );
    // Draw it in both halves of the blink.
    steps(&mut h, 1);
    shot(&mut h, "timers_flash");
    h.state_mut().app.timer_widget_clicked(TimerKind::Pomodoro);
    assert!(!h.state().app.timers.flash.0);
    assert_eq!(kind(&h), None);
    // Saved: a new window has the same state.
    let again = Gui::headless(d.clone(), test_config(&d));
    assert_eq!(again.app.timers.alarm.time, "18:30");
    assert_eq!(again.app.timers.pomodoro.work, 1);
}

#[test]
fn wheel_scrolls_the_tab_strip() {
    let files: Vec<(String, String)> = (0..14)
        .map(|i| (format!("file_with_a_long_name_{i:02}.txt"), "x\n".to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let d = project("tabscroll", &refs);
    let mut h = harness(&d);
    for (f, _) in &files {
        h.state_mut().app.open_file(&d.join(f), true);
    }
    steps(&mut h, 3);
    let off = |h: &Harness<'static, Gui>| h.state().tab_scroll.get(&0).map(|t| t.0).unwrap_or(0.0);
    // The last tab is active: the strip is scrolled to show it.
    let end = off(&h);
    assert!(end > 0.0);
    let ed = h.state().editor_rect_for_test();
    let over_tabs = egui::Pos2::new(ed.left() + 200.0, ed.top() - 16.0);
    let wheel = |h: &mut Harness<'static, Gui>, dy: f32| {
        h.event(Event::PointerMoved(over_tabs));
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::Vec2::new(0.0, dy),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        steps(h, 20);
    };
    // Wheel up scrolls back toward the first tabs, without changing the active tab or
    // scrolling the editor.
    let cur = h.state().app.cur;
    wheel(&mut h, 400.0);
    assert!(off(&h) < end, "{} !< {end}", off(&h));
    assert_eq!(h.state().app.cur, cur);
    assert_eq!(h.state().app.buf().unwrap().scroll_y, 0);
    wheel(&mut h, 100_000.0);
    assert_eq!(off(&h), 0.0);
    shot(&mut h, "tabs_scrolled");
    // Down again: never past the last tab.
    wheel(&mut h, -100_000.0);
    assert!((off(&h) - end).abs() < 1.0, "{} vs {end}", off(&h));
    // Showing a tab out of view brings it back.
    wheel(&mut h, 100_000.0);
    h.state_mut().app.run(crate::commands::Command::PrevBuffer);
    steps(&mut h, 2);
    assert!(off(&h) > 0.0);
}

/// Preview every installed font in Select Font, rendering each (the real GPU path).
/// Slow; run with --ignored.
#[test]
#[ignore]
fn select_font_previews_every_installed_font() {
    let d = project("fontcrawl", &[("a.rs", "fn main() { let x = \"½ → ✓\"; }\n")]);
    let mut h = harness(&d);
    // Like the app: a character no font has draws as a box (the harness panics).
    h.ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
    h.state_mut().app.open_file(&d.join("a.rs"), true);
    steps(&mut h, 2);
    h.state_mut().app.run(crate::commands::Command::SelectFont);
    steps(&mut h, 2);
    let n = match &h.state().app.modal {
        Some(Modal::Palette(p)) => p.shown.len(),
        _ => panic!("no palette"),
    };
    for i in 0..n {
        let name = h.state().app.config.editor.font_face.clone();
        eprintln!("font {i}/{n}: {name:?}");
        press(&mut h, Modifiers::NONE, Key::ArrowDown);
        let _ = h.render();
    }
}

#[test]
fn widgets_take_clicks_over_overflowing_tabs() {
    let files: Vec<(String, String)> = (0..14)
        .map(|i| (format!("file_with_a_long_name_{i:02}.txt"), "x\n".to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let d = project("tabsunder", &refs);
    let mut h = harness(&d);
    for (f, _) in &files {
        h.state_mut().app.open_file(&d.join(f), true);
    }
    // Show a tab in the middle, so others run under the widgets.
    let want = d.join(&files[6].0);
    let mid = (h.state().app.buffers.iter())
        .position(|b| b.path.as_deref() == Some(want.as_path()))
        .unwrap();
    h.state_mut().app.set_cur(mid);
    steps(&mut h, 3);
    let cur = h.state().app.cur;
    let ed = h.state().editor_rect_for_test();
    let at = egui::Pos2::new(ed.right() - 20.0, ed.top() - 16.0);
    h.event(Event::PointerMoved(at));
    steps(&mut h, 1);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        steps(&mut h, 1);
    }
    assert_eq!(h.state().app.cur, cur, "a tab under the clock took the click");
    assert!(matches!(
        &h.state().app.modal,
        Some(Modal::Timers(t)) if t.kind == crate::app::TimerKind::Clock
    ));
    press(&mut h, Modifiers::NONE, Key::Escape);
    h.event(Event::PointerMoved(egui::Pos2::new(ed.left() + 300.0, ed.top() + 200.0)));
    steps(&mut h, 2);
    shot(&mut h, "tabs_under_widgets");
}

/// Pressing a tab half hidden under the widgets switches to it and scrolls it into view,
/// which slides its × under the mouse: the release must not close it.
#[test]
fn clicking_a_half_hidden_tab_does_not_close_it() {
    let files: Vec<(String, String)> = (0..14)
        .map(|i| (format!("f{i:02}.txt"), "x\n".to_string()))
        .collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let d = project("tabhalf", &refs);
    let mut h = harness(&d);
    for (f, _) in &files {
        h.state_mut().app.open_file(&d.join(f), true);
    }
    h.state_mut().app.set_cur(0);
    steps(&mut h, 3);
    let ed = h.state().editor_rect_for_test();
    // Click every few points from the widgets leftwards over the last tabs, starting
    // from the first tab each time, so some click lands on a tab half under the widgets.
    let mut switched = 0;
    let mut x = ed.right() - 10.0;
    while x > ed.right() - 250.0 {
        h.state_mut().app.set_cur(0);
        steps(&mut h, 2);
        let at = egui::Pos2::new(x, ed.top() - 16.0);
        h.event(Event::PointerMoved(at));
        steps(&mut h, 1);
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.event(button(true));
        steps(&mut h, 1);
        let opened = h.state().app.cur != 0;
        let n = h.state().app.buffers.len();
        h.event(button(false));
        steps(&mut h, 1);
        if opened {
            switched += 1;
            assert_eq!(h.state().app.buffers.len(), n, "the tab closed right after opening (x {x})");
        }
        if h.state().app.modal.is_some() {
            press(&mut h, Modifiers::NONE, Key::Escape);
            steps(&mut h, 1);
        }
        x -= 3.0;
    }
    assert!(switched > 0, "no click switched tabs");
}

/// Opening a file selects it in the sidebar (expanding its folders) and scrolls to it;
/// `reveal_opened_file = false` leaves the sidebar alone.
#[test]
fn opening_a_file_reveals_it_in_the_sidebar() {
    let mut files: Vec<(String, String)> =
        (0..80).map(|i| (format!("a/f{i:02}.txt"), String::new())).collect();
    files.push(("z/deep/target.txt".into(), "x\n".into()));
    files.push(("z/other.txt".into(), "x\n".into()));
    let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let d = project("reveal", &refs);
    let mut h = harness(&d);
    let a = d.canonicalize().unwrap().join("a");
    h.state_mut().app.tree.expand_path(&a);
    h.state_mut().app.tree.rebuild();
    steps(&mut h, 2);
    let target = d.canonicalize().unwrap().join("z/deep/target.txt");
    h.state_mut().app.open_file(&target, true);
    steps(&mut h, 2);
    let t = &h.state().app.tree;
    assert_eq!(t.selected_path(), Some(target.as_path()));
    assert!(t.sel >= t.scroll && t.scroll > 0, "the sidebar didn't scroll to it");
    assert_eq!(h.state().app.focus, Focus::Editor);
    // Off: the selection stays where it was.
    h.state_mut().app.config.editor.reveal_opened_file = false;
    h.state_mut().app.open_file(&d.join("z/other.txt"), true);
    steps(&mut h, 2);
    assert_eq!(h.state().app.tree.selected_path(), Some(target.as_path()));
}

/// The welcome screen's hints come from the keymap (right per platform, and rebindable).
#[test]
fn welcome_screen_shows_the_keymaps_shortcuts() {
    let d = project("welcome", &[]);
    let mut h = harness(&d);
    steps(&mut h, 2);
    let lines = h.state().welcome_lines();
    assert_eq!(lines.len(), 7, "{lines:?}");
    let km = &h.state().app.keymap;
    assert!(lines.contains(&(
        "Command Palette",
        km.shortcut(crate::commands::Command::ShowCommands)
    )));
    if !cfg!(target_os = "macos") {
        assert!(lines.iter().all(|(_, k)| !k.contains("Cmd")), "{lines:?}");
    }
}

/// Preferences: Key Bindings, like Preferences: Settings: the default bindings (with every
/// command listed) on the left, your keymap.toml on the right. Saving applies it; so does
/// changing the file outside the app.
#[test]
fn key_bindings_window() {
    use crate::commands::Command;
    let d = project("keymapwin", &[]);
    let mut h = harness(&d);
    h.state_mut().app.settings_window = true;
    h.state_mut().app.run(Command::OpenKeymap);
    steps(&mut h, 2);
    let user = d.join("keymap.toml");
    assert!(user.is_file(), "keymap.toml is created from the template");
    {
        let app = &h.state().app;
        assert_eq!((app.layout, app.group), (2, 1));
        assert_eq!(app.buf().unwrap().name(), "keymap.toml");
        let left = &app.buffers[app.group_shown(0).unwrap()];
        assert_eq!(left.name(), "Default Key Bindings.toml");
        assert!(left.read_only);
        for &(c, _, _) in crate::commands::COMMANDS {
            let id = crate::commands::id(c);
            assert!(
                left.lines.iter().any(|l| l.starts_with(&format!("#   {id} "))),
                "{id} isn't listed"
            );
        }
        assert!(left.lines.iter().any(|l| l == "command = \"goto_file\""));
    }
    shot(&mut h, "key_bindings_side_by_side");
    // Bind a key in the right pane and save.
    {
        let b = h.state_mut().app.buf_mut().unwrap();
        b.move_doc(true, false);
        b.insert_text("\n[[keys]]\nkeys = \"Cmd+Alt+J\"\ncommand = \"goto_symbol\"\n");
    }
    press(&mut h, CMD, Key::S);
    let bound = |g: &Gui, k: &str| {
        (g.app.keymap.keys_for(Command::GotoSymbol)).contains(&crate::keymap::display(k))
    };
    assert!(bound(h.state(), "Cmd+Alt+J"));
    // Changed outside the app (another window, a text editor): picked up too.
    let text = std::fs::read_to_string(&user).unwrap().replace("Cmd+Alt+J", "Cmd+Alt+K");
    std::fs::write(&user, text).unwrap();
    assert!(wait(&mut h, 5, |g| bound(g, "Cmd+Alt+K")), "the edit wasn't picked up");
    // Settings open in the same window too; closing the key bindings leaves them open.
    h.state_mut().app.run(Command::OpenSettings);
    steps(&mut h, 2);
    let keymap_tab = (h.state().app.buffers.iter())
        .position(|b| b.name() == "keymap.toml")
        .unwrap();
    h.state_mut().app.set_cur(keymap_tab);
    press(&mut h, CMD, Key::W);
    let names: Vec<String> = h.state().app.buffers.iter().map(|b| b.name()).collect();
    assert!(!h.state().app.quit, "{names:?}");
    assert!(names.iter().any(|n| n == ".farsub-test-config.toml"), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("Key Bindings") || n == "keymap.toml"), "{names:?}");
}

/// A window with no folder starts without the sidebar (nothing to show); Toggle Sidebar
/// still opens it, and opening a folder shows it.
#[test]
fn empty_window_starts_without_the_sidebar() {
    use crate::commands::Command;
    let d = project("nosidebar", &[("a.txt", "x\n")]);
    let mut h = project_harness(crate::project::Project::from_folders(vec![]), test_config(&d));
    steps(&mut h, 2);
    assert!(!h.state().app.sidebar_visible);
    h.state_mut().app.run(Command::ToggleSidebar);
    assert!(h.state().app.sidebar_visible, "it can still be opened");
    h.state_mut().app.run(Command::ToggleSidebar);
    // A folder window shows it.
    let h2 = harness(&d);
    assert!(h2.state().app.sidebar_visible);
}

/// Popups and dialogs keep the mouse to themselves: no I-beam from the editor under a
/// context menu's padding or under a dialog's dimmed backdrop.
#[test]
fn popups_and_dialogs_hide_the_editor_cursor() {
    let text = "some text on a line\n".repeat(40);
    let d = project("popcursor", &[("a.txt", text.as_str())]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.txt"), true);
    steps(&mut h, 2);
    let ed = h.state().editor_rect_for_test();
    let icon_at = |h: &mut Harness<'static, Gui>, at: egui::Pos2| {
        h.event(Event::PointerMoved(at));
        steps(h, 2);
        h.output().platform_output.cursor_icon
    };
    let at = egui::Pos2::new(ed.left() + 120.0, ed.top() + 60.0);
    assert_eq!(icon_at(&mut h, at), egui::CursorIcon::Text, "over plain text");
    // A context menu there: its top padding (no item under it) isn't the editor.
    h.state_mut()
        .app
        .open_context_menu(crate::menu::ContextKind::Editor, (at.x, at.y));
    steps(&mut h, 2);
    let pad = egui::Pos2::new(at.x + 20.0, at.y + 2.0);
    assert_ne!(icon_at(&mut h, pad), egui::CursorIcon::Text, "context menu padding");
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(icon_at(&mut h, at), egui::CursorIcon::Text);
    // A dialog dims the window: the editor behind it doesn't take the mouse.
    h.state_mut().app.open_timer_dialog(crate::app::TimerKind::Clock);
    steps(&mut h, 2);
    assert_ne!(icon_at(&mut h, at), egui::CursorIcon::Text, "behind a dialog");
}

/// Every completion kind gets a Codicons icon that's really in the bundled font.
#[test]
fn completion_kinds_have_codicons() {
    const KINDS: &[&str] = &[
        "txt", "meth", "fn", "ctor", "field", "var", "class", "iface", "mod", "prop", "unit",
        "val", "enum", "kw", "snip", "color", "file", "ref", "dir", "variant", "const",
        "struct", "event", "op", "type",
    ];
    // Goto Symbol's kind names (LSP SymbolKind) too.
    const SYMBOL_KINDS: &[&str] = &[
        "file", "mod", "ns", "pkg", "class", "method", "prop", "field", "ctor", "enum",
        "iface", "fn", "var", "const", "str", "num", "bool", "arr", "obj", "key", "null",
        "variant", "struct", "event", "op", "type",
    ];
    let font = include_bytes!("../runtime/fonts/codicon.ttf");
    let face = ttf_parser::Face::parse(font, 0).unwrap();
    for k in KINDS.iter().chain(SYMBOL_KINDS) {
        let (icon, _) = crate::gui::kind_codicon(k);
        let ch = icon.chars().next().unwrap();
        assert!(face.glyph_index(ch).is_some(), "{k}: U+{:04X} isn't in the font", ch as u32);
    }
    // The list, drawn (for a look with FARSUB_SHOTS).
    let d = project("kindicons", &[("a.rs", "fn main() {}\n")]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.rs"), true);
    steps(&mut h, 2);
    let id = h.state().app.buf().unwrap().id;
    let items: Vec<crate::app::CompItem> = KINDS
        .iter()
        .map(|k| crate::app::CompItem {
            label: format!("item_{k}"),
            detail: (*k).to_string(),
            kind: k,
            insert: String::new(),
            edit_start: None,
            filter: format!("item_{k}"),
            sort: String::new(),
            additional: vec![],
        })
        .collect();
    let shown = (0..items.len()).collect();
    h.state_mut().app.completion = Some(crate::app::Completion {
        buf_id: id,
        start: crate::editor::Pos::new(0, 0),
        items,
        shown,
        sel: Some(0),
    });
    steps(&mut h, 3);
    assert!(h.state().app.completion.is_some());
    shot(&mut h, "completion_kind_icons");
}

/// Markdown uses the regex grammar by default (it colors inline markup and fenced code by
/// its language); `highlighter = { markdown = "tree-sitter" }` switches it, live.
#[test]
fn markdown_file_is_highlighted() {
    let md = "# Title\n\nSome **bold** and `code`.\n\n```rust\nfn main() {}\n```\n";
    let d = project("mdhl", &[("a.md", md)]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("a.md"), true);
    let style = |g: &Gui, c: &str| g.app.syn.theme.style_for(c);
    let at = |g: &Gui, ln: usize, col: usize| {
        let b = g.app.buf().unwrap();
        b.hl.line(ln)
            .and_then(|l| l.iter().find(|sp| sp.0 <= col && col < sp.1).map(|sp| sp.2))
    };
    let ok = wait(&mut h, 5, |g| {
        at(g, 0, 3) == style(g, "markup.heading")
            && at(g, 2, 8).is_some_and(|s| s.bold)
            && at(g, 5, 0).is_some() // `fn`, colored as Rust
    });
    let b = h.state().app.buf().unwrap();
    assert!(
        ok,
        "grammar {:?} fallback {:?}: {:?}",
        b.hl.grammar,
        b.hl.fallback_name(),
        (0..7).map(|l| b.hl.line(l).cloned()).collect::<Vec<_>>()
    );
    assert_eq!(b.hl.grammar, None, "regex, not tree-sitter");
    assert!(at(h.state(), 2, 20).is_some(), "inline code is colored");
    shot(&mut h, "markdown_file");
    // Tree-sitter instead, from the settings file: the open file switches.
    let cfg = d.join(".farsub-test-config.toml");
    std::fs::write(&cfg, "[editor]\nhighlighter = { markdown = \"tree-sitter\" }\n").unwrap();
    assert!(
        wait(&mut h, 5, |g| g.app.buf().unwrap().hl.grammar.as_deref() == Some("markdown")),
        "switched to tree-sitter"
    );
    assert!(wait(&mut h, 5, |g| at(g, 0, 3) == style(g, "markup.heading")));
}

/// Goto Symbol shows each symbol's kind as its Codicon (from the language server).
#[test]
fn goto_symbol_shows_kind_icons() {
    if !clangd_available() {
        eprintln!("clangd not found; skipping");
        return;
    }
    let src = "struct Point { int x; int y; };\nenum Color { Red, Green };\nint add(int a, int b) {\n    return a + b;\n}\nint main() {\n    return add(1, 2);\n}\n";
    let d = project("symicons", &[("main.c", src)]);
    let mut h = harness(&d);
    h.state_mut().app.open_file(&d.join("main.c"), true);
    assert!(wait(&mut h, 20, |g| g.app.buf().unwrap().lsp_client.is_some()));
    steps(&mut h, 5);
    h.state_mut().app.run(crate::commands::Command::GotoSymbol);
    let icons = |g: &Gui| match &g.app.modal {
        Some(Modal::Palette(p)) if !p.loading => {
            p.items.iter().map(|i| (i.label.trim().to_string(), i.icon)).collect()
        }
        _ => vec![],
    };
    assert!(wait(&mut h, 10, |g| icons(g).iter().any(|(l, _)| l == "add")), "{:?}", icons(h.state()));
    let got = icons(h.state());
    let icon_of = |n: &str| got.iter().find(|(l, _)| l == n).and_then(|(_, i)| *i);
    assert_eq!(icon_of("add"), Some("fn"), "{got:?}");
    // clangd calls a C struct a class.
    assert_eq!(icon_of("Point"), Some("class"), "{got:?}");
    assert_eq!(icon_of("x"), Some("field"), "{got:?}");
    assert_eq!(icon_of("Color"), Some("enum"), "{got:?}");
    shot(&mut h, "goto_symbol_icons");
}
