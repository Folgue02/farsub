//! Application state and all input/command handling, independent of the GUI toolkit.
//! The GUI (gui.rs) feeds keys/text/clipboard events in and renders this state.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use lsp_types as lt;
use serde_json::{Value, json};

use crate::commands::{COMMANDS, Command};
use crate::config::{self, Config};
use crate::editor::{Buffer, Pos, char_len, is_word};
use crate::fileops;
use crate::highlight::{HlCache, Syntax};
use crate::keymap::{self, Ctx};
use crate::keys::{Key, KeyCode};
use crate::lsp::{Manager, Pending, from_lsp, path_to_uri, to_lsp, uri_to_path};
use crate::menu::{self, MenuState};
use crate::palette::{Action, Item, Mode, Palette};
use crate::tree::Tree;

pub enum AppEvent {
    Lsp(usize, Value),
    LspExit(usize),
    Files(PathBuf, Vec<String>),
}

/// Event sender that also wakes the GUI so background results show up immediately.
#[derive(Clone)]
pub struct EventTx {
    tx: Sender<AppEvent>,
    wake: Arc<OnceLock<Box<dyn Fn() + Send + Sync>>>,
}

impl EventTx {
    pub fn new(tx: Sender<AppEvent>) -> Self {
        EventTx {
            tx,
            wake: Arc::new(OnceLock::new()),
        }
    }

    pub fn set_waker(&self, f: impl Fn() + Send + Sync + 'static) {
        let _ = self.wake.set(Box::new(f));
    }

    pub fn send(&self, ev: AppEvent) -> bool {
        let ok = self.tx.send(ev).is_ok();
        if let Some(w) = self.wake.get() {
            w();
        }
        ok
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Editor,
    Sidebar,
}

pub enum PromptKind {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    RenameMove(PathBuf),
    SaveAs,
    OpenPath,
    Find { origin: Pos },
    ReplaceFind,
    ReplaceWith(String),
    RenameSymbol,
}

impl PromptKind {
    fn is_path(&self) -> bool {
        matches!(
            self,
            PromptKind::NewFile(_)
                | PromptKind::NewFolder(_)
                | PromptKind::RenameMove(_)
                | PromptKind::SaveAs
                | PromptKind::OpenPath
        )
    }
}

pub struct Prompt {
    pub title: String,
    pub input: String,
    pub cursor: usize,
    pub kind: PromptKind,
}

impl Prompt {
    pub fn kind_is_path(&self) -> bool {
        self.kind.is_path()
    }
}

pub enum ConfirmKind {
    Trash(Vec<PathBuf>),
    CloseBuffer(usize),
    Quit,
}

pub struct Confirm {
    pub message: String,
    pub kind: ConfirmKind,
}

pub enum Modal {
    Palette(Palette),
    Prompt(Prompt),
    Confirm(Confirm),
    Keymap(KeymapView),
}

/// The keyboard-shortcut reference (Help → Keyboard Shortcuts).
pub struct KeymapView {
    pub query: String,
    pub sel: usize,
    pub scroll: usize,
}

#[derive(Clone, Debug)]
pub struct KeymapRow {
    pub section: &'static str,
    pub keys: String,
    pub desc: String,
    pub cmd: Option<Command>,
}

/// All shortcuts grouped by where they apply, filtered by a search string.
pub fn keymap_rows(query: &str) -> Vec<KeymapRow> {
    let mut rows = vec![];
    for (ctx, section) in [
        (Ctx::Global, "General"),
        (Ctx::Editor, "Editor"),
        (Ctx::Sidebar, "Sidebar (when focused)"),
    ] {
        for (cmd, name, _) in COMMANDS {
            let keys: Vec<String> = keymap::BINDINGS
                .iter()
                .filter(|b| b.1 == *cmd && b.2 == ctx)
                .map(|b| keymap::display(b.0))
                .collect();
            if !keys.is_empty() {
                rows.push(KeymapRow {
                    section,
                    keys: keys.join(", "),
                    desc: name.to_string(),
                    cmd: Some(*cmd),
                });
            }
        }
    }
    for (section, keys, desc) in keymap::DOC_KEYS {
        rows.push(KeymapRow {
            section,
            keys: keymap::display(keys),
            desc: desc.to_string(),
            cmd: None,
        });
    }
    for (cmd, name, in_palette) in COMMANDS {
        if *in_palette && keymap::keys_for(*cmd).is_empty() {
            rows.push(KeymapRow {
                section: "No shortcut (use the command palette)",
                keys: String::new(),
                desc: name.to_string(),
                cmd: Some(*cmd),
            });
        }
    }
    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        rows.retain(|r| {
            r.desc.to_lowercase().contains(&q)
                || r.keys.to_lowercase().contains(&q)
                || r.section.to_lowercase().contains(&q)
        });
    }
    rows
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub start: Pos,
    pub end: Pos,
    pub severity: u8,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct CompItem {
    pub label: String,
    pub detail: String,
    pub kind: &'static str,
    pub insert: String,
    pub edit_start: Option<Pos>,
    pub filter: String,
    pub sort: String,
    pub additional: Vec<(Pos, Pos, String)>,
}

pub struct Completion {
    pub buf_id: usize,
    pub start: Pos,
    pub items: Vec<CompItem>,
    pub shown: Vec<usize>,
    pub sel: usize,
}

pub struct Hover {
    pub lines: Vec<String>,
}

pub struct App {
    /// Project folder shown in the sidebar and searched by Goto Anything.
    pub root: PathBuf,
    pub config: Config,
    pub focus: Focus,
    pub sidebar_visible: bool,
    pub show_hidden: bool,
    pub buffers: Vec<Buffer>,
    pub cur: usize,
    next_buf_id: usize,
    pub syn: Syntax,
    pub modal: Option<Modal>,
    pub completion: Option<Completion>,
    pub hover: Option<Hover>,
    pub status: Option<(String, Instant)>,
    pub lsp: Manager,
    lsp_warned: HashSet<String>,
    pub diagnostics: HashMap<PathBuf, Vec<Diag>>,
    pub files: HashMap<PathBuf, Vec<String>>,
    indexing: Option<PathBuf>,
    pub search: Option<String>,
    clip: String,
    clip_line: bool,
    /// Text the GUI should put on the system clipboard.
    pub pending_copy: Option<String>,
    /// Sidebar clipboard: paths and whether they were cut.
    pub file_clip: Option<(Vec<PathBuf>, bool)>,
    jumps: Vec<(PathBuf, Pos)>,
    closed: Vec<PathBuf>,
    pub quit: bool,
    /// Visible editor lines (set by the GUI), for paging and centering.
    pub page: usize,
    pub font_size: f32,
    /// Highlighting didn't finish within the frame budget; redraw again soon.
    pub hl_pending: bool,
    pub tree: Tree,
    tree_revealed: Option<PathBuf>,
    theme_before: Option<String>,
    /// Open menu (menu bar navigation), if any.
    pub menu: Option<MenuState>,
    pub menu_bar_visible: bool,
    tx: EventTx,
}

fn kind_name(k: i64) -> &'static str {
    const N: [&str; 27] = [
        "", "file", "mod", "ns", "pkg", "class", "method", "prop", "field", "ctor", "enum",
        "iface", "fn", "var", "const", "str", "num", "bool", "arr", "obj", "key", "null",
        "variant", "struct", "event", "op", "type",
    ];
    N.get(k as usize).copied().unwrap_or("")
}

fn completion_kind(k: i64) -> &'static str {
    const N: [&str; 26] = [
        "", "txt", "meth", "fn", "ctor", "field", "var", "class", "iface", "mod", "prop", "unit",
        "val", "enum", "kw", "snip", "color", "file", "ref", "dir", "variant", "const", "struct",
        "event", "op", "type",
    ];
    N.get(k as usize).copied().unwrap_or("")
}

fn kind_num<T: serde::Serialize>(k: &T) -> i64 {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// Strip LSP snippet syntax (`${1:name}`, `$0`) down to plain text.
fn strip_snippet(s: &str) -> String {
    let mut out = String::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' && i + 1 < cs.len() {
            out.push(cs[i + 1]);
            i += 2;
        } else if c == '$' && i + 1 < cs.len() && cs[i + 1].is_ascii_digit() {
            i += 1;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
        } else if c == '$' && i + 1 < cs.len() && cs[i + 1] == '{' {
            i += 2;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i < cs.len() && (cs[i] == ':' || cs[i] == '|') {
                i += 1;
            }
            let mut depth = 1;
            while i < cs.len() {
                match cs[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    '|' | ',' if depth == 1 && out.ends_with(|ch: char| ch != ' ') => {}
                    ch => out.push(ch),
                }
                i += 1;
            }
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn markdown_to_lines(s: &str) -> Vec<String> {
    let mut out = vec![];
    for l in s.lines() {
        if l.trim_start().starts_with("```") {
            continue;
        }
        out.push(l.replace("&nbsp;", " ").replace("\\_", "_"));
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.trim().is_empty()) {
        out.remove(0);
    }
    out
}

impl App {
    pub fn new(root: PathBuf, config: Config, tx: EventTx) -> Self {
        let syn = Syntax::load(&config.editor.theme);
        let mut tree = Tree::new();
        tree.set_root(root.clone(), false);
        let mut app = App {
            root,
            focus: Focus::Editor,
            sidebar_visible: config.editor.show_sidebar,
            show_hidden: false,
            buffers: vec![],
            cur: 0,
            next_buf_id: 1,
            syn,
            modal: None,
            completion: None,
            hover: None,
            status: None,
            lsp: Manager::new(tx.clone()),
            lsp_warned: HashSet::new(),
            diagnostics: HashMap::new(),
            files: HashMap::new(),
            indexing: None,
            search: None,
            clip: String::new(),
            clip_line: false,
            pending_copy: None,
            file_clip: None,
            jumps: vec![],
            closed: vec![],
            quit: false,
            page: 30,
            font_size: config.editor.font_size,
            hl_pending: false,
            tree,
            tree_revealed: None,
            theme_before: None,
            menu: None,
            menu_bar_visible: config.editor.show_menu_bar,
            config,
            tx,
        };
        if let Some(e) = app.config.error.clone() {
            app.msg(format!("Config error: {e}"));
        }
        app
    }

    pub fn event_tx(&self) -> EventTx {
        self.tx.clone()
    }

    pub fn msg(&mut self, s: impl Into<String>) {
        self.status = Some((s.into(), Instant::now()));
    }

    /// Expire the status message; returns true if the UI needs a repaint.
    pub fn tick(&mut self) -> bool {
        if let Some((_, t)) = &self.status
            && t.elapsed() > Duration::from_secs(6)
        {
            self.status = None;
            return true;
        }
        false
    }

    pub fn buf(&self) -> Option<&Buffer> {
        self.buffers.get(self.cur)
    }

    pub fn buf_mut(&mut self) -> Option<&mut Buffer> {
        self.buffers.get_mut(self.cur)
    }

    fn buf_index(&self, id: usize) -> Option<usize> {
        self.buffers.iter().position(|b| b.id == id)
    }

    pub fn title(&self) -> String {
        let root = self.root.file_name().unwrap_or_default().to_string_lossy();
        match self.buf() {
            Some(b) => format!(
                "{}{} — {root}",
                b.name(),
                if b.modified { " •" } else { "" }
            ),
            None => format!("{root} — farsub"),
        }
    }

    pub fn any_modified(&self) -> bool {
        self.buffers.iter().any(|b| b.modified)
    }

    fn refresh_tree(&mut self) {
        self.tree.refresh();
        self.files.clear();
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .display()
            .to_string()
    }

    fn resolve(&self, base: &Path, s: &str) -> PathBuf {
        let s = s.trim();
        if s == "~" {
            return std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
        }
        if let Some(rest) = s.strip_prefix("~/") {
            return std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(rest);
        }
        let p = PathBuf::from(s);
        if p.is_absolute() { p } else { base.join(p) }
    }

    // ===================================================================== input entry points

    /// A non-text key press. Returns false if nothing handled it (so an Alt+letter can
    /// still produce its character via the text event).
    pub fn on_key(&mut self, k: Key) -> bool {
        if k.is_text_key() {
            return false;
        }
        if self.menu.is_some() {
            self.menu_key(k);
            return true;
        }
        if self.modal.is_some() {
            self.modal_key(k);
            return true;
        }
        if self.hover.is_some() && k.code == KeyCode::Esc {
            self.hover = None;
            return true;
        }
        let sidebar = self.focus == Focus::Sidebar && self.sidebar_visible;
        let own = if sidebar {
            keymap::lookup(k, Ctx::Sidebar)
        } else {
            None
        };
        if let Some(cmd) = own.or_else(|| keymap::lookup(k, Ctx::Global)) {
            self.run(cmd);
            return true;
        }
        if let (KeyCode::Char(c @ '1'..='9'), true, false) = (k.code, k.ctrl, k.shift) {
            let i = c as usize - '1' as usize;
            if i < self.buffers.len() {
                self.cur = i;
                self.focus = Focus::Editor;
            }
            return true;
        }
        if sidebar {
            return false;
        }
        self.editor_key(k)
    }

    /// Typed text (already composed by the OS: shifted chars, dead keys, IME).
    pub fn on_text(&mut self, s: &str) {
        for c in s.chars().filter(|c| !c.is_control()) {
            if self.menu.is_some() {
                self.menu_char(c);
                continue;
            }
            match &mut self.modal {
                Some(Modal::Keymap(v)) => {
                    v.query.push(c);
                    v.sel = 0;
                    v.scroll = 0;
                }
                Some(Modal::Palette(p)) => {
                    p.query.push(c);
                    self.palette_query_changed();
                }
                Some(Modal::Prompt(p)) => {
                    let b = crate::editor::byte_idx(&p.input, p.cursor);
                    p.input.insert(b, c);
                    p.cursor += 1;
                    self.prompt_changed();
                }
                Some(Modal::Confirm(_)) => {
                    let Some(Modal::Confirm(cf)) = self.modal.take() else {
                        return;
                    };
                    match c {
                        'y' | 'Y' => self.confirm(cf.kind),
                        'n' | 'N' => {}
                        _ => self.modal = Some(Modal::Confirm(cf)),
                    }
                }
                None if self.focus == Focus::Sidebar && self.sidebar_visible => self.tree_char(c),
                None => self.editor_text(c),
            }
        }
    }

    pub fn on_paste(&mut self, s: &str) {
        match &mut self.modal {
            Some(Modal::Prompt(p)) => {
                let line = s.lines().next().unwrap_or("");
                let b = crate::editor::byte_idx(&p.input, p.cursor);
                p.input.insert_str(b, line);
                p.cursor += char_len(line);
                self.prompt_changed();
            }
            Some(Modal::Palette(p)) => {
                p.query.push_str(s.lines().next().unwrap_or(""));
                self.palette_query_changed();
            }
            Some(_) => {}
            None if self.focus == Focus::Sidebar && self.sidebar_visible => {
                self.run(Command::SidebarPaste)
            }
            None => self.paste_text(s.to_string()),
        }
    }

    pub fn on_copy(&mut self, cut: bool) {
        if self.modal.is_some() {
            return;
        }
        let cmd = match (self.focus == Focus::Sidebar && self.sidebar_visible, cut) {
            (true, false) => Command::SidebarCopy,
            (true, true) => Command::SidebarCut,
            (false, false) => Command::Copy,
            (false, true) => Command::Cut,
        };
        self.run(cmd);
    }

    fn paste_text(&mut self, text: String) {
        let line_mode = self.clip_line && text == self.clip;
        let Some(b) = self.buf_mut() else { return };
        if b.read_only {
            return;
        }
        if line_mode && b.selection().is_none() {
            // A whole line copied without selection is pasted above the current line.
            let c = b.cursor;
            b.goto(Pos::new(c.line, 0));
            b.insert_text(&text);
            b.goto(Pos::new(c.line + 1, c.col));
        } else {
            b.insert_text(&text);
        }
        self.completion = None;
    }

    pub fn on_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Lsp(cid, v) => self.on_lsp(cid, v),
            AppEvent::LspExit(cid) => {
                if let Some(c) = self.lsp.clients.get_mut(&cid)
                    && c.alive
                {
                    c.alive = false;
                    let name = c.cmd.clone();
                    self.msg(format!("{name} exited"));
                }
            }
            AppEvent::Files(root, list) => {
                self.indexing = None;
                self.files.insert(root.clone(), list);
                if let Some(Modal::Palette(p)) = &mut self.modal
                    && p.mode == Mode::Files
                {
                    p.loading = false;
                    let items = Self::file_items(&root, &self.files[&root]);
                    p.set_items(items);
                }
            }
        }
    }

    // ===================================================================== modal

    fn prompt(&mut self, title: &str, def: &str, kind: PromptKind) {
        self.completion = None;
        self.hover = None;
        self.modal = Some(Modal::Prompt(Prompt {
            title: title.to_string(),
            input: def.to_string(),
            cursor: char_len(def),
            kind,
        }));
    }

    fn modal_key(&mut self, k: Key) {
        match self.modal.take() {
            Some(Modal::Palette(p)) => {
                self.modal = Some(Modal::Palette(p));
                self.palette_key(k);
            }
            Some(Modal::Prompt(p)) => self.prompt_key(k, p),
            Some(Modal::Keymap(v)) => self.keymap_key(k, v),
            Some(Modal::Confirm(c)) => match k.code {
                KeyCode::Enter => self.confirm(c.kind),
                KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Confirm(c)),
            },
            None => {}
        }
    }

    fn confirm(&mut self, kind: ConfirmKind) {
        match kind {
            ConfirmKind::Trash(paths) => match fileops::trash(&paths) {
                Ok(()) => {
                    self.tree.marked.clear();
                    self.refresh_tree();
                    self.msg(format!("Moved {} item(s) to the Trash", paths.len()));
                }
                Err(e) => self.msg(format!("Move to Trash failed: {e}")),
            },
            ConfirmKind::CloseBuffer(id) => {
                if let Some(i) = self.buf_index(id) {
                    self.close_buffer_at(i);
                }
            }
            ConfirmKind::Quit => self.quit = true,
        }
    }

    fn prompt_key(&mut self, k: Key, mut p: Prompt) {
        let is_find = matches!(p.kind, PromptKind::Find { .. });
        match k.code {
            KeyCode::Esc => {
                if let PromptKind::Find { origin } = p.kind
                    && let Some(b) = self.buf_mut()
                {
                    b.goto(origin);
                }
                return;
            }
            KeyCode::Enter if is_find => {
                // Sublime: Enter = next match, Shift+Enter = previous; the panel stays open.
                self.search = Some(p.input.clone());
                self.find_next(!k.shift);
            }
            KeyCode::Enter => return self.prompt_accept(p),
            KeyCode::Backspace => {
                if p.cursor > 0 {
                    let n = if k.ctrl || k.alt {
                        // Delete back to the previous path separator / word.
                        let chars: Vec<char> = p.input.chars().take(p.cursor).collect();
                        let mut i = p.cursor - 1;
                        while i > 0 && !matches!(chars[i - 1], '/' | ' ') {
                            i -= 1;
                        }
                        p.cursor - i
                    } else {
                        1
                    };
                    for _ in 0..n {
                        let b = crate::editor::byte_idx(&p.input, p.cursor - 1);
                        p.input.remove(b);
                        p.cursor -= 1;
                    }
                }
            }
            KeyCode::Delete => {
                if p.cursor < char_len(&p.input) {
                    let b = crate::editor::byte_idx(&p.input, p.cursor);
                    p.input.remove(b);
                }
            }
            KeyCode::Left => p.cursor = p.cursor.saturating_sub(1),
            KeyCode::Right => p.cursor = (p.cursor + 1).min(char_len(&p.input)),
            KeyCode::Home => p.cursor = 0,
            KeyCode::End => p.cursor = char_len(&p.input),
            KeyCode::Char('a') if k.ctrl => p.cursor = 0,
            KeyCode::Char('u') if k.ctrl => {
                p.input.clear();
                p.cursor = 0;
            }
            KeyCode::Tab if p.kind.is_path() => {
                let base = match &p.kind {
                    PromptKind::NewFile(d) | PromptKind::NewFolder(d) => d.clone(),
                    _ => self.root.clone(),
                };
                p.input = complete_path(&base, &p.input);
                p.cursor = char_len(&p.input);
            }
            KeyCode::F(3) | KeyCode::Down if is_find => {
                self.search = Some(p.input.clone());
                self.find_next(!k.shift);
            }
            KeyCode::Up if is_find => {
                self.search = Some(p.input.clone());
                self.find_next(false);
            }
            _ => {}
        }
        let changed = matches!(
            k.code,
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Char('u')
        );
        self.modal = Some(Modal::Prompt(p));
        if changed {
            self.prompt_changed();
        }
    }

    /// Incremental find while typing in the find panel.
    fn prompt_changed(&mut self) {
        let Some(Modal::Prompt(p)) = &self.modal else {
            return;
        };
        let PromptKind::Find { origin } = p.kind else {
            return;
        };
        let q = p.input.clone();
        self.search = Some(q.clone());
        let pg = self.page;
        if let Some(b) = self.buf_mut() {
            if let Some((s, e)) = b.find(&q, origin, true, false) {
                b.anchor = Some(s);
                b.cursor = e;
                if b.cursor.line < b.scroll_y || b.cursor.line >= b.scroll_y + pg {
                    center(b, pg);
                }
            } else {
                b.goto(origin);
            }
        }
    }

    fn prompt_accept(&mut self, p: Prompt) {
        let input = p.input.trim().to_string();
        match p.kind {
            PromptKind::NewFile(dir) | PromptKind::NewFolder(dir) if input.is_empty() => {
                let _ = dir;
            }
            PromptKind::NewFile(dir) => {
                let path = self.resolve(&dir, &input);
                if path.exists() {
                    return self.msg(format!("{} already exists", self.rel(&path)));
                }
                if let Some(parent) = path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    return self.msg(format!("Create folder failed: {e}"));
                }
                if let Err(e) = std::fs::write(&path, "") {
                    return self.msg(format!("Create file failed: {e}"));
                }
                self.refresh_tree();
                self.tree.reveal(&path);
                self.open_file(&path, true);
            }
            PromptKind::NewFolder(dir) => {
                let path = self.resolve(&dir, &input);
                match std::fs::create_dir_all(&path) {
                    Ok(()) => {
                        self.refresh_tree();
                        self.tree.reveal(&path);
                        self.tree.expanded.insert(path);
                        self.tree.rebuild();
                    }
                    Err(e) => self.msg(format!("Create folder failed: {e}")),
                }
            }
            PromptKind::RenameMove(src) => {
                if input.is_empty() {
                    return;
                }
                let dst = self.resolve(&self.root.clone(), &input);
                if dst == src {
                    return;
                }
                if dst.exists() {
                    return self.msg(format!("{} already exists", self.rel(&dst)));
                }
                match fileops::move_path(&src, &dst) {
                    Ok(()) => {
                        self.rebind_paths(&src, &dst);
                        self.refresh_tree();
                        self.tree.reveal(&dst);
                        self.msg(format!("Moved to {}", self.rel(&dst)));
                    }
                    Err(e) => self.msg(format!("Rename failed: {e}")),
                }
            }
            PromptKind::SaveAs => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.root.clone(), &input);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let cur = self.cur;
                self.close_lsp_doc(cur);
                self.set_buffer_path(cur, path);
                self.save_current();
                self.lsp_open(cur);
                self.refresh_tree();
            }
            PromptKind::OpenPath => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.root.clone(), &input);
                let path = path.canonicalize().unwrap_or(path);
                if path.is_dir() {
                    self.set_root(path);
                } else if path.is_file() {
                    self.open_file(&path, true);
                } else {
                    self.msg(format!("No such file or folder: {input}"));
                }
            }
            PromptKind::Find { .. } => {}
            PromptKind::ReplaceFind => {
                if !input.is_empty() {
                    let f = p.input.clone();
                    self.prompt(
                        &format!("Replace '{f}' with"),
                        "",
                        PromptKind::ReplaceWith(f),
                    );
                }
            }
            PromptKind::ReplaceWith(f) => {
                let n = self
                    .buf_mut()
                    .map(|b| b.replace_all(&f, &p.input))
                    .unwrap_or(0);
                self.msg(format!("Replaced {n} occurrence(s)"));
            }
            PromptKind::RenameSymbol => {
                if input.is_empty() {
                    return;
                }
                self.lsp_position_request(
                    "textDocument/rename",
                    json!({"newName": input}),
                    Pending::Rename,
                    "renameProvider",
                );
            }
        }
    }

    pub fn set_root(&mut self, root: PathBuf) {
        self.root = root.clone();
        self.tree.set_root(root, self.show_hidden);
        self.tree_revealed = None;
        self.sidebar_visible = true;
        self.msg(format!("Opened folder {}", self.root.display()));
    }

    // ===================================================================== sidebar

    fn focus_sidebar(&mut self) {
        self.sidebar_visible = true;
        self.focus = Focus::Sidebar;
        self.completion = None;
        self.hover = None;
        self.tree.set_root(self.root.clone(), self.show_hidden);
        let cur = self.buf().and_then(|b| b.path.clone());
        if let Some(p) = cur
            && self.tree_revealed.as_ref() != Some(&p)
        {
            self.tree.reveal(&p);
            self.tree_revealed = Some(p);
        }
    }

    pub fn tree_click(&mut self, i: usize, open: bool) {
        self.focus = Focus::Sidebar;
        if i < self.tree.nodes.len() {
            self.tree.sel = i;
            if open {
                self.tree_open(false);
            }
        }
    }

    fn tree_open(&mut self, keep_focus: bool) {
        let Some(n) = self.tree.selected() else {
            return;
        };
        if n.is_dir {
            self.tree.toggle();
            return;
        }
        let path = n.path.clone();
        if self.open_file(&path, true).is_some() {
            self.tree_revealed = Some(path);
            if keep_focus {
                self.focus = Focus::Sidebar;
            }
        }
    }

    /// Single-letter sidebar commands (arrive as text so Shift+A gives 'A').
    fn tree_char(&mut self, c: char) {
        if let Some(cmd) = keymap::lookup(keymap::key_for_char(c), Ctx::Sidebar) {
            self.run(cmd);
        }
    }

    fn sidebar_nav(&mut self, cmd: Command) {
        use Command::*;
        let page = self.page.max(1) as isize;
        match cmd {
            SidebarUp => self.tree.move_sel(-1),
            SidebarDown => self.tree.move_sel(1),
            SidebarPageUp => self.tree.move_sel(-page),
            SidebarPageDown => self.tree.move_sel(page),
            SidebarTop => self.tree.sel = 0,
            SidebarBottom => self.tree.sel = self.tree.nodes.len().saturating_sub(1),
            SidebarExpand => self.tree.expand(),
            SidebarCollapse => self.tree.collapse(),
            SidebarOpen => self.tree_open(false),
            SidebarOpenKeepFocus => self.tree_open(true),
            SidebarMark => {
                self.tree.toggle_mark();
                self.tree.move_sel(1);
            }
            SidebarMarkUp | SidebarMarkDown => {
                if self.tree.marked.is_empty() {
                    self.tree.toggle_mark();
                }
                self.tree
                    .move_sel(if cmd == SidebarMarkUp { -1 } else { 1 });
                if let Some(p) = self.tree.selected_path().map(Path::to_path_buf) {
                    self.tree.marked.insert(p);
                }
            }
            SidebarEscape => {
                if !self.tree.marked.is_empty() {
                    self.tree.marked.clear();
                } else if self.file_clip.is_some() {
                    self.file_clip = None;
                } else {
                    self.focus = Focus::Editor;
                }
            }
            _ => {}
        }
    }

    fn run_sidebar(&mut self, cmd: Command) {
        use Command::*;
        let dir = self.tree.target_dir();
        let rel_dir = |app: &App| {
            let r = app.rel(&dir);
            if r.is_empty() {
                "./".to_string()
            } else {
                format!("{r}/")
            }
        };
        match cmd {
            SidebarNewFile => {
                let t = format!(
                    "New file in {}  (use a/b.txt for subfolders)",
                    rel_dir(self)
                );
                self.prompt(&t, "", PromptKind::NewFile(dir.clone()));
            }
            SidebarNewFolder => {
                let t = format!("New folder in {}", rel_dir(self));
                self.prompt(&t, "", PromptKind::NewFolder(dir.clone()));
            }
            SidebarRename => {
                let Some(src) = self.tree.selected_path().map(Path::to_path_buf) else {
                    return;
                };
                let rel = self.rel(&src);
                self.prompt(
                    "Rename / move to (relative to the project)",
                    &rel,
                    PromptKind::RenameMove(src),
                );
                // Put the cursor before the extension, like Sublime/VS Code.
                if let Some(Modal::Prompt(p)) = &mut self.modal
                    && let Some(dot) = p.input.rfind('.')
                    && p.input.rfind('/').is_none_or(|s| s < dot)
                    && dot > 0
                {
                    p.cursor = char_len(&p.input[..dot]);
                }
            }
            SidebarDelete => {
                let t = self.tree.targets();
                if t.is_empty() {
                    return;
                }
                let what = if t.len() == 1 {
                    self.rel(&t[0])
                } else {
                    format!("{} items", t.len())
                };
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Move {what} to the Trash?"),
                    kind: ConfirmKind::Trash(t),
                }));
            }
            SidebarCopy | SidebarCut => {
                let t = self.tree.targets();
                if t.is_empty() {
                    return;
                }
                let cut = cmd == SidebarCut;
                self.msg(format!(
                    "{} {} item(s) — select a folder and press p to paste",
                    if cut { "Cut" } else { "Copied" },
                    t.len()
                ));
                self.file_clip = Some((t, cut));
                self.tree.marked.clear();
            }
            SidebarPaste => {
                let Some((srcs, cut)) = self.file_clip.clone() else {
                    return self.msg("Nothing to paste — copy (c) or cut (x) files first");
                };
                let mut last = None;
                let mut n = 0;
                for src in &srcs {
                    let name = src
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if cut && src.parent() == Some(dir.as_path()) {
                        continue;
                    }
                    let dst = fileops::unique_dest(&dir, &name);
                    let res = if cut {
                        fileops::move_path(src, &dst).map(|_| self.rebind_paths(src, &dst))
                    } else {
                        fileops::copy_recursive(src, &dst)
                    };
                    match res {
                        Ok(()) => {
                            n += 1;
                            last = Some(dst);
                        }
                        Err(e) => self.msg(format!("{}: {e}", self.rel(src))),
                    }
                }
                if cut {
                    self.file_clip = None;
                }
                self.tree.expand_path(&dir);
                self.tree.expanded.insert(dir.clone());
                self.refresh_tree();
                if let Some(p) = last {
                    self.tree.reveal(&p);
                }
                self.msg(format!(
                    "{} {n} item(s) into {}",
                    if cut { "Moved" } else { "Pasted" },
                    rel_dir(self)
                ));
            }
            SidebarDuplicate => {
                let t = self.tree.targets();
                let mut last = None;
                for src in &t {
                    let parent = src.parent().unwrap_or(&self.root).to_path_buf();
                    let name = src
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let dst = fileops::unique_dest(&parent, &name);
                    match fileops::copy_recursive(src, &dst) {
                        Ok(()) => last = Some(dst),
                        Err(e) => self.msg(format!("Duplicate failed: {e}")),
                    }
                }
                self.tree.marked.clear();
                self.refresh_tree();
                if let Some(p) = last {
                    self.tree.reveal(&p);
                }
            }
            SidebarCopyPath => {
                let t = self.tree.targets();
                let s = t
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !s.is_empty() {
                    self.msg(format!("Copied path: {s}"));
                    self.pending_copy = Some(s);
                }
            }
            _ => {}
        }
    }

    /// Point open buffers at a file's new location after a rename/move (also for folders).
    fn rebind_paths(&mut self, src: &Path, dst: &Path) {
        for i in 0..self.buffers.len() {
            let Some(p) = self.buffers[i].path.clone() else {
                continue;
            };
            if let Ok(rest) = p.strip_prefix(src) {
                let np = if rest.as_os_str().is_empty() {
                    dst.to_path_buf()
                } else {
                    dst.join(rest)
                };
                self.close_lsp_doc(i);
                self.set_buffer_path(i, np);
                self.lsp_open(i);
            }
        }
        if let Some(r) = &self.tree_revealed
            && r.starts_with(src)
        {
            self.tree_revealed = None;
        }
    }

    fn set_buffer_path(&mut self, i: usize, path: PathBuf) {
        let first = self.buffers[i].lines[0].clone();
        let lang = self.config.language_for(&path).cloned();
        let syntax = lang
            .as_ref()
            .and_then(|l| l.syntax.as_deref())
            .and_then(|s| self.syn.find_by_name(s))
            .unwrap_or_else(|| self.syn.detect(Some(&path), &first));
        let b = &mut self.buffers[i];
        b.set_path(path);
        b.apply_language(lang.as_ref(), &self.config.editor);
        b.hl.reset(syntax);
    }

    // ===================================================================== settings

    fn open_settings(&mut self) {
        let Some(path) = self.config.path.clone().or_else(config::config_path) else {
            return self.msg("No config location (set $HOME or $FARSUB_CONFIG)");
        };
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, config::TEMPLATE) {
                return self.msg(format!("Could not create {}: {e}", path.display()));
            }
        }
        self.open_file(&path, true);
    }

    fn reload_config(&mut self) {
        let new = Config::load_from(self.config.path.clone());
        if let Some(e) = &new.error {
            return self.msg(format!("Config error: {e}"));
        }
        let langs_changed =
            format!("{:?}", new.languages) != format!("{:?}", self.config.languages);
        self.config = new;
        if !self.syn.set_theme(&self.config.editor.theme) {
            self.msg(format!("Unknown theme '{}'", self.config.editor.theme));
        }
        for b in &mut self.buffers {
            b.hl.invalidate(0);
        }
        self.font_size = self.config.editor.font_size;
        for i in 0..self.buffers.len() {
            if let Some(p) = self.buffers[i].path.clone() {
                let lang = self.config.language_for(&p).cloned();
                let b = &mut self.buffers[i];
                b.apply_language(lang.as_ref(), &self.config.editor);
            }
        }
        if langs_changed {
            self.restart_lsp();
        }
        if self
            .status
            .as_ref()
            .is_none_or(|s| !s.0.starts_with("Unknown theme"))
        {
            self.msg("Settings reloaded");
        }
    }

    fn restart_lsp(&mut self) {
        self.lsp.restart();
        self.lsp_warned.clear();
        self.diagnostics.clear();
        for i in 0..self.buffers.len() {
            self.buffers[i].lsp_client = None;
            self.lsp_open(i);
        }
    }

    fn open_palette(&mut self, mode: Mode) {
        let mut p = Palette::new(mode, Palette::prefix(mode));
        if matches!(mode, Mode::Symbols | Mode::Line)
            && let Some(b) = self.buf()
        {
            p.origin = Some((self.cur, b.cursor, b.scroll_y));
        }
        self.modal = Some(Modal::Palette(p));
        self.populate_palette();
    }

    fn palette(&mut self) -> Option<&mut Palette> {
        match &mut self.modal {
            Some(Modal::Palette(p)) => Some(p),
            _ => None,
        }
    }

    fn file_root(&self) -> PathBuf {
        self.root.clone()
    }

    fn file_items(root: &Path, files: &[String]) -> Vec<Item> {
        files
            .iter()
            .map(|f| Item {
                label: f.clone(),
                detail: String::new(),
                action: Action::Open(root.join(f), None),
            })
            .collect()
    }

    fn start_indexing(&mut self, root: PathBuf) {
        if self.indexing.as_ref() == Some(&root) {
            return;
        }
        self.indexing = Some(root.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let walker = ignore::WalkBuilder::new(&root)
                .hidden(true)
                .git_ignore(true)
                .build();
            for e in walker.flatten() {
                if e.file_type().is_some_and(|t| t.is_file())
                    && let Ok(rel) = e.path().strip_prefix(&root)
                {
                    out.push(rel.to_string_lossy().into_owned());
                }
                if out.len() >= 200_000 {
                    break;
                }
            }
            let _ = tx.send(AppEvent::Files(root, out));
        });
    }

    fn populate_palette(&mut self) {
        let Some(mode) = self.palette().map(|p| p.mode) else {
            return;
        };
        let mut loading = false;
        let mut title = String::new();
        let items: Vec<Item> = match mode {
            Mode::Files => {
                let root = self.file_root();
                title = format!(
                    "Files in {}",
                    root.file_name().unwrap_or_default().to_string_lossy()
                );
                if let Some(list) = self.files.get(&root) {
                    Self::file_items(&root, list)
                } else {
                    self.start_indexing(root);
                    loading = true;
                    vec![]
                }
            }
            Mode::Commands => COMMANDS
                .iter()
                .filter(|c| c.2)
                .map(|(c, name, _)| Item {
                    label: name.to_string(),
                    detail: keymap::shortcut(*c),
                    action: Action::Command(*c),
                })
                .collect(),
            Mode::Symbols => {
                title = "Symbols".into();
                let items = self.buf().map(heuristic_symbols).unwrap_or_default();
                if self.request_doc_symbols() {
                    loading = true;
                }
                items
            }
            Mode::Line => {
                title = "Goto line".into();
                vec![]
            }
            Mode::WorkspaceSymbols => {
                title = "Project symbols (type to search)".into();
                vec![]
            }
            Mode::Buffers => self
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| Item {
                    label: b.name(),
                    detail: b
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                        + if b.modified { "  (modified)" } else { "" },
                    action: Action::Buffer(i),
                })
                .collect(),
            Mode::Themes => {
                let cur = self.config.editor.theme.clone();
                self.syn
                    .theme_names()
                    .into_iter()
                    .map(|n| Item {
                        detail: if n == cur {
                            "current".into()
                        } else {
                            String::new()
                        },
                        action: Action::Theme(n.clone()),
                        label: n,
                    })
                    .collect()
            }
            Mode::Locations => return,
        };
        if let Some(p) = self.palette() {
            p.loading = loading;
            if !title.is_empty() {
                p.title = title;
            }
            p.set_items(items);
        }
        if mode == Mode::WorkspaceSymbols {
            self.request_workspace_symbols();
        }
    }

    fn palette_query_changed(&mut self) {
        let Some(p) = self.palette() else { return };
        let new_mode = p.mode_for_query();
        if new_mode != p.mode {
            p.mode = new_mode;
            self.populate_palette();
        } else {
            p.refilter();
            if new_mode == Mode::WorkspaceSymbols {
                self.request_workspace_symbols();
            }
        }
        self.palette_preview();
    }

    fn palette_preview(&mut self) {
        let Some(Modal::Palette(p)) = &self.modal else {
            return;
        };
        if p.mode == Mode::Themes {
            if let Some(Action::Theme(n)) = p.selected().map(|i| i.action.clone()) {
                self.apply_theme(&n);
            }
            return;
        }
        let target = match p.mode {
            Mode::Symbols => match p.selected().map(|i| &i.action) {
                Some(Action::Goto(pos)) => Some(*pos),
                _ => None,
            },
            Mode::Line => p
                .needle()
                .trim()
                .parse::<usize>()
                .ok()
                .map(|n| Pos::new(n.saturating_sub(1), 0)),
            _ => None,
        };
        if let (Some(pos), Some((bi, _, _))) = (target, p.origin)
            && let Some(b) = self.buffers.get_mut(bi)
        {
            b.goto(pos);
            center(b, self.page);
        }
    }

    fn palette_key(&mut self, k: Key) {
        let Some(p) = self.palette() else { return };
        match k.code {
            KeyCode::Esc => {
                let origin = p.origin;
                if p.mode == Mode::Themes
                    && let Some(t) = self.theme_before.take()
                {
                    self.apply_theme(&t);
                    self.modal = None;
                    return;
                }
                if let Some((bi, pos, sy)) = origin
                    && let Some(b) = self.buffers.get_mut(bi)
                {
                    b.goto(pos);
                    b.scroll_y = sy;
                }
                self.modal = None;
            }
            KeyCode::Enter => {
                let Some(Modal::Palette(p)) = self.modal.take() else {
                    return;
                };
                self.palette_accept(p);
            }
            KeyCode::Up => {
                p.move_sel(-1);
                self.palette_preview();
            }
            KeyCode::Down | KeyCode::Tab => {
                p.move_sel(1);
                self.palette_preview();
            }
            KeyCode::Char('p') if k.ctrl && p.mode == Mode::Files => {
                p.move_sel(1);
            }
            KeyCode::PageUp => {
                p.move_sel(-10);
                self.palette_preview();
            }
            KeyCode::PageDown => {
                p.move_sel(10);
                self.palette_preview();
            }
            KeyCode::Backspace if !k.ctrl => {
                if p.query.pop().is_none() && p.fixed {
                    return;
                }
                self.palette_query_changed();
            }
            KeyCode::Backspace if k.ctrl => {
                p.query.clear();
                self.palette_query_changed();
            }
            _ => {}
        }
    }

    fn palette_accept(&mut self, p: Palette) {
        if p.mode == Mode::Line {
            if let Ok(n) = p.needle().trim().parse::<usize>()
                && let Some(b) = self.buf_mut()
            {
                b.goto(Pos::new(n.saturating_sub(1), 0));
                let pg = self.page;
                if let Some(b) = self.buf_mut() {
                    center(b, pg);
                }
            }
            return;
        }
        let line = p.file_line();
        let Some(item) = p.selected().cloned() else {
            return;
        };
        match item.action {
            Action::Command(c) => self.run(c),
            Action::Open(path, pos) => {
                let pos = pos.or(line.map(|l| Pos::new(l - 1, 0)));
                self.goto_location(&path, pos);
            }
            Action::Goto(pos) => {
                self.focus = Focus::Editor;
                self.push_jump();
                if let Some((bi, _, _)) = p.origin {
                    self.cur = bi;
                }
                let pg = self.page;
                if let Some(b) = self.buf_mut() {
                    b.goto(pos);
                    center(b, pg);
                }
            }
            Action::Buffer(i) => {
                if i < self.buffers.len() {
                    self.cur = i;
                    self.focus = Focus::Editor;
                }
            }
            Action::Theme(n) => {
                self.apply_theme(&n);
                self.theme_before = None;
                self.msg(format!(
                    "Theme: {n} (set theme = \"{n}\" in settings to keep it)"
                ));
                self.config.editor.theme = n;
            }
            Action::None => {}
        }
    }

    // ===================================================================== files & buffers

    pub fn open_file(&mut self, path: &Path, focus: bool) -> Option<usize> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(i) = self
            .buffers
            .iter()
            .position(|b| b.path.as_deref() == Some(&path))
        {
            if focus {
                self.cur = i;
                self.focus = Focus::Editor;
            }
            return Some(i);
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.msg(format!("open {}: {e}", path.display()));
                return None;
            }
        };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            self.msg("Binary file — not opening");
            return None;
        }
        let text = String::from_utf8_lossy(&bytes);
        let first = text.lines().next().unwrap_or("");
        let syntax = self.syn.detect(Some(&path), first);
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let b = Buffer::from_text(id, Some(path.clone()), &text, HlCache::new(syntax));
        self.buffers.push(b);
        let i = self.buffers.len() - 1;
        self.set_buffer_path(i, path);
        if std::fs::metadata(&self.buffers[i].path.clone().unwrap())
            .is_ok_and(|m| m.permissions().readonly())
        {
            self.buffers[i].read_only = true;
        }
        if focus {
            self.cur = i;
            self.focus = Focus::Editor;
        }
        self.lsp_open(i);
        Some(i)
    }

    pub fn open_new_path(&mut self, path: PathBuf) {
        let syntax = self.syn.detect(Some(&path), "");
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let b = Buffer::from_text(id, Some(path.clone()), "", HlCache::new(syntax));
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        self.set_buffer_path(self.cur, path);
        self.focus = Focus::Editor;
    }

    fn goto_location(&mut self, path: &Path, pos: Option<Pos>) {
        self.push_jump();
        if let Some(i) = self.open_file(path, true)
            && let Some(pos) = pos
        {
            let pg = self.page;
            let b = &mut self.buffers[i];
            b.goto(pos);
            center(b, pg);
        }
    }

    fn push_jump(&mut self) {
        if let Some(b) = self.buf()
            && let Some(p) = &b.path
        {
            let e = (p.clone(), b.cursor);
            if self.jumps.last() != Some(&e) {
                self.jumps.push(e);
            }
            if self.jumps.len() > 100 {
                self.jumps.remove(0);
            }
        }
    }

    fn save_current(&mut self) {
        let Some(b) = self.buf_mut() else { return };
        if b.path.is_none() {
            self.prompt("Save as", "", PromptKind::SaveAs);
            return;
        }
        match b.save() {
            Ok(()) => {
                let name = b.name();
                let saved = b.path.clone();
                let lines = b.lines.len();
                let uri = path_to_uri(b.path.as_ref().unwrap());
                let cid = b.lsp_client;
                let text = b.text();
                self.sync_lsp();
                if let Some(c) = cid.and_then(|c| self.lsp.clients.get_mut(&c)) {
                    c.notify(
                        "textDocument/didSave",
                        json!({"textDocument": {"uri": uri}, "text": text}),
                    );
                }
                self.msg(format!("Saved {name} ({lines} lines)"));
                if saved.is_some() && saved == self.config.path {
                    self.reload_config();
                }
                self.tree.refresh();
            }
            Err(e) => self.msg(format!("Save failed: {e}")),
        }
    }

    fn close_buffer_at(&mut self, i: usize) {
        let b = self.buffers.remove(i);
        if let Some(p) = &b.path {
            self.closed.push(p.clone());
        }
        if let (Some(cid), Some(path)) = (b.lsp_client, &b.path)
            && let Some(c) = self.lsp.clients.get_mut(&cid)
        {
            c.notify(
                "textDocument/didClose",
                json!({"textDocument": {"uri": path_to_uri(path)}}),
            );
        }
        self.completion = None;
        self.hover = None;
        if self.buffers.is_empty() {
            self.cur = 0;
        } else if self.cur >= self.buffers.len() || self.cur > i {
            self.cur = self.cur.saturating_sub(1).min(self.buffers.len() - 1);
        }
    }

    // ===================================================================== editor

    fn editor_key_inner(&mut self, k: Key) {
        if self.buffers.is_empty() {
            return;
        }
        // Completion popup navigation.
        if self.completion.is_some() {
            match k.code {
                KeyCode::Up => return self.comp_move(-1),
                KeyCode::Down => return self.comp_move(1),
                KeyCode::PageUp => return self.comp_move(-8),
                KeyCode::PageDown => return self.comp_move(8),
                KeyCode::Enter | KeyCode::Tab if !k.ctrl && !k.shift => return self.comp_accept(),
                KeyCode::Esc => {
                    self.completion = None;
                    return;
                }
                _ => {}
            }
        }
        if self.hover.take().is_some() && k.code == KeyCode::Esc {
            return;
        }
        if let Some(cmd) = keymap::lookup(k, Ctx::Editor) {
            self.run(cmd);
            return;
        }
        let page = self.page.max(1) as isize;
        let Some(b) = self.buf_mut() else { return };
        let sel = k.shift;
        let word = k.alt || (k.ctrl && !k.sup);
        let typed: Option<char> = None;
        match k.code {
            KeyCode::Left if k.sup => b.move_home(sel),
            KeyCode::Right if k.sup => b.move_end(sel),
            KeyCode::Up if k.sup => b.move_doc(false, sel),
            KeyCode::Down if k.sup => b.move_doc(true, sel),
            KeyCode::Left => b.move_left(sel, word),
            KeyCode::Right => b.move_right(sel, word),
            KeyCode::Up if k.ctrl => b.scroll_y = b.scroll_y.saturating_sub(1),
            KeyCode::Down if k.ctrl => {
                b.scroll_y = (b.scroll_y + 1).min(b.lines.len().saturating_sub(1))
            }
            KeyCode::Up => b.move_vert(-1, sel),
            KeyCode::Down => b.move_vert(1, sel),
            KeyCode::Home if k.ctrl => b.move_doc(false, sel),
            KeyCode::End if k.ctrl => b.move_doc(true, sel),
            KeyCode::Home => b.move_home(sel),
            KeyCode::End => b.move_end(sel),
            KeyCode::PageUp => {
                b.move_vert(-page, sel);
                b.scroll_y = b.scroll_y.saturating_sub(page as usize);
            }
            KeyCode::PageDown => {
                b.move_vert(page, sel);
                b.scroll_y += page as usize;
            }
            KeyCode::Backspace if k.sup => b.delete_to_line_start(),
            KeyCode::Backspace if k.ctrl || k.alt => b.delete_word(false),
            KeyCode::Backspace => b.backspace(),
            KeyCode::Delete if k.ctrl || k.alt => b.delete_word(true),
            KeyCode::Delete => b.delete_forward(),
            KeyCode::Enter => b.newline(),
            KeyCode::Tab if k.shift => b.outdent(),
            KeyCode::Tab => {
                let multi = b.selection().is_some_and(|(a, c)| a.line != c.line);
                if multi { b.indent() } else { b.insert_indent() }
            }
            KeyCode::Esc => {
                b.anchor = None;
                self.completion = None;
            }
            _ => {}
        }
        self.after_editor_key(typed);
    }

    fn after_editor_key(&mut self, typed: Option<char>) {
        let Some(b) = self.buf() else { return };
        let (c, bid) = (b.cursor, b.id);
        if let Some(comp) = &self.completion {
            if comp.buf_id != bid || c.line != comp.start.line || c.col < comp.start.col {
                self.completion = None;
            } else if typed.is_some_and(|ch| !is_word(ch)) {
                self.completion = None;
            } else {
                self.comp_refilter();
            }
        }
        let Some(b) = self.buf() else { return };
        if let Some(ch) = typed {
            let trig = b
                .lsp_client
                .and_then(|c| self.lsp.clients.get(&c))
                .map(|c| c.triggers.clone())
                .unwrap_or_default();
            if trig.iter().any(|t| t.ends_with(ch)) && self.completion.is_none() {
                self.request_completion();
            } else if self.completion.is_none() && is_word(ch) {
                // Auto-trigger after a few identifier characters, like Sublime.
                let ws = b.word_start(b.cursor);
                if b.cursor.col - ws.col == 3 && b.lsp_client.is_some() {
                    self.request_completion();
                }
            }
        }
    }

    // ===================================================================== menu & keymap

    /// Whether a command can do anything right now (menus grey out the rest).
    pub fn enabled(&self, cmd: Command) -> bool {
        use Command::*;
        let has_buf = !self.buffers.is_empty();
        let lsp = self.buf().is_some_and(|b| b.lsp_client.is_some());
        match cmd {
            Save | SaveAs | CloseBuffer | Undo | Redo | Copy | Cut | Paste | SelectAll
            | SelectLine | SelectWord | DeleteLine | DuplicateLine | MoveLineUp | MoveLineDown
            | ToggleComment | Indent | Outdent | InsertLineAfter | InsertLineBefore | JoinLines
            | UpperCase | LowerCase | SortLines | Find | FindNext | FindPrev | Replace
            | GotoSymbol | GotoLine | NextBuffer | PrevBuffer | SwitchBuffer | NextDiagnostic
            | PrevDiagnostic | RevealInSidebar => has_buf,
            Completion | Hover | GotoDefinition | FindReferences | Rename | Format => lsp,
            SaveAll => self.any_modified(),
            ReopenClosed => !self.closed.is_empty(),
            JumpBack => !self.jumps.is_empty(),
            SidebarPaste => self.file_clip.is_some(),
            _ => true,
        }
    }

    /// Open a menu by index (mouse), or close it with `None`.
    pub fn set_menu(&mut self, menu: Option<usize>) {
        self.menu = menu.map(|m| MenuState {
            menu: m,
            item: first_enabled(self, m),
        });
    }

    pub fn menu_activate(&mut self, menu: usize, item: usize) {
        if let Some(Some(cmd)) = menu::items(menu).get(item).copied() {
            if self.enabled(cmd) {
                self.menu = None;
                self.run(cmd);
            }
        }
    }

    fn menu_key(&mut self, k: Key) {
        let Some(mut m) = self.menu else { return };
        let n_menus = menu::MENUS.len();
        let items = menu::items(m.menu);
        let step = |m: &MenuState, d: isize| -> usize {
            let n = items.len() as isize;
            let mut i = m.item as isize;
            for _ in 0..n {
                i = (i + d).rem_euclid(n);
                if let Some(Some(c)) = items.get(i as usize)
                    && self.enabled(*c)
                {
                    return i as usize;
                }
            }
            m.item
        };
        match k.code {
            KeyCode::Esc | KeyCode::F(10) => self.menu = None,
            KeyCode::Left => {
                m.menu = (m.menu + n_menus - 1) % n_menus;
                m.item = first_enabled(self, m.menu);
                self.menu = Some(m);
            }
            KeyCode::Right => {
                m.menu = (m.menu + 1) % n_menus;
                m.item = first_enabled(self, m.menu);
                self.menu = Some(m);
            }
            KeyCode::Up => {
                m.item = step(&m, -1);
                self.menu = Some(m);
            }
            KeyCode::Down => {
                m.item = step(&m, 1);
                self.menu = Some(m);
            }
            KeyCode::Home => {
                m.item = first_enabled(self, m.menu);
                self.menu = Some(m);
            }
            KeyCode::Enter => self.menu_activate(m.menu, m.item),
            _ => {}
        }
    }

    /// Typing a letter in an open menu jumps to (and runs) the item starting with it.
    fn menu_char(&mut self, c: char) {
        let Some(m) = self.menu else { return };
        let c = c.to_ascii_lowercase();
        let hits: Vec<usize> = menu::items(m.menu)
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.is_some_and(|cmd| {
                    self.enabled(cmd) && menu::label(cmd).to_lowercase().starts_with(c)
                })
            })
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [] => {}
            [one] => self.menu_activate(m.menu, *one),
            many => {
                let next = many.iter().find(|&&i| i > m.item).unwrap_or(&many[0]);
                self.menu = Some(MenuState {
                    menu: m.menu,
                    item: *next,
                });
            }
        }
    }

    fn keymap_key(&mut self, k: Key, mut v: KeymapView) {
        let n = keymap_rows(&v.query).len();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                if let Some(cmd) = keymap_rows(&v.query).get(v.sel).and_then(|r| r.cmd) {
                    self.run(cmd);
                    return;
                }
            }
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp => v.sel = v.sel.saturating_sub(15),
            KeyCode::PageDown => v.sel = (v.sel + 15).min(n.saturating_sub(1)),
            KeyCode::Home => v.sel = 0,
            KeyCode::End => v.sel = n.saturating_sub(1),
            KeyCode::Backspace => {
                if k.ctrl {
                    v.query.clear();
                } else {
                    v.query.pop();
                }
                v.sel = 0;
            }
            _ => {}
        }
        self.modal = Some(Modal::Keymap(v));
    }

    // ===================================================================== commands

    pub fn run(&mut self, cmd: Command) {
        use Command::*;
        let page = self.page;
        match cmd {
            GotoFile => self.open_palette(Mode::Files),
            ShowCommands => self.open_palette(Mode::Commands),
            GotoSymbol => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                self.focus = Focus::Editor;
                self.open_palette(Mode::Symbols)
            }
            GotoWorkspaceSymbol => self.open_palette(Mode::WorkspaceSymbols),
            GotoLine => {
                if self.buffers.is_empty() {
                    return;
                }
                self.focus = Focus::Editor;
                self.open_palette(Mode::Line)
            }
            SwitchBuffer => {
                if self.buffers.is_empty() {
                    return self.msg("No open files");
                }
                let mut p = Palette::new(Mode::Buffers, "");
                p.title = "Open files".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            SelectTheme => {
                self.theme_before = Some(self.config.editor.theme.clone());
                let mut p = Palette::new(Mode::Themes, "");
                p.title = "arrows preview, Enter applies".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
                let cur = self.config.editor.theme.clone();
                if let Some(Modal::Palette(p)) = &mut self.modal
                    && let Some(i) = p.shown.iter().position(|(i, _)| p.items[*i].label == cur)
                {
                    p.sel = i;
                }
            }
            NewFile => {
                let id = self.next_buf_id;
                self.next_buf_id += 1;
                let b = Buffer::from_text(id, None, "", HlCache::new("Plain Text".into()));
                self.buffers.push(b);
                self.cur = self.buffers.len() - 1;
                self.focus = Focus::Editor;
            }
            OpenFolder => {
                let def = format!("{}/", self.root.display());
                self.prompt(
                    "Open folder or file  (Tab completes)",
                    &def,
                    PromptKind::OpenPath,
                );
            }
            OpenSettings => self.open_settings(),
            ReopenClosed => {
                while let Some(p) = self.closed.pop() {
                    if p.is_file() {
                        self.open_file(&p, true);
                        return;
                    }
                }
                self.msg("No recently closed files");
            }
            Quit => {
                let n = self.buffers.iter().filter(|b| b.modified).count();
                if n > 0 {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!("{n} file(s) have unsaved changes. Quit anyway?"),
                        kind: ConfirmKind::Quit,
                    }));
                } else {
                    self.quit = true;
                }
            }
            NextBuffer | PrevBuffer => {
                let n = self.buffers.len();
                if n > 0 {
                    self.cur = if cmd == NextBuffer {
                        (self.cur + 1) % n
                    } else {
                        (self.cur + n - 1) % n
                    };
                    self.focus = Focus::Editor;
                    self.completion = None;
                    self.hover = None;
                }
            }
            ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                if !self.sidebar_visible {
                    self.focus = Focus::Editor;
                }
            }
            FocusSidebar => {
                if self.focus == Focus::Sidebar && self.sidebar_visible {
                    self.focus = Focus::Editor;
                } else {
                    self.focus_sidebar();
                }
            }
            FocusEditor => self.focus = Focus::Editor,
            RevealInSidebar => {
                self.tree_revealed = None;
                self.focus_sidebar();
            }
            ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.tree.set_root(self.root.clone(), self.show_hidden);
                let s = if self.show_hidden { "shown" } else { "hidden" };
                self.msg(format!("Hidden files {s}"));
            }
            RefreshSidebar => {
                self.refresh_tree();
                self.msg("Sidebar refreshed");
            }
            SidebarUp | SidebarDown | SidebarPageUp | SidebarPageDown | SidebarTop
            | SidebarBottom | SidebarExpand | SidebarCollapse | SidebarOpen
            | SidebarOpenKeepFocus | SidebarMark | SidebarMarkUp | SidebarMarkDown
            | SidebarEscape => {
                if !self.sidebar_visible || self.focus != Focus::Sidebar {
                    self.focus_sidebar();
                }
                self.sidebar_nav(cmd);
            }
            ShowKeymap => {
                self.completion = None;
                self.hover = None;
                self.modal = Some(Modal::Keymap(KeymapView {
                    query: String::new(),
                    sel: 0,
                    scroll: 0,
                }));
            }
            OpenMenu => {
                self.completion = None;
                self.hover = None;
                self.menu = Some(MenuState {
                    menu: 0,
                    item: first_enabled(self, 0),
                });
            }
            ToggleMenuBar => {
                self.menu_bar_visible = !self.menu_bar_visible;
                if !self.menu_bar_visible {
                    self.msg("Menu bar hidden — F10 still opens it");
                }
            }
            SidebarNewFile | SidebarNewFolder | SidebarRename | SidebarDelete | SidebarCopy
            | SidebarCut | SidebarPaste | SidebarDuplicate | SidebarCopyPath => {
                if !self.sidebar_visible {
                    self.focus_sidebar();
                }
                self.run_sidebar(cmd);
            }
            ZoomIn => self.font_size = (self.font_size + 1.0).min(40.0),
            ZoomOut => self.font_size = (self.font_size - 1.0).max(8.0),
            ZoomReset => self.font_size = self.config.editor.font_size,
            JumpBack => {
                if let Some((path, pos)) = self.jumps.pop()
                    && let Some(i) = self.open_file(&path, true)
                {
                    let b = &mut self.buffers[i];
                    b.goto(pos);
                    center(b, page);
                }
            }
            RestartLsp => {
                self.restart_lsp();
                self.msg("Language servers restarted");
            }
            SaveAll => {
                let cur = self.cur;
                for i in 0..self.buffers.len() {
                    if self.buffers[i].modified && self.buffers[i].path.is_some() {
                        self.cur = i;
                        self.save_current();
                    }
                }
                self.cur = cur;
            }
            _ => self.run_editor(cmd),
        }
    }

    fn apply_theme(&mut self, name: &str) {
        self.syn.set_theme(name);
        for b in &mut self.buffers {
            b.hl.invalidate(0);
        }
    }

    fn editor_text(&mut self, c: char) {
        let Some(b) = self.buf_mut() else { return };
        if b.read_only {
            return;
        }
        b.insert_char(c);
        self.hover = None;
        self.after_editor_key(Some(c));
    }

    fn editor_key(&mut self, k: Key) -> bool {
        if self.buffers.is_empty() {
            return false;
        }
        if matches!(k.code, KeyCode::Char(_)) && !k.ctrl && keymap::lookup(k, Ctx::Editor).is_none()
        {
            return false;
        }
        self.editor_key_inner(k);
        true
    }

    fn close_lsp_doc(&mut self, i: usize) {
        let b = &mut self.buffers[i];
        if let (Some(cid), Some(path)) = (b.lsp_client.take(), &b.path)
            && let Some(c) = self.lsp.clients.get_mut(&cid)
        {
            c.notify(
                "textDocument/didClose",
                json!({"textDocument": {"uri": path_to_uri(path)}}),
            );
        }
    }

    /// Editor mouse click / drag: `clicks` 1 = place, 2 = word, 3 = line.
    pub fn editor_click(&mut self, pos: Pos, clicks: u32, extend: bool) {
        self.focus = Focus::Editor;
        self.completion = None;
        self.hover = None;
        let Some(b) = self.buf_mut() else { return };
        let pos = b.clamp(pos);
        match clicks {
            2 => {
                if let Some((s, e)) = b.word_bounds(pos) {
                    b.anchor = Some(s);
                    b.cursor = e;
                } else {
                    b.goto(pos);
                }
            }
            3 => {
                b.goto(Pos::new(pos.line, 0));
                b.select_line();
            }
            _ => {
                b.set_cursor(pos, extend);
                b.want_col = b.cursor.col;
            }
        }
    }

    pub fn editor_drag(&mut self, pos: Pos) {
        if let Some(b) = self.buf_mut() {
            let pos = b.clamp(pos);
            b.set_cursor(pos, true);
            b.want_col = b.cursor.col;
        }
    }

    fn run_editor(&mut self, cmd: Command) {
        use Command::*;
        if self.buffers.is_empty() {
            if self.buffers.is_empty() {
                self.msg("No open file");
            }
            return;
        }
        let page = self.page;
        match cmd {
            Save => return self.save_current(),
            SaveAs => {
                let def = self
                    .buf()
                    .and_then(|b| b.path.as_ref())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                return self.prompt("Save as", &def, PromptKind::SaveAs);
            }
            CloseBuffer => {
                let b = &self.buffers[self.cur];
                if b.modified {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!("{} has unsaved changes. Close anyway?", b.name()),
                        kind: ConfirmKind::CloseBuffer(b.id),
                    }));
                } else {
                    self.close_buffer_at(self.cur);
                }
                return;
            }
            Copy | Cut => {
                let b = &mut self.buffers[self.cur];
                let (text, line) = match b.selected_text() {
                    Some(t) => (t, false),
                    None => (format!("{}\n", b.lines[b.cursor.line]), true),
                };
                if cmd == Cut && !b.read_only {
                    if line {
                        b.delete_line();
                    } else {
                        b.insert_text("");
                    }
                }
                self.pending_copy = Some(text.clone());
                self.clip = text;
                self.clip_line = line;
                return;
            }
            Paste => return self.paste_text(self.clip.clone()),
            Find => {
                let b = &self.buffers[self.cur];
                let def = b
                    .selected_text()
                    .filter(|t| !t.contains('\n'))
                    .or_else(|| self.search.clone())
                    .unwrap_or_default();
                let origin = b.selection().map(|s| s.0).unwrap_or(b.cursor);
                return self.prompt("Find", &def, PromptKind::Find { origin });
            }
            Replace => {
                let def = self.buffers[self.cur]
                    .selected_text()
                    .filter(|t| !t.contains('\n'))
                    .or_else(|| self.search.clone())
                    .unwrap_or_default();
                return self.prompt("Replace all — find", &def, PromptKind::ReplaceFind);
            }
            FindNext => return self.find_next(true),
            FindPrev => return self.find_next(false),
            Completion => return self.request_completion(),
            Hover => {
                let pos = self.buffers[self.cur].cursor;
                let id = self.buffers[self.cur].id;
                self.lsp_position_request(
                    "textDocument/hover",
                    json!({}),
                    Pending::Hover { buf: id, pos },
                    "hoverProvider",
                );
                return;
            }
            GotoDefinition => {
                self.lsp_position_request(
                    "textDocument/definition",
                    json!({}),
                    Pending::Definition,
                    "definitionProvider",
                );
                return;
            }
            FindReferences => {
                self.lsp_position_request(
                    "textDocument/references",
                    json!({"context": {"includeDeclaration": true}}),
                    Pending::References,
                    "referencesProvider",
                );
                return;
            }
            Rename => {
                let b = &self.buffers[self.cur];
                let w = b
                    .word_bounds(b.cursor)
                    .map(|(a, e)| b.text_range(a, e))
                    .unwrap_or_default();
                return self.prompt("Rename symbol to", &w, PromptKind::RenameSymbol);
            }
            Format => {
                self.sync_lsp();
                let b = &self.buffers[self.cur];
                let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
                    return self.msg("No language server for this file");
                };
                let id = b.id;
                let tab = b.indent_unit == "\t";
                let size = if tab { 4 } else { b.indent_unit.len() };
                if let Some(c) = self.lsp.clients.get_mut(&cid) {
                    c.request(
                        "textDocument/formatting",
                        json!({"textDocument": {"uri": path_to_uri(&path)},
                               "options": {"tabSize": size, "insertSpaces": !tab}}),
                        Pending::Format { buf: id },
                    );
                }
                return;
            }
            NextDiagnostic | PrevDiagnostic => return self.jump_diagnostic(cmd == NextDiagnostic),
            _ => {}
        }
        let b = &mut self.buffers[self.cur];
        if b.read_only {
            self.msg("Read-only buffer");
            return;
        }
        match cmd {
            Undo => {
                if !b.undo() {
                    self.msg("Nothing to undo");
                }
            }
            Redo => {
                if !b.redo() {
                    self.msg("Nothing to redo");
                }
            }
            SelectAll => b.select_all(),
            SelectLine => b.select_line(),
            SelectWord => {
                b.select_word_or_next();
            }
            DeleteLine => b.delete_line(),
            DuplicateLine => b.duplicate(),
            MoveLineUp => b.move_lines(true),
            MoveLineDown => b.move_lines(false),
            ToggleComment => {
                if !b.toggle_comment() {
                    self.msg("No comment syntax known for this file type");
                }
            }
            Indent => b.indent(),
            Outdent => b.outdent(),
            InsertLineAfter => b.insert_line_after(false),
            InsertLineBefore => b.insert_line_after(true),
            JoinLines => b.join_lines(),
            UpperCase => b.transform_selection(|s| s.to_uppercase()),
            LowerCase => b.transform_selection(|s| s.to_lowercase()),
            SortLines => b.sort_lines(),
            _ => {}
        }
        let _ = page;
    }

    fn find_next(&mut self, forward: bool) {
        let Some(q) = self.search.clone().filter(|s| !s.is_empty()) else {
            return self.msg("Nothing to find (Ctrl+F)");
        };
        let pg = self.page;
        let Some(b) = self.buf_mut() else { return };
        let from = match (b.selection(), forward) {
            (Some((_, e)), true) => e,
            (Some((s, _)), false) => s,
            (None, _) => b.cursor,
        };
        match b.find(&q, from, forward, false) {
            Some((s, e)) => {
                b.anchor = Some(s);
                b.cursor = e;
                if b.cursor.line < b.scroll_y || b.cursor.line >= b.scroll_y + pg {
                    center(b, pg);
                }
            }
            None => self.msg(format!("'{q}' not found")),
        }
    }

    fn jump_diagnostic(&mut self, forward: bool) {
        let Some(b) = self.buf() else { return };
        let Some(path) = &b.path else { return };
        let Some(ds) = self.diagnostics.get(path).filter(|d| !d.is_empty()) else {
            return self.msg("No diagnostics");
        };
        let c = b.cursor;
        let mut starts: Vec<(Pos, String)> =
            ds.iter().map(|d| (d.start, d.message.clone())).collect();
        starts.sort_by_key(|s| s.0);
        let target = if forward {
            starts.iter().find(|s| s.0 > c).or(starts.first())
        } else {
            starts.iter().rev().find(|s| s.0 < c).or(starts.last())
        }
        .cloned();
        if let Some((p, m)) = target {
            let pg = self.page;
            if let Some(b) = self.buf_mut() {
                b.goto(p);
                center(b, pg);
            }
            self.msg(m.lines().next().unwrap_or("").to_string());
        }
    }

    // ===================================================================== completion popup

    fn comp_move(&mut self, d: isize) {
        if let Some(c) = &mut self.completion {
            if c.shown.is_empty() {
                return;
            }
            let n = c.shown.len() as isize;
            c.sel = if d.abs() == 1 {
                (c.sel as isize + d).rem_euclid(n) as usize
            } else {
                (c.sel as isize + d).clamp(0, n - 1) as usize
            };
        }
    }

    fn comp_refilter(&mut self) {
        let Some(b) = self.buffers.get(self.cur) else {
            return;
        };
        let Some(c) = &mut self.completion else {
            return;
        };
        let prefix = b.text_range(c.start, b.cursor).to_lowercase();
        let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
        use fuzzy_matcher::FuzzyMatcher;
        let mut scored: Vec<(i64, usize)> = c
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                if prefix.is_empty() {
                    return Some((0, i));
                }
                matcher.fuzzy_match(&it.filter, &prefix).map(|s| {
                    let bonus = if it.filter.to_lowercase().starts_with(&prefix) {
                        1000
                    } else {
                        0
                    };
                    (s + bonus, i)
                })
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| c.items[a.1].sort.cmp(&c.items[b.1].sort))
        });
        c.shown = scored.into_iter().map(|s| s.1).collect();
        c.sel = 0;
        if c.shown.is_empty() {
            self.completion = None;
        }
    }

    fn comp_accept(&mut self) {
        let Some(c) = self.completion.take() else {
            return;
        };
        let Some(item) = c.shown.get(c.sel).and_then(|&i| c.items.get(i)).cloned() else {
            return;
        };
        let Some(b) = self.buf_mut() else { return };
        let start = item
            .edit_start
            .filter(|s| s.line == b.cursor.line && s.col <= b.cursor.col)
            .unwrap_or(c.start);
        let end = b.cursor;
        // Also swallow the rest of the identifier to the right of the cursor.
        let end = b
            .word_bounds(end)
            .map(|(_, e)| e)
            .filter(|e| *e > end)
            .unwrap_or(end);
        let mut edits = vec![(start, end, item.insert.clone())];
        edits.extend(item.additional.iter().cloned());
        let before_lines = b.lines.len();
        b.apply_edits(edits);
        // Place cursor after the inserted text (adjusting for lines added above by imports).
        let added = b.lines.len() as isize
            - before_lines as isize
            - item.insert.matches('\n').count() as isize;
        let line = (start.line as isize + added.max(0)) as usize;
        let end = crate::editor::end_of(Pos::new(line, start.col), &item.insert);
        b.goto(end);
    }

    // ===================================================================== LSP plumbing

    fn lsp_open(&mut self, i: usize) {
        let b = &self.buffers[i];
        let (Some(path), Some(lang)) = (b.path.clone(), b.lang.clone()) else {
            return;
        };
        let Some(cfg) = self.config.language(&lang).cloned() else {
            return;
        };
        if cfg.server.is_none() {
            return;
        }
        match self.lsp.client_for(&cfg, &path) {
            Ok(cid) => {
                let b = &mut self.buffers[i];
                b.lsp_client = Some(cid);
                b.lsp_dirty = false;
                b.lsp_changes.clear();
                b.lsp_full = false;
                let text = b.text();
                let version = b.version;
                if let Some(c) = self.lsp.clients.get_mut(&cid) {
                    c.notify(
                        "textDocument/didOpen",
                        json!({"textDocument": {"uri": path_to_uri(&path), "languageId": cfg.language_id(),
                               "version": version, "text": text}}),
                    );
                }
            }
            Err(e) => {
                if self.lsp_warned.insert(lang) {
                    self.msg(format!("LSP: {e}"));
                }
            }
        }
    }

    /// Push full-document changes for every buffer edited since the last sync.
    pub fn sync_lsp(&mut self) {
        for b in &mut self.buffers {
            if !b.lsp_dirty {
                continue;
            }
            b.lsp_dirty = false;
            let changes = std::mem::take(&mut b.lsp_changes);
            let full = std::mem::replace(&mut b.lsp_full, false);
            let (Some(cid), Some(path)) = (b.lsp_client, &b.path) else {
                continue;
            };
            let Some(c) = self.lsp.clients.get_mut(&cid) else {
                continue;
            };
            // Incremental sync sends only the edited ranges instead of the whole file.
            let content: Vec<Value> = if c.ready && c.sync_kind == 2 && !full {
                changes
                    .into_iter()
                    .map(|(a, z, text)| {
                        json!({"range": {"start": {"line": a.line, "character": a.col},
                                         "end": {"line": z.line, "character": z.col}},
                               "text": text})
                    })
                    .collect()
            } else {
                vec![json!({"text": b.text()})]
            };
            c.notify(
                "textDocument/didChange",
                json!({"textDocument": {"uri": path_to_uri(path), "version": b.version},
                       "contentChanges": content}),
            );
        }
    }

    fn lsp_position_request(&mut self, method: &str, extra: Value, p: Pending, cap: &str) -> bool {
        self.sync_lsp();
        let Some(b) = self.buf() else { return false };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            self.msg("No language server for this file");
            return false;
        };
        let pos = to_lsp(&b.lines[b.cursor.line], b.cursor);
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return false;
        };
        if c.ready && !c.has_cap(cap) {
            let name = c.cmd.clone();
            self.msg(format!("{name} does not support {method}"));
            return false;
        }
        let mut params = json!({"textDocument": {"uri": path_to_uri(&path)}, "position": pos});
        if let (Some(o), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        c.request(method, params, p);
        true
    }

    fn request_completion(&mut self) {
        let Some(b) = self.buf() else { return };
        let (id, pos) = (b.id, b.cursor);
        self.lsp_position_request(
            "textDocument/completion",
            json!({"context": {"triggerKind": 1}}),
            Pending::Completion { buf: id, pos },
            "completionProvider",
        );
    }

    fn request_doc_symbols(&mut self) -> bool {
        self.sync_lsp();
        let Some(b) = self.buf() else { return false };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            return false;
        };
        let id = b.id;
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return false;
        };
        if c.ready && !c.has_cap("documentSymbolProvider") {
            return false;
        }
        c.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": path_to_uri(&path)}}),
            Pending::DocSymbols { buf: id },
        );
        true
    }

    fn request_workspace_symbols(&mut self) {
        let cid = self.buf().and_then(|b| b.lsp_client).or_else(|| {
            self.lsp
                .clients
                .iter()
                .find(|(_, c)| c.alive)
                .map(|(k, _)| *k)
        });
        let Some(query) = self.palette().map(|p| p.needle().to_string()) else {
            return;
        };
        let Some(cid) = cid else {
            if let Some(p) = self.palette() {
                p.title = "Project symbols — no language server running".into();
            }
            return;
        };
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return;
        };
        let id = c.request(
            "workspace/symbol",
            json!({"query": query}),
            Pending::WorkspaceSymbols,
        );
        if let Some(p) = self.palette() {
            p.ws_request = Some(id);
            p.loading = true;
        }
    }

    fn on_lsp(&mut self, cid: usize, msg: Value) {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).map(String::from);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (id, method) {
            (Some(id), Some(m)) => self.on_server_request(cid, id, &m, params),
            (None, Some(m)) => self.on_notification(cid, &m, params),
            (Some(id), None) => {
                let Some(c) = self.lsp.clients.get_mut(&cid) else {
                    return;
                };
                let Some(n) = id.as_i64() else { return };
                let Some(pending) = c.pending.remove(&n) else {
                    return;
                };
                if let Some(err) = msg.get("error") {
                    let m = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("error");
                    if !matches!(
                        pending,
                        Pending::Completion { .. } | Pending::WorkspaceSymbols
                    ) {
                        self.msg(format!("LSP: {m}"));
                    }
                    return;
                }
                let result = msg.get("result").cloned().unwrap_or(Value::Null);
                self.on_response(cid, n, pending, result);
            }
            _ => {}
        }
    }

    fn on_server_request(&mut self, cid: usize, id: Value, method: &str, params: Value) {
        let result = match method {
            "workspace/configuration" => {
                let items = params
                    .get("items")
                    .and_then(|i| i.as_array())
                    .cloned()
                    .unwrap_or_default();
                let c = self.lsp.clients.get(&cid);
                Value::Array(
                    items
                        .iter()
                        .map(|it| {
                            let section = it.get("section").and_then(|s| s.as_str());
                            c.map(|c| c.setting(section)).unwrap_or(Value::Null)
                        })
                        .collect(),
                )
            }
            "workspace/applyEdit" => {
                if let Some(edit) = params.get("edit").cloned()
                    && let Ok(e) = serde_json::from_value::<lt::WorkspaceEdit>(edit)
                {
                    self.apply_workspace_edit(e);
                }
                json!({"applied": true})
            }
            "workspace/workspaceFolders" => {
                let root = self
                    .lsp
                    .clients
                    .get(&cid)
                    .map(|c| c.root.clone())
                    .unwrap_or_default();
                json!([{"uri": path_to_uri(&root), "name": root.file_name().unwrap_or_default().to_string_lossy()}])
            }
            _ => Value::Null,
        };
        if let Some(c) = self.lsp.clients.get_mut(&cid) {
            c.respond(id, result);
        }
    }

    fn on_notification(&mut self, cid: usize, method: &str, params: Value) {
        match method {
            "textDocument/publishDiagnostics" => {
                let Ok(p) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params) else {
                    return;
                };
                let Some(path) = uri_to_path(p.uri.as_str()) else {
                    return;
                };
                let buf = self.buffers.iter().find(|b| b.path.as_ref() == Some(&path));
                let conv = |l: u32, c: u32| match buf {
                    Some(b) => from_lsp(&b.lines, l, c),
                    None => Pos::new(l as usize, c as usize),
                };
                let ds = p
                    .diagnostics
                    .into_iter()
                    .map(|d| Diag {
                        start: conv(d.range.start.line, d.range.start.character),
                        end: conv(d.range.end.line, d.range.end.character),
                        severity: d.severity.map(|s| kind_num(&s) as u8).unwrap_or(1),
                        message: match d.source {
                            Some(s) => format!("{s}: {}", d.message),
                            None => d.message,
                        },
                    })
                    .collect();
                self.diagnostics.insert(path, ds);
            }
            "window/showMessage" => {
                if let Some(m) = params.get("message").and_then(|m| m.as_str()) {
                    let t = params.get("type").and_then(|t| t.as_i64()).unwrap_or(3);
                    if t <= 2 {
                        self.msg(format!("LSP: {m}"));
                    }
                }
            }
            "$/progress" => {
                let Some(c) = self.lsp.clients.get_mut(&cid) else {
                    return;
                };
                let v = params.get("value").cloned().unwrap_or(Value::Null);
                match v.get("kind").and_then(|k| k.as_str()) {
                    Some("begin") | Some("report") => {
                        let title = v.get("title").and_then(|t| t.as_str()).map(String::from);
                        let m = v.get("message").and_then(|t| t.as_str()).unwrap_or("");
                        let prev = c.progress.clone().unwrap_or_default();
                        let t = title
                            .unwrap_or_else(|| prev.split(':').next().unwrap_or("").to_string());
                        c.progress = Some(if m.is_empty() { t } else { format!("{t}: {m}") });
                    }
                    Some("end") => c.progress = None,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn on_response(&mut self, cid: usize, req_id: i64, pending: Pending, result: Value) {
        match pending {
            Pending::Initialize => {
                let caps = result.get("capabilities").cloned().unwrap_or(Value::Null);
                if let Some(c) = self.lsp.clients.get_mut(&cid) {
                    c.on_initialized(caps);
                }
            }
            Pending::Completion { buf, pos } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                if bi != self.cur || self.focus != Focus::Editor || self.modal.is_some() {
                    return;
                }
                let b = &self.buffers[bi];
                if b.cursor.line != pos.line {
                    return;
                }
                let Ok(resp) = serde_json::from_value::<lt::CompletionResponse>(result) else {
                    return;
                };
                let list = match resp {
                    lt::CompletionResponse::Array(a) => a,
                    lt::CompletionResponse::List(l) => l.items,
                };
                let items: Vec<CompItem> = list
                    .into_iter()
                    .take(2000)
                    .map(|it| {
                        let snippet = it.insert_text_format == Some(lt::InsertTextFormat::SNIPPET);
                        let (text, start) = match &it.text_edit {
                            Some(lt::CompletionTextEdit::Edit(e)) => (
                                e.new_text.clone(),
                                Some(from_lsp(
                                    &b.lines,
                                    e.range.start.line,
                                    e.range.start.character,
                                )),
                            ),
                            Some(lt::CompletionTextEdit::InsertAndReplace(e)) => (
                                e.new_text.clone(),
                                Some(from_lsp(
                                    &b.lines,
                                    e.insert.start.line,
                                    e.insert.start.character,
                                )),
                            ),
                            None => (
                                it.insert_text.clone().unwrap_or_else(|| it.label.clone()),
                                None,
                            ),
                        };
                        let insert = if snippet { strip_snippet(&text) } else { text };
                        let additional = it
                            .additional_text_edits
                            .unwrap_or_default()
                            .into_iter()
                            .map(|e| {
                                (
                                    from_lsp(&b.lines, e.range.start.line, e.range.start.character),
                                    from_lsp(&b.lines, e.range.end.line, e.range.end.character),
                                    e.new_text,
                                )
                            })
                            .collect();
                        CompItem {
                            filter: it.filter_text.clone().unwrap_or_else(|| it.label.clone()),
                            sort: it.sort_text.clone().unwrap_or_else(|| it.label.clone()),
                            kind: it.kind.map(|k| completion_kind(kind_num(&k))).unwrap_or(""),
                            detail: it.detail.unwrap_or_default(),
                            label: it.label,
                            insert,
                            edit_start: start,
                            additional,
                        }
                    })
                    .collect();
                if items.is_empty() {
                    return;
                }
                let start = b.word_start(b.cursor);
                self.completion = Some(Completion {
                    buf_id: buf,
                    start,
                    items,
                    shown: vec![],
                    sel: 0,
                });
                self.comp_refilter();
            }
            Pending::Hover { buf, pos } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                if bi != self.cur || self.buffers[bi].cursor != pos {
                    return;
                }
                let Ok(Some(h)) = serde_json::from_value::<Option<lt::Hover>>(result) else {
                    return self.msg("No hover info");
                };
                let ms = |m: lt::MarkedString| match m {
                    lt::MarkedString::String(s) => s,
                    lt::MarkedString::LanguageString(l) => l.value,
                };
                let text = match h.contents {
                    lt::HoverContents::Scalar(m) => ms(m),
                    lt::HoverContents::Array(a) => {
                        a.into_iter().map(ms).collect::<Vec<_>>().join("\n---\n")
                    }
                    lt::HoverContents::Markup(m) => m.value,
                };
                let lines = markdown_to_lines(&text);
                if lines.is_empty() {
                    return self.msg("No hover info");
                }
                self.hover = Some(Hover { lines });
            }
            Pending::Definition | Pending::References => {
                let locs = parse_locations(result);
                if locs.is_empty() {
                    return self.msg("No results");
                }
                let is_def = matches!(pending, Pending::Definition);
                if is_def && locs.len() == 1 {
                    let (path, l, c) = locs[0].clone();
                    let pos = self.lsp_pos_for(&path, l, c);
                    self.goto_location(&path, Some(pos));
                    return;
                }
                let items: Vec<Item> = locs
                    .into_iter()
                    .map(|(path, l, c)| {
                        let pos = self.lsp_pos_for(&path, l, c);
                        let preview = self.line_text(&path, pos.line);
                        let rel = self.rel_path(&path);
                        Item {
                            label: format!("{rel}:{}", pos.line + 1),
                            detail: preview.trim().chars().take(80).collect(),
                            action: Action::Open(path, Some(pos)),
                        }
                    })
                    .collect();
                let mut p = Palette::new(Mode::Locations, "");
                p.title = format!(
                    "{} {}",
                    items.len(),
                    if is_def { "definitions" } else { "references" }
                );
                p.set_items(items);
                self.modal = Some(Modal::Palette(p));
            }
            Pending::DocSymbols { buf } => {
                let Some(Modal::Palette(p)) = &self.modal else {
                    return;
                };
                if p.mode != Mode::Symbols {
                    return;
                }
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                let Ok(Some(resp)) =
                    serde_json::from_value::<Option<lt::DocumentSymbolResponse>>(result)
                else {
                    if let Some(p) = self.palette() {
                        p.loading = false;
                    }
                    return;
                };
                let lines = &self.buffers[bi].lines;
                let mut items = vec![];
                match resp {
                    lt::DocumentSymbolResponse::Flat(v) => {
                        for s in v {
                            let pos = from_lsp(
                                lines,
                                s.location.range.start.line,
                                s.location.range.start.character,
                            );
                            items.push(Item {
                                label: s.name,
                                detail: format!(
                                    "{} {}",
                                    kind_name(kind_num(&s.kind)),
                                    s.container_name.unwrap_or_default()
                                ),
                                action: Action::Goto(pos),
                            });
                        }
                    }
                    lt::DocumentSymbolResponse::Nested(v) => {
                        fn walk(
                            v: Vec<lt::DocumentSymbol>,
                            depth: usize,
                            lines: &[String],
                            out: &mut Vec<Item>,
                        ) {
                            for s in v {
                                let r = s.selection_range.start;
                                out.push(Item {
                                    label: format!("{}{}", "  ".repeat(depth), s.name),
                                    detail: format!(
                                        "{} {}",
                                        kind_name(kind_num(&s.kind)),
                                        s.detail.unwrap_or_default()
                                    )
                                    .chars()
                                    .take(60)
                                    .collect(),
                                    action: Action::Goto(from_lsp(lines, r.line, r.character)),
                                });
                                if let Some(ch) = s.children {
                                    walk(ch, depth + 1, lines, out);
                                }
                            }
                        }
                        walk(v, 0, lines, &mut items);
                    }
                }
                items.sort_by_key(|i| match i.action {
                    Action::Goto(p) => p,
                    _ => Pos::default(),
                });
                if let Some(p) = self.palette() {
                    p.loading = false;
                    p.set_items(items);
                }
                self.palette_preview();
            }
            Pending::WorkspaceSymbols => {
                let Some(p) = self.palette() else { return };
                if p.mode != Mode::WorkspaceSymbols || p.ws_request != Some(req_id) {
                    return;
                }
                let Ok(Some(resp)) =
                    serde_json::from_value::<Option<lt::WorkspaceSymbolResponse>>(result)
                else {
                    p.loading = false;
                    return;
                };
                let mut raw: Vec<(String, String, PathBuf, u32, u32)> = vec![];
                match resp {
                    lt::WorkspaceSymbolResponse::Flat(v) => {
                        for s in v {
                            if let Some(path) = uri_to_path(s.location.uri.as_str()) {
                                let r = s.location.range.start;
                                raw.push((
                                    s.name,
                                    kind_name(kind_num(&s.kind)).into(),
                                    path,
                                    r.line,
                                    r.character,
                                ));
                            }
                        }
                    }
                    lt::WorkspaceSymbolResponse::Nested(v) => {
                        for s in v {
                            let (uri, r) = match s.location {
                                lt::OneOf::Left(l) => (l.uri, l.range.start),
                                lt::OneOf::Right(w) => (w.uri, lt::Position::default()),
                            };
                            if let Some(path) = uri_to_path(uri.as_str()) {
                                raw.push((
                                    s.name,
                                    kind_name(kind_num(&s.kind)).into(),
                                    path,
                                    r.line,
                                    r.character,
                                ));
                            }
                        }
                    }
                }
                let items: Vec<Item> = raw
                    .into_iter()
                    .map(|(name, kind, path, l, c)| {
                        let pos = self.lsp_pos_for(&path, l, c);
                        let rel = self.rel_path(&path);
                        Item {
                            label: name,
                            detail: format!("{kind}  {rel}:{}", pos.line + 1),
                            action: Action::Open(path, Some(pos)),
                        }
                    })
                    .collect();
                if let Some(p) = self.palette() {
                    p.loading = false;
                    p.set_items(items);
                }
            }
            Pending::Rename => match serde_json::from_value::<Option<lt::WorkspaceEdit>>(result) {
                Ok(Some(e)) => {
                    let (n, f) = self.apply_workspace_edit(e);
                    self.msg(format!(
                        "Renamed: {n} edit(s) in {f} file(s) — save to keep"
                    ));
                }
                _ => self.msg("Rename failed"),
            },
            Pending::Format { buf } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                let Ok(Some(edits)) = serde_json::from_value::<Option<Vec<lt::TextEdit>>>(result)
                else {
                    return;
                };
                let b = &mut self.buffers[bi];
                let es = edits
                    .into_iter()
                    .map(|e| {
                        (
                            from_lsp(&b.lines, e.range.start.line, e.range.start.character),
                            from_lsp(&b.lines, e.range.end.line, e.range.end.character),
                            e.new_text,
                        )
                    })
                    .collect::<Vec<_>>();
                let n = es.len();
                b.apply_edits(es);
                self.msg(if n == 0 {
                    "Already formatted".to_string()
                } else {
                    format!("Formatted ({n} edits)")
                });
            }
            Pending::Other => {}
        }
    }

    fn lsp_pos_for(&self, path: &Path, l: u32, c: u32) -> Pos {
        if let Some(b) = self
            .buffers
            .iter()
            .find(|b| b.path.as_deref() == Some(path))
        {
            return from_lsp(&b.lines, l, c);
        }
        Pos::new(l as usize, c as usize)
    }

    fn line_text(&self, path: &Path, line: usize) -> String {
        if let Some(b) = self
            .buffers
            .iter()
            .find(|b| b.path.as_deref() == Some(path))
        {
            return b.lines.get(line).cloned().unwrap_or_default();
        }
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| s.lines().nth(line).map(String::from))
            .unwrap_or_default()
    }

    fn rel_path(&self, path: &Path) -> String {
        let root = self.file_root();
        path.strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn apply_workspace_edit(&mut self, edit: lt::WorkspaceEdit) -> (usize, usize) {
        let mut per_file: Vec<(PathBuf, Vec<lt::TextEdit>)> = vec![];
        if let Some(changes) = edit.changes {
            for (uri, edits) in changes {
                if let Some(p) = uri_to_path(uri.as_str()) {
                    per_file.push((p, edits));
                }
            }
        }
        let push_tde = |tde: lt::TextDocumentEdit, out: &mut Vec<(PathBuf, Vec<lt::TextEdit>)>| {
            if let Some(p) = uri_to_path(tde.text_document.uri.as_str()) {
                let edits = tde
                    .edits
                    .into_iter()
                    .map(|e| match e {
                        lt::OneOf::Left(t) => t,
                        lt::OneOf::Right(a) => a.text_edit,
                    })
                    .collect();
                out.push((p, edits));
            }
        };
        match edit.document_changes {
            Some(lt::DocumentChanges::Edits(v)) => {
                for t in v {
                    push_tde(t, &mut per_file);
                }
            }
            Some(lt::DocumentChanges::Operations(ops)) => {
                for op in ops {
                    if let lt::DocumentChangeOperation::Edit(t) = op {
                        push_tde(t, &mut per_file);
                    }
                }
            }
            None => {}
        }
        let (mut n, files) = (0, per_file.len());
        let (cur, focus) = (self.cur, self.focus);
        for (path, edits) in per_file {
            let Some(i) = self.open_file(&path, false) else {
                continue;
            };
            let b = &mut self.buffers[i];
            let es: Vec<_> = edits
                .into_iter()
                .map(|e| {
                    (
                        from_lsp(&b.lines, e.range.start.line, e.range.start.character),
                        from_lsp(&b.lines, e.range.end.line, e.range.end.character),
                        e.new_text,
                    )
                })
                .collect();
            n += es.len();
            b.apply_edits(es);
        }
        self.cur = cur.min(self.buffers.len().saturating_sub(1));
        self.focus = focus;
        (n, files)
    }

    pub fn lsp_status(&self) -> String {
        let Some(b) = self.buf() else {
            return String::new();
        };
        match b.lsp_client.and_then(|c| self.lsp.clients.get(&c)) {
            Some(c) if !c.alive => format!("{} (dead)", c.cmd),
            Some(c) if !c.ready => format!("{} starting…", c.cmd),
            Some(c) => match &c.progress {
                Some(p) => format!("{} · {}", c.cmd, p.chars().take(40).collect::<String>()),
                None => c.cmd.clone(),
            },
            None => String::new(),
        }
    }
}

fn parse_locations(v: Value) -> Vec<(PathBuf, u32, u32)> {
    let mut out = vec![];
    let arr = match v {
        Value::Array(a) => a,
        Value::Null => vec![],
        o => vec![o],
    };
    for l in arr {
        if let Ok(loc) = serde_json::from_value::<lt::Location>(l.clone()) {
            if let Some(p) = uri_to_path(loc.uri.as_str()) {
                out.push((p, loc.range.start.line, loc.range.start.character));
            }
        } else if let Ok(link) = serde_json::from_value::<lt::LocationLink>(l)
            && let Some(p) = uri_to_path(link.target_uri.as_str())
        {
            let r = link.target_selection_range.start;
            out.push((p, r.line, r.character));
        }
    }
    out
}

pub fn center(b: &mut Buffer, page: usize) {
    b.scroll_y = b.cursor.line.saturating_sub(page / 3);
}

/// Fallback "Goto Symbol" when no language server is available: keyword-based scan.
fn heuristic_symbols(b: &Buffer) -> Vec<Item> {
    const PREFIXES: &[&str] = &[
        "pub(crate) ",
        "pub(super) ",
        "pub ",
        "export default ",
        "export ",
        "async ",
        "static ",
        "unsafe ",
        "public ",
        "private ",
        "protected ",
        "abstract ",
        "final ",
        "inline ",
        "extern \"C\" ",
    ];
    const KEYWORDS: &[&str] = &[
        "fn ",
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "impl<",
        "mod ",
        "macro_rules! ",
        "type ",
        "def ",
        "class ",
        "function ",
        "func ",
        "interface ",
        "module ",
        "union ",
        "const fn ",
    ];
    let md = b
        .path
        .as_ref()
        .and_then(|p| p.extension())
        .is_some_and(|e| e == "md" || e == "markdown");
    let mut out = vec![];
    for (i, line) in b.lines.iter().enumerate() {
        let t = line.trim_start();
        if md {
            if t.starts_with('#') {
                out.push(Item {
                    label: t.to_string(),
                    detail: String::new(),
                    action: Action::Goto(Pos::new(i, 0)),
                });
            }
            continue;
        }
        let mut s = t;
        loop {
            match PREFIXES.iter().find(|p| s.starts_with(*p)) {
                Some(p) => s = &s[p.len()..],
                None => break,
            }
        }
        if KEYWORDS.iter().any(|k| s.starts_with(k)) {
            let label: String = t
                .trim_end_matches(['{', ' ', ':'])
                .chars()
                .take(90)
                .collect();
            let indent = line.len() - t.len();
            out.push(Item {
                label: format!("{}{}", " ".repeat(indent.min(8)), label),
                detail: format!("{}", i + 1),
                action: Action::Goto(Pos::new(i, indent)),
            });
        }
    }
    out
}

fn first_enabled(app: &App, menu: usize) -> usize {
    menu::items(menu)
        .iter()
        .position(|c| c.is_some_and(|c| app.enabled(c)))
        .unwrap_or(0)
}

/// Shell-style Tab completion for path prompts.
fn complete_path(base: &Path, input: &str) -> String {
    let (dir_part, prefix) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    let dir = if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else if let Some(rest) = dir_part.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(rest)
    } else {
        base.join(dir_part)
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return input.to_string();
    };
    let mut names: Vec<(String, bool)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.')))
                .then(|| (n, e.path().is_dir()))
        })
        .collect();
    names.sort();
    match names.as_slice() {
        [] => input.to_string(),
        [(n, d)] => format!("{dir_part}{n}{}", if *d { "/" } else { "" }),
        many => {
            let first = &many[0].0;
            let mut common = first.len();
            for (n, _) in &many[1..] {
                common = common.min(
                    first
                        .bytes()
                        .zip(n.bytes())
                        .take_while(|(a, b)| a == b)
                        .count(),
                );
            }
            while !first.is_char_boundary(common) {
                common -= 1;
            }
            format!("{dir_part}{}", &first[..common])
        }
    }
}
