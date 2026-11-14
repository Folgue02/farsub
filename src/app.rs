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
use crate::installer::{self, InstallEvent};
use crate::keymap::{self, Ctx, Keymap};
use crate::keys::{Key, KeyCode};
use crate::lsp::{Manager, Pending, from_lsp, path_to_uri, to_lsp, uri_to_path};
use crate::menu::{self, Menu, MenuState};
use crate::packages;
use crate::palette::{Action, IndentAction, Item, Mode, Palette};
use crate::project::{self, Last, Project, Recent};
use crate::tree::Tree;
use crate::workspace::{BufferState, Workspace};

pub enum AppEvent {
    Lsp(usize, Value),
    LspExit(usize),
    Files(PathBuf, Vec<String>),
    Install(String, InstallEvent),
    Packages(Box<packages::Loaded>),
    PackageInstalled(Result<String, String>),
    /// A native menu item was chosen (command id).
    Menu(String),
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

    /// Repaint without an event (e.g. terminal output arrived).
    pub fn wake(&self) {
        if let Some(w) = self.wake.get() {
            w();
        }
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
    Console,
    Terminal,
}

/// What the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelTab {
    Terminal(usize),
    Console,
}

/// The bottom panel (terminals + console), resizable from its top edge.
pub struct Panel {
    pub visible: bool,
    pub tab: PanelTab,
    pub height: f32,
}

/// A console selection end: (log entry id, char offset in the rendered line).
pub type ConsolePos = (u64, usize);

/// The console panel (View › Show Console, Ctrl+`): the app log plus a command line.
pub struct Console {
    pub input: String,
    pub cursor: usize,
    history: Vec<String>,
    hist_pos: Option<usize>,
    /// Lines scrolled up from the bottom (0 = follow new output).
    pub scroll: usize,
    pub show_debug: bool,
    pub filter: String,
    /// Wrap long lines (otherwise the panel scrolls sideways).
    pub wrap: bool,
    pub hscroll: f32,
    /// (anchor, head) of the text selection.
    pub sel: Option<(ConsolePos, ConsolePos)>,
}

impl Default for Console {
    fn default() -> Self {
        Console {
            input: String::new(),
            cursor: 0,
            history: vec![],
            hist_pos: None,
            scroll: 0,
            show_debug: false,
            filter: String::new(),
            wrap: true,
            hscroll: 0.0,
            sel: None,
        }
    }
}

pub enum PromptKind {
    NewFile(PathBuf),
    NewFolder(PathBuf),
    /// (source, folder the typed path is relative to)
    RenameMove(PathBuf, PathBuf),
    SaveAs,
    OpenPath,
    Find {
        origin: Pos,
    },
    ReplaceFind,
    ReplaceWith(String),
    RenameSymbol,
    PackageUrl,
    OpenProject,
    SaveProjectAs,
    AddFolder,
}

impl PromptKind {
    fn is_path(&self) -> bool {
        matches!(
            self,
            PromptKind::NewFile(_)
                | PromptKind::NewFolder(_)
                | PromptKind::RenameMove(..)
                | PromptKind::SaveAs
                | PromptKind::OpenPath
                | PromptKind::OpenProject
                | PromptKind::SaveProjectAs
                | PromptKind::AddFolder
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
    UninstallServer(String),
    RemovePackage(String),
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
    Servers(ServersView),
    LspInfo(LspInfoView),
}

/// LSP Info (like Neovim's :LspInfo).
pub struct LspInfoView {
    pub scroll: usize,
}

/// How an LSP Info line is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Heading,
    Text,
    Dim,
    Good,
    Warn,
    Bad,
}

/// The language-server manager (like :Mason).
pub struct ServersView {
    pub sel: usize,
    pub details: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServerStatus {
    Installed(PathBuf),
    OnPath(PathBuf),
    Installing,
    Failed(String),
    Missing,
}

/// The keyboard-shortcut reference (Help → Keyboard Shortcuts).
pub struct KeymapView {
    pub query: String,
    pub sel: usize,
    pub scroll: usize,
}

#[derive(Clone, Debug)]
pub struct KeymapRow {
    pub section: String,
    pub keys: String,
    pub desc: String,
    pub cmd: Option<Command>,
}

/// All shortcuts grouped by where they apply, filtered by a search string.
pub fn keymap_rows(km: &Keymap, query: &str) -> Vec<KeymapRow> {
    let mut rows = vec![];
    for (ctx, section) in [
        (Ctx::Global, "General"),
        (Ctx::Editor, "Editor"),
        (Ctx::Sidebar, "Sidebar (when focused)"),
    ] {
        for (cmd, name, _) in COMMANDS {
            let keys: Vec<String> = km
                .bindings
                .iter()
                .filter(|b| b.cmd == *cmd && b.ctx == ctx)
                .map(|b| keymap::display(&b.keys))
                .collect();
            if !keys.is_empty() {
                rows.push(KeymapRow {
                    section: section.to_string(),
                    keys: keys.join(", "),
                    desc: name.to_string(),
                    cmd: Some(*cmd),
                });
            }
        }
    }
    for d in &km.docs {
        rows.push(KeymapRow {
            section: d.section.clone(),
            keys: keymap::display(&d.keys),
            desc: d.description.clone(),
            cmd: None,
        });
    }
    for (cmd, name, in_palette) in COMMANDS {
        if *in_palette && km.keys_for(*cmd).is_empty() {
            rows.push(KeymapRow {
                section: "No shortcut (use the command palette)".into(),
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
    /// Nothing is selected until Ctrl+N / Ctrl+P (see docs/SPEC.md §5).
    pub sel: Option<usize>,
}

pub struct Hover {
    pub lines: Vec<String>,
    /// Diagnostics at the hovered position: (severity, message).
    pub diagnostics: Vec<(u8, String)>,
    /// Buffer position the popup points at (mouse hover); `None` = at the cursor.
    pub anchor: Option<Pos>,
}

/// Mouse-hover tracking: the word under the pointer and since when it's been there.
struct HoverTrack {
    buf: usize,
    word: (Pos, Pos),
    pos: Pos,
    since: Instant,
    requested: bool,
}

/// Most editor groups (layout columns).
pub const MAX_GROUPS: usize = 4;

/// Defaults-beside-user-settings view (Preferences: Settings).
pub struct SettingsView {
    pub defaults_id: usize,
    pub user_id: usize,
    /// Layout to return to when the view closes.
    pub prev_layout: usize,
}

pub struct App {
    /// The open project (or anonymous folder set) shown in the sidebar.
    pub project: Project,
    pub recent: Recent,
    last_autosave: Instant,
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
    indexing: HashSet<PathBuf>,
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
    pub console: Console,
    /// Right-click menu, if open.
    pub context_menu: Option<menu::ContextMenu>,
    contexts: HashMap<String, Vec<Option<menu::Item>>>,
    hover_track: Option<HoverTrack>,
    pub panel: Panel,
    pub terminals: Vec<crate::terminal::Terminal>,
    next_term_id: usize,
    /// Sidebar width in points (drag its edge to resize).
    pub sidebar_width: f32,
    /// Editor groups (Sublime's layout columns), 1–4.
    pub layout: usize,
    /// Focused group.
    pub group: usize,
    /// Last active buffer id per group.
    pub group_active: [Option<usize>; MAX_GROUPS],
    /// Width of each group as a fraction of the editor area (sums to 1).
    pub group_split: Vec<f32>,
    /// Open while "Preferences: Settings" shows defaults beside the user file.
    pub settings_view: Option<SettingsView>,
    pub keymap: Keymap,
    pub menus: Vec<Menu>,
    /// Server name → install log (present while installing or after a failure).
    pub install_log: HashMap<String, Vec<String>>,
    pub installing: HashSet<String>,
    pub install_failed: HashMap<String, String>,
    /// Installed Sublime packages (folder names).
    pub packages: Vec<String>,
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
    pub fn new(project: Project, config: Config, tx: EventTx) -> Self {
        let syn = Syntax::load(&config.editor.theme);
        let mut tree = Tree::new();
        tree.anim = Duration::from_millis(if config.editor.animations { 160 } else { 0 });
        tree.set_roots(project.folders.clone(), false);
        let recent = Recent::load(config.data_dir.as_deref());
        let mut app = App {
            project,
            recent,
            last_autosave: Instant::now(),
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
            indexing: HashSet::new(),
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
            console: Console::default(),
            context_menu: None,
            contexts: HashMap::new(),
            hover_track: None,
            panel: Panel {
                visible: false,
                tab: PanelTab::Console,
                height: 240.0,
            },
            terminals: vec![],
            next_term_id: 1,
            sidebar_width: 250.0,
            layout: 1,
            group: 0,
            group_active: [None; MAX_GROUPS],
            group_split: vec![1.0],
            settings_view: None,
            keymap: Keymap::default(),
            menus: vec![],
            install_log: HashMap::new(),
            installing: HashSet::new(),
            install_failed: HashMap::new(),
            packages: vec![],
            config,
            tx,
        };
        app.apply_project_config();
        app.recent.touch(&app.project);
        app.recent.save(app.config.data_dir.as_deref());
        let errs = app.rebuild_keys_and_menus();
        if let Some(e) = app.config.error.clone().or(errs) {
            app.msg(format!("Config error: {e}"));
        }
        // User syntaxes/themes from Packages/ load in the background.
        if let Some(dir) = app.config.packages_dir().filter(|d| d.is_dir()) {
            app.packages = packages::list(&dir);
            packages::load_async(Some(dir), app.tx.clone());
        }
        app
    }

    pub fn event_tx(&self) -> EventTx {
        self.tx.clone()
    }

    pub fn msg(&mut self, s: impl Into<String>) {
        let s = s.into();
        crate::log::info("app", &s);
        self.status = Some((s, Instant::now()));
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

    /// The active buffer of the focused group (None when that group has no tabs).
    pub fn buf(&self) -> Option<&Buffer> {
        self.buffers.get(self.cur).filter(|b| b.group == self.group)
    }

    pub fn buf_mut(&mut self) -> Option<&mut Buffer> {
        let g = self.group;
        self.buffers.get_mut(self.cur).filter(|b| b.group == g)
    }

    // ===================================================================== groups

    /// Make buffer `i` current; focus follows it to its group.
    pub fn set_cur(&mut self, i: usize) {
        self.cur = i;
        if let Some(b) = self.buffers.get(i) {
            self.group = b.group;
            self.group_active[b.group] = Some(b.id);
        }
    }

    /// Add a buffer to the focused group; returns its index.
    fn push_buffer(&mut self, mut b: Buffer) -> usize {
        b.group = self.group;
        self.buffers.push(b);
        self.buffers.len() - 1
    }

    /// Buffer indices in a group, in tab order.
    pub fn group_tabs(&self, g: usize) -> Vec<usize> {
        (0..self.buffers.len())
            .filter(|&i| self.buffers[i].group == g)
            .collect()
    }

    /// The buffer a group shows: its last active one, else its first tab.
    pub fn group_shown(&self, g: usize) -> Option<usize> {
        self.group_active[g]
            .and_then(|id| self.buf_index(id))
            .filter(|&i| self.buffers[i].group == g)
            .or_else(|| self.group_tabs(g).first().copied())
    }

    pub fn focus_group(&mut self, g: usize) {
        if g >= self.layout {
            return;
        }
        self.group = g;
        if let Some(i) = self.group_shown(g) {
            self.set_cur(i);
        }
        self.focus = Focus::Editor;
        self.completion = None;
        self.hover = None;
    }

    /// Switch to 1–4 columns. Tabs of removed groups move into the last remaining one.
    pub fn set_layout(&mut self, n: usize) {
        let n = n.clamp(1, MAX_GROUPS);
        if n < self.layout {
            let last = n - 1;
            for b in &mut self.buffers {
                if b.group > last {
                    b.group = last;
                }
            }
            for g in n..MAX_GROUPS {
                self.group_active[g] = None;
            }
        }
        self.layout = n;
        self.group_split = vec![1.0 / n as f32; n];
        let g = self.group.min(n - 1);
        let keep = self.buf().map(|b| b.id);
        self.group = g;
        match keep.and_then(|id| self.buf_index(id)) {
            Some(i) => self.set_cur(i),
            None => {
                if let Some(i) = self.group_shown(g) {
                    self.set_cur(i);
                }
            }
        }
    }

    /// Move the current file to group `g` (adding columns if needed); focus follows it.
    pub fn move_to_group(&mut self, g: usize) {
        if g >= MAX_GROUPS {
            return;
        }
        let Some(id) = self.buf().map(|b| b.id) else {
            return;
        };
        if g >= self.layout {
            self.set_layout(g + 1);
        }
        let i = self.buf_index(id).unwrap();
        let from = self.buffers[i].group;
        if from == g {
            return;
        }
        // The group it leaves shows its neighbour (the tab to the left, else the first).
        let rest: Vec<usize> = self
            .group_tabs(from)
            .into_iter()
            .filter(|&t| t != i)
            .collect();
        let neighbour = rest
            .iter()
            .rev()
            .find(|&&t| t < i)
            .or(rest.first())
            .copied();
        self.group_active[from] = neighbour.map(|t| self.buffers[t].id);
        self.buffers[i].group = g;
        self.set_cur(i);
        self.focus = Focus::Editor;
    }

    /// After a buffer in group `g` closed at index `i`, show a neighbouring tab of the
    /// same group (the one to its left, like Sublime), keeping focus in that group.
    fn reselect_after_close(&mut self, g: usize, i: usize) {
        let tabs = self.group_tabs(g);
        let pick = tabs
            .iter()
            .rev()
            .find(|&&t| t < i)
            .or_else(|| tabs.first())
            .copied();
        match pick {
            Some(t) => self.set_cur(t),
            None => {
                self.group_active[g] = None;
                self.group = g;
                // Keep `cur` valid; buf() is None for an empty focused group.
                self.cur = self.cur.min(self.buffers.len().saturating_sub(1));
            }
        }
    }

    /// Cycle tabs within the focused group.
    fn cycle_tab(&mut self, d: isize) {
        let tabs = self.group_tabs(self.group);
        if tabs.is_empty() {
            return;
        }
        let at = tabs.iter().position(|&t| t == self.cur).unwrap_or(0) as isize;
        let n = tabs.len() as isize;
        self.set_cur(tabs[(at + d).rem_euclid(n) as usize]);
    }

    fn buf_index(&self, id: usize) -> Option<usize> {
        self.buffers.iter().position(|b| b.id == id)
    }

    pub fn title(&self) -> String {
        let root = self.project.name();
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

    /// A path for display: relative to its project folder (prefixed with the folder's name
    /// when the project has several).
    pub fn rel(&self, p: &Path) -> String {
        match self.project.folder_of(p) {
            Some(f) => {
                let rest = p.strip_prefix(&f.path).unwrap_or(p).display().to_string();
                if self.project.folders.len() > 1 {
                    if rest.is_empty() {
                        f.name.clone()
                    } else {
                        format!("{}/{rest}", f.name)
                    }
                } else {
                    rest
                }
            }
            None => p.display().to_string(),
        }
    }

    /// Where relative paths typed in prompts start from.
    pub fn base_dir(&self) -> PathBuf {
        self.project
            .primary()
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
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
        if self.context_menu.is_some() {
            self.context_key(k);
            return true;
        }
        if self.menu.is_some() {
            self.menu_key(k);
            return true;
        }
        if self.modal.is_some() {
            // Cmd+V into prompts/palettes even when a native menu swallowed the paste event.
            if k.ctrl
                && !k.shift
                && !k.alt
                && k.code == KeyCode::Char('v')
                && matches!(self.modal, Some(Modal::Prompt(_) | Modal::Palette(_)))
            {
                let t = self.clipboard_text();
                self.on_paste(&t);
                return true;
            }
            self.modal_key(k);
            return true;
        }
        // Completion navigation (literal Ctrl on macOS; on other platforms Ctrl+N/P only
        // navigate while the list is open).
        if self.completion.is_some()
            && self.focus == Focus::Editor
            && k.ctrl
            && !k.sup
            && !k.shift
            && !k.alt
            && matches!(k.code, KeyCode::Char('n') | KeyCode::Char('p'))
        {
            self.comp_move(if k.code == KeyCode::Char('n') { 1 } else { -1 });
            return true;
        }
        if self.hover.is_some() && k.code == KeyCode::Esc {
            self.hover = None;
            return true;
        }
        if self.focus == Focus::Terminal && self.terminal_shown().is_some() {
            return self.terminal_key(k);
        }
        if self.focus == Focus::Console && self.console_shown() {
            if k.ctrl && !k.shift && !k.alt && k.code == KeyCode::Char('a') {
                self.console_select_all();
                return true;
            }
            if let Some(cmd) = self.keymap.lookup(k, Ctx::Global) {
                self.run(cmd);
            } else {
                self.console_key(k);
            }
            return true;
        }
        let sidebar = self.focus == Focus::Sidebar && self.sidebar_visible;
        let own = if sidebar {
            self.keymap.lookup(k, Ctx::Sidebar)
        } else {
            None
        };
        if let Some(cmd) = own.or_else(|| self.keymap.lookup(k, Ctx::Global)) {
            self.run(cmd);
            return true;
        }
        // Select tab N of the focused group: Cmd+N on macOS, Alt+N elsewhere (Sublime;
        // Ctrl+N focuses group N there).
        let tab_mod = if cfg!(target_os = "macos") {
            k.sup && !k.alt
        } else {
            k.alt && !k.ctrl
        };
        if let (KeyCode::Char(c @ '1'..='9'), true, false) = (k.code, tab_mod, k.shift) {
            let n = c as usize - '1' as usize;
            if let Some(&i) = self.group_tabs(self.group).get(n) {
                self.set_cur(i);
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
            if self.context_menu.is_some() {
                self.context_char(c);
                continue;
            }
            if self.menu.is_some() {
                self.menu_char(c);
                continue;
            }
            let in_term = self.focus == Focus::Terminal && self.terminal_shown().is_some();
            let in_console = self.focus == Focus::Console && self.console_shown();
            match &mut self.modal {
                Some(Modal::Keymap(v)) => {
                    v.query.push(c);
                    v.sel = 0;
                    v.scroll = 0;
                }
                Some(Modal::Servers(_)) => self.servers_char(c),
                Some(Modal::LspInfo(_)) => self.lsp_info_char(c),
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
                None if in_term => {
                    if let Some(id) = self.terminal_shown()
                        && self.terminals.iter().any(|t| t.id == id && t.is_exited())
                    {
                        // "Press any key to close" covers letters too.
                        self.close_terminal(id);
                        return;
                    }
                    let mut b = [0u8; 4];
                    let bytes = c.encode_utf8(&mut b).as_bytes().to_vec();
                    if let Some(t) = self.current_terminal_mut() {
                        t.sel = None;
                        t.write(&bytes);
                    }
                }
                None if in_console => {
                    let b = crate::editor::byte_idx(&self.console.input, self.console.cursor);
                    self.console.input.insert(b, c);
                    self.console.cursor += 1;
                }
                None if self.focus == Focus::Sidebar && self.sidebar_visible => self.tree_char(c),
                None => self.editor_text(c),
            }
        }
    }

    pub fn on_paste(&mut self, s: &str) {
        let in_term = self.focus == Focus::Terminal && self.terminal_shown().is_some();
        let in_console = self.focus == Focus::Console && self.console_shown();
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
            None if in_term => {
                if let Some(t) = self.current_terminal_mut() {
                    t.paste(s);
                }
            }
            None if in_console => {
                let line = s.lines().next().unwrap_or("");
                let b = crate::editor::byte_idx(&self.console.input, self.console.cursor);
                self.console.input.insert_str(b, line);
                self.console.cursor += char_len(line);
            }
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
        if self.focus == Focus::Terminal && self.terminal_shown().is_some() {
            // Copy the selection; with nothing selected the key goes to the shell (on
            // Windows/Linux, Ctrl+C / Ctrl+X arrive here as copy/cut events).
            if let Some(t) = self.current_terminal_mut() {
                match t.selection_text() {
                    Some(text) if !cut => {
                        t.sel = None;
                        self.pending_copy = Some(text);
                    }
                    _ => t.write(if cut { b"\x18" } else { b"\x03" }),
                }
            }
            return;
        }
        if self.focus == Focus::Console && self.console_shown() {
            if let Some(t) = self.console_selection_text() {
                self.pending_copy = Some(t);
            }
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
                    std::thread::sleep(Duration::from_millis(20));
                    let code = c.exit_code();
                    let name = c.server.clone();
                    let why = code
                        .map(|c| format!(" (exit code {c})"))
                        .unwrap_or_default();
                    crate::log::warn(&format!("lsp:{name}"), format!("server exited{why}"));
                    self.msg(format!(
                        "{name} exited{why} — see the console (Ctrl+`) or LSP Info"
                    ));
                }
            }
            AppEvent::Install(name, ev) => self.on_install(name, ev),
            AppEvent::Packages(l) => self.on_packages(*l),
            AppEvent::PackageInstalled(res) => match res {
                Ok(name) => {
                    self.msg(format!("Installed package {name} — loading syntaxes…"));
                    self.run(Command::ReloadPackages);
                }
                Err(e) => self.msg(format!("Package install failed: {e}")),
            },
            AppEvent::Menu(id) => self.on_menu_command(&id),
            AppEvent::Files(root, list) => {
                self.indexing.remove(&root);
                self.files.insert(root, list);
                if matches!(&self.modal, Some(Modal::Palette(p)) if p.mode == Mode::Files) {
                    let (items, loading) = self.project_file_items();
                    if let Some(p) = self.palette() {
                        p.loading = loading;
                        p.set_items(items);
                    }
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
            Some(Modal::Servers(v)) => self.servers_key(k, v),
            Some(Modal::LspInfo(v)) => self.lsp_info_key(k, v),
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
            ConfirmKind::UninstallServer(name) => {
                let Some(data) = self.config.data_dir.clone() else {
                    return;
                };
                match installer::uninstall(&data, &name) {
                    Ok(()) => {
                        self.install_log.remove(&name);
                        self.lsp.reset_server(&name);
                        self.msg(format!("Uninstalled {name}"));
                        self.reopen_lsp_docs();
                    }
                    Err(e) => self.msg(format!("Uninstall failed: {e}")),
                }
                self.modal = Some(Modal::Servers(ServersView {
                    sel: self.servers_sel_of(&name),
                    details: false,
                }));
            }
            ConfirmKind::RemovePackage(name) => {
                let Some(dir) = self.config.packages_dir() else {
                    return;
                };
                match fileops::trash(&[dir.join(&name)]) {
                    Ok(()) => {
                        self.msg(format!("Moved package {name} to the Trash"));
                        self.run(Command::ReloadPackages);
                    }
                    Err(e) => self.msg(format!("Remove failed: {e}")),
                }
            }
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
                    PromptKind::RenameMove(_, base) => base.clone(),
                    _ => self.base_dir(),
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
            PromptKind::RenameMove(src, base) => {
                if input.is_empty() {
                    return;
                }
                let dst = self.resolve(&base, &input);
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
                let path = self.resolve(&self.base_dir(), &input);
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
                let path = self.resolve(&self.base_dir(), &input);
                let path = path.canonicalize().unwrap_or(path);
                if path.extension().is_some_and(|e| e == project::PROJECT_EXT) {
                    self.open_project_file(&path);
                } else if path.is_dir() {
                    self.switch_project(Project::from_folders(vec![path]));
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
            PromptKind::OpenProject => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.base_dir(), &input);
                if path.is_dir() {
                    self.switch_project(Project::from_folders(vec![
                        path.canonicalize().unwrap_or(path),
                    ]));
                } else {
                    self.open_project_file(&path);
                }
            }
            PromptKind::SaveProjectAs => {
                if input.is_empty() {
                    return;
                }
                let mut path = self.resolve(&self.base_dir(), &input);
                if path.extension().is_none_or(|e| e != project::PROJECT_EXT) {
                    path.set_extension(project::PROJECT_EXT);
                }
                let old_ws = self.project.workspace_file(self.config.data_dir.as_deref());
                match self.project.save_as(&path) {
                    Ok(()) => {
                        if let Some(old) = old_ws
                            && self
                                .project
                                .file
                                .as_ref()
                                .is_some_and(|f| f.with_extension(project::WORKSPACE_EXT) != old)
                        {
                            let _ = std::fs::remove_file(old);
                        }
                        self.save_workspace();
                        self.recent.touch(&self.project);
                        self.recent.save(self.config.data_dir.as_deref());
                        self.refresh_tree();
                        self.msg(format!("Saved project {}", path.display()));
                    }
                    Err(e) => self.msg(format!("Save project failed: {e}")),
                }
            }
            PromptKind::AddFolder => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.base_dir(), &input);
                let path = path.canonicalize().unwrap_or(path);
                if !path.is_dir() {
                    return self.msg(format!("Not a folder: {input}"));
                }
                if self.project.add_folder(path.clone()) {
                    self.project_folders_changed();
                    self.tree.reveal(&path);
                }
            }
            PromptKind::PackageUrl => {
                if input.is_empty() {
                    return;
                }
                let Some(dir) = self.config.packages_dir() else {
                    return self.msg("No config folder for packages");
                };
                self.msg(format!("Cloning {input}…"));
                packages::install_git(dir, input, self.tx.clone());
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

    // ===================================================================== projects & workspaces

    /// Merge project `[[languages]]`/`[[servers]]` and `[settings]` over the loaded config.
    fn apply_project_config(&mut self) {
        let mut errs = vec![];
        if let Some(snippet) = self.project.config_snippet()
            && let Err(e) = self.config.merge_str(&snippet)
        {
            errs.push(e);
        }
        if !self.project.settings.is_empty() {
            match project::apply_settings(&self.config.editor, &self.project.settings) {
                Ok(ed) => self.config.editor = ed,
                Err(e) => errs.push(e),
            }
        }
        self.font_size = self.config.editor.font_size;
        if !errs.is_empty() {
            self.msg(format!("Project settings: {}", errs.join("; ")));
        }
    }

    fn open_project_file(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(p) => self.switch_project(p),
            Err(e) => self.msg(format!("Open project failed: {e}")),
        }
    }

    /// Save the current session, close everything, and open another project's session.
    pub fn switch_project(&mut self, p: Project) {
        self.save_workspace();
        for i in 0..self.buffers.len() {
            self.close_lsp_doc(i);
        }
        self.lsp.restart();
        self.buffers.clear();
        self.cur = 0;
        self.group_active = [None; MAX_GROUPS];
        self.diagnostics.clear();
        self.completion = None;
        self.hover = None;
        self.modal = None;
        self.files.clear();
        self.closed.clear();
        self.jumps.clear();
        self.tree.expanded.clear();
        self.tree_revealed = None;
        self.project = p;
        // Reload config so the previous project's settings don't leak into this one.
        let data = self.config.data_dir.clone();
        let path = self.config.path.clone();
        self.config = Config::load_from(path);
        if data.is_some() {
            self.config.data_dir = data;
        }
        self.apply_project_config();
        let _ = self.rebuild_keys_and_menus();
        self.syn.set_theme(&self.config.editor.theme);
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        self.restore_workspace();
        self.recent.touch(&self.project);
        self.recent.save(self.config.data_dir.as_deref());
        self.msg(format!("Opened {}", self.project.name()));
    }

    /// Folders were added/removed: update the sidebar and the project file (if any).
    fn project_folders_changed(&mut self) {
        if let Some(f) = self.project.file.clone()
            && let Err(e) = self.project.save_as(&f)
        {
            self.msg(format!("Couldn't update {}: {e}", f.display()));
        }
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        self.files.clear();
        self.sidebar_visible = true;
    }

    /// The project file was edited in farsub: apply folders and settings in place.
    fn reload_project_file(&mut self) {
        let Some(f) = self.project.file.clone() else {
            return;
        };
        match Project::load(&f) {
            Ok(p) => {
                self.project = p;
                self.reload_config();
                self.tree
                    .set_roots(self.project.folders.clone(), self.show_hidden);
                self.files.clear();
                self.msg("Project reloaded");
            }
            Err(e) => self.msg(format!("Project error: {e}")),
        }
    }

    pub fn capture_workspace(&self) -> Workspace {
        let hot = self.config.editor.hot_exit;
        let buffers = self
            .buffers
            .iter()
            .filter(|b| b.path.is_some() || (hot && b.modified))
            .map(|b| BufferState {
                group: b.group,
                path: b.path.clone(),
                cursor: (b.cursor.line, b.cursor.col),
                anchor: b.anchor.map(|a| (a.line, a.col)),
                scroll: (b.scroll_y, b.scroll_x),
                unsaved: (hot && b.modified).then(|| b.text()),
                syntax: b.syntax_override.then(|| b.hl.syntax.clone()),
                indent: b
                    .indent_override
                    .then(|| (b.indent_unit == "\t", b.tab_size)),
            })
            .collect();
        let mut expanded: Vec<PathBuf> = self
            .tree
            .expanded
            .iter()
            .filter(|p| !self.tree.is_root(p))
            .cloned()
            .collect();
        expanded.sort();
        let collapsed_roots = self
            .project
            .folders
            .iter()
            .filter(|f| !self.tree.expanded.contains(&f.path));
        let mut ws = Workspace {
            buffers,
            active: self.cur,
            sidebar_visible: self.sidebar_visible,
            show_hidden: self.show_hidden,
            expanded,
            search: self.search.clone(),
            closed: self.closed.iter().rev().take(20).rev().cloned().collect(),
            sidebar_width: Some(self.sidebar_width),
            console_visible: self.console_shown(),
            console_height: Some(self.panel.height),
            layout: self.layout,
            focused_group: self.group,
        };
        // Remember collapsed project folders as "!path" entries.
        ws.expanded
            .extend(collapsed_roots.map(|f| PathBuf::from(format!("!{}", f.path.display()))));
        ws
    }

    pub fn save_workspace(&mut self) {
        self.last_autosave = Instant::now();
        let Some(f) = self.project.workspace_file(self.config.data_dir.as_deref()) else {
            return;
        };
        if let Err(e) = self.capture_workspace().save(&f) {
            self.msg(format!("Couldn't save session: {e}"));
        }
    }

    /// Called by the GUI every frame; saves the session every 30 seconds.
    pub fn autosave_tick(&mut self) {
        if self.last_autosave.elapsed() > Duration::from_secs(30) {
            self.save_workspace();
        }
    }

    pub fn restore_workspace(&mut self) {
        let Some(f) = self.project.workspace_file(self.config.data_dir.as_deref()) else {
            return;
        };
        let Some(ws) = Workspace::load(&f) else {
            return;
        };
        self.set_layout(ws.layout.max(1));
        for bs in &ws.buffers {
            self.group = bs.group.min(self.layout - 1);
            let i = match (&bs.path, &bs.unsaved) {
                (Some(p), None) if p.is_file() => self.open_file(p, false),
                (_, None) => None,
                (path, Some(text)) => {
                    let id = self.next_buf_id;
                    self.next_buf_id += 1;
                    let b = Buffer::from_text(
                        id,
                        path.clone(),
                        text,
                        HlCache::new("Plain Text".into()),
                    );
                    let i = self.push_buffer(b);
                    if let Some(p) = path.clone() {
                        self.set_buffer_path(i, p);
                    }
                    self.buffers[i].modified = true;
                    self.lsp_open(i);
                    Some(i)
                }
            };
            if let Some(i) = i {
                let b = &mut self.buffers[i];
                b.group = bs.group.min(self.layout - 1);
                b.cursor = b.clamp(Pos::new(bs.cursor.0, bs.cursor.1));
                b.anchor = bs.anchor.map(|a| b.clamp(Pos::new(a.0, a.1)));
                b.want_col = b.cursor.col;
                b.scroll_y = bs.scroll.0.min(b.lines.len().saturating_sub(1));
                b.scroll_x = bs.scroll.1;
                if let Some(sx) = &bs.syntax {
                    self.set_syntax_at(i, sx);
                }
                if let Some((tabs, w)) = bs.indent {
                    let b = &mut self.buffers[i];
                    b.set_tab_width(w);
                    b.set_indent(if tabs {
                        crate::editor::Indent::Tabs
                    } else {
                        crate::editor::Indent::Spaces(w)
                    });
                }
            }
        }
        // Each group shows its first tab, then the active file (and its group) is focused.
        for g in 0..self.layout {
            self.group_active[g] = self.group_tabs(g).first().map(|&i| self.buffers[i].id);
        }
        self.group = ws.focused_group.min(self.layout - 1);
        let active = ws.active.min(self.buffers.len().saturating_sub(1));
        if active < self.buffers.len() {
            self.set_cur(active);
        }
        self.sidebar_visible = ws.sidebar_visible;
        self.show_hidden = ws.show_hidden;
        for e in &ws.expanded {
            let s = e.to_string_lossy();
            if let Some(root) = s.strip_prefix('!') {
                self.tree.expanded.remove(Path::new(root));
            } else if e.is_dir() {
                self.tree.expanded.insert(e.clone());
            }
        }
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        self.tree.rebuild();
        self.search = ws.search.clone();
        self.closed = ws.closed.clone();
        if let Some(w) = ws.sidebar_width {
            self.sidebar_width = w.clamp(120.0, 900.0);
        }
        if let Some(h) = ws.console_height {
            self.panel.height = h.clamp(80.0, 1200.0);
        }
        if ws.console_visible {
            self.panel.visible = true;
            self.panel.tab = PanelTab::Console;
        }
    }

    // ===================================================================== sidebar

    fn focus_sidebar(&mut self) {
        self.sidebar_visible = true;
        self.focus = Focus::Sidebar;
        self.completion = None;
        self.hover = None;
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
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
        if let Some(cmd) = self.keymap.lookup(keymap::key_for_char(c), Ctx::Sidebar) {
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
                if self.tree.is_root(&src) {
                    return self.msg("That's a project folder — use Project › Remove Folder, or rename it on disk");
                }
                let base = self
                    .tree
                    .root_of(&src)
                    .map(|f| f.path.clone())
                    .unwrap_or_else(|| self.base_dir());
                let rel = src
                    .strip_prefix(&base)
                    .unwrap_or(&src)
                    .display()
                    .to_string();
                let title = format!(
                    "Rename / move to (relative to {})",
                    base.file_name().unwrap_or_default().to_string_lossy()
                );
                self.prompt(&title, &rel, PromptKind::RenameMove(src, base));
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
                    let parent = src
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.base_dir());
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
        let lang = self.config.language_for(&path, &first).cloned();
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

    /// Where the read-only default settings reference is written.
    pub fn defaults_settings_path(&self) -> PathBuf {
        self.config
            .data_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir)
            .join("Default Settings.toml")
    }

    /// Preferences: Settings — like Sublime: the full default settings (read-only) on the
    /// left, the user's file on the right, in a two-column layout.
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
        let defaults = self.defaults_settings_path();
        if let Some(parent) = defaults.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Always rewritten, so it matches this version of farsub.
        if let Err(e) = std::fs::write(&defaults, config::DEFAULT_SETTINGS) {
            return self.msg(format!("Could not write {}: {e}", defaults.display()));
        }
        let prev_layout = self
            .settings_view
            .take()
            .map(|v| v.prev_layout)
            .unwrap_or(self.layout);
        if self.layout < 2 {
            self.set_layout(2);
        }
        let place = |app: &mut App, p: &Path, g: usize| -> Option<usize> {
            app.group = g;
            let i = app.open_file(p, true)?;
            app.buffers[i].group = g;
            app.set_cur(i);
            Some(app.buffers[i].id)
        };
        let Some(defaults_id) = place(self, &defaults, 0) else {
            return;
        };
        if let Some(i) = self.buf_index(defaults_id) {
            self.buffers[i].read_only = true;
        }
        let Some(user_id) = place(self, &path, 1) else {
            return;
        };
        self.focus = Focus::Editor;
        self.settings_view = Some(SettingsView {
            defaults_id,
            user_id,
            prev_layout,
        });
    }

    fn reload_config(&mut self) {
        let mut new = Config::load_from(self.config.path.clone());
        if let Some(snippet) = self.project.config_snippet() {
            let _ = new.merge_str(&snippet);
        }
        if let Ok(ed) = project::apply_settings(&new.editor, &self.project.settings) {
            new.editor = ed;
        }
        if let Some(e) = &new.error {
            return self.msg(format!("Config error: {e}"));
        }
        let langs_changed =
            new.languages != self.config.languages || new.servers != self.config.servers;
        let data = self.config.data_dir.clone();
        self.config = new;
        if self.config.data_dir.is_none() {
            self.config.data_dir = data;
        }
        if let Some(e) = self.rebuild_keys_and_menus() {
            self.msg(format!("Config error: {e}"));
        }
        if !self.syn.set_theme(&self.config.editor.theme) {
            self.msg(format!("Unknown theme '{}'", self.config.editor.theme));
        }
        for b in &mut self.buffers {
            b.hl.invalidate(0);
        }
        self.font_size = self.config.editor.font_size;
        self.tree.anim = Duration::from_millis(if self.config.editor.animations {
            160
        } else {
            0
        });
        for i in 0..self.buffers.len() {
            if let Some(p) = self.buffers[i].path.clone() {
                let first = self.buffers[i].lines[0].clone();
                let lang = self.config.language_for(&p, &first).cloned();
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

    /// Goto Anything items for every project folder that has been indexed. Returns true if
    /// some folders are still being indexed.
    fn project_file_items(&mut self) -> (Vec<Item>, bool) {
        let multi = self.project.folders.len() > 1;
        let mut items = vec![];
        let mut loading = false;
        for f in self.project.folders.clone() {
            match self.files.get(&f.path) {
                Some(list) => items.extend(list.iter().map(|rel| Item {
                    label: if multi {
                        format!("{}/{rel}", f.name)
                    } else {
                        rel.clone()
                    },
                    detail: String::new(),
                    action: Action::Open(f.path.join(rel), None),
                })),
                None => {
                    loading = true;
                    self.start_indexing(f);
                }
            }
        }
        (items, loading)
    }

    fn start_indexing(&mut self, folder: crate::project::Folder) {
        if !self.indexing.insert(folder.path.clone()) {
            return;
        }
        let root = folder.path.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let excl = folder.clone();
            let walker = ignore::WalkBuilder::new(&root)
                .hidden(true)
                .git_ignore(true)
                // .gitignore applies even when the folder isn't a git repository.
                .require_git(false)
                .filter_entry(move |e| {
                    let name = e.file_name().to_string_lossy();
                    let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                    e.depth() == 0
                        || (!excl.excluded(&name, is_dir) && !name.ends_with(".farsub-workspace"))
                })
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
                title = format!("Files in {}", self.project.name());
                let (items, still) = self.project_file_items();
                loading = still;
                items
            }
            Mode::Commands => COMMANDS
                .iter()
                .filter(|c| c.2)
                .map(|(c, name, _)| Item {
                    label: name.to_string(),
                    detail: self.keymap.shortcut(*c),
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
            Mode::Projects => {
                let mut v: Vec<Item> = self
                    .recent
                    .projects
                    .iter()
                    .filter(|p| p.is_file())
                    .map(|p| Item {
                        label: p
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        detail: p.display().to_string(),
                        action: Action::Project(Last::Project(p.clone())),
                    })
                    .collect();
                v.extend(
                    self.recent
                        .folders
                        .iter()
                        .filter(|fs| fs.iter().all(|f| f.is_dir()))
                        .map(|fs| Item {
                            label: fs
                                .iter()
                                .map(|f| {
                                    f.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned()
                                })
                                .collect::<Vec<_>>()
                                .join(", "),
                            detail: fs
                                .iter()
                                .map(|f| f.display().to_string())
                                .collect::<Vec<_>>()
                                .join("  ·  "),
                            action: Action::Project(Last::Folders(fs.clone())),
                        }),
                );
                v
            }
            Mode::Syntaxes => {
                let cur = self.buf().map(|b| b.hl.syntax.clone()).unwrap_or_default();
                let mut v: Vec<Item> = self
                    .syn
                    .ss
                    .syntaxes()
                    .iter()
                    .filter(|s| !s.hidden)
                    .map(|s| Item {
                        label: s.name.clone(),
                        detail: if s.name == cur {
                            "current".into()
                        } else {
                            s.file_extensions
                                .iter()
                                .take(4)
                                .map(|e| format!(".{e}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        },
                        action: Action::Syntax(s.name.clone()),
                    })
                    .collect();
                v.sort_by_key(|i| i.label.to_lowercase());
                v.dedup_by(|a, b| a.label == b.label);
                v
            }
            Mode::Indentation => {
                let Some(b) = self.buf() else { return };
                let kind = b.indent_kind();
                let cur = |on: bool| {
                    if on {
                        "current".to_string()
                    } else {
                        String::new()
                    }
                };
                let guess = match crate::editor::guess_indent(&b.lines) {
                    Some(crate::editor::Indent::Tabs) => "detected: tabs".to_string(),
                    Some(crate::editor::Indent::Spaces(n)) => format!("detected: {n} spaces"),
                    None => "nothing to detect".into(),
                };
                let mut v = vec![
                    Item {
                        label: "Indent Using Spaces".into(),
                        detail: cur(kind != crate::editor::Indent::Tabs),
                        action: Action::Indent(IndentAction::Spaces),
                    },
                    Item {
                        label: "Indent Using Tabs".into(),
                        detail: cur(kind == crate::editor::Indent::Tabs),
                        action: Action::Indent(IndentAction::Tabs),
                    },
                ];
                for n in 1..=8 {
                    v.push(Item {
                        label: format!("Tab Width: {n}"),
                        detail: cur(b.tab_size == n),
                        action: Action::Indent(IndentAction::Width(n)),
                    });
                }
                v.push(Item {
                    label: "Guess Settings From Buffer".into(),
                    detail: guess,
                    action: Action::Indent(IndentAction::Guess),
                });
                v.push(Item {
                    label: "Convert Indentation to Spaces".into(),
                    detail: String::new(),
                    action: Action::Indent(IndentAction::ToSpaces),
                });
                v.push(Item {
                    label: "Convert Indentation to Tabs".into(),
                    detail: String::new(),
                    action: Action::Indent(IndentAction::ToTabs),
                });
                v
            }
            Mode::Packages => self
                .packages
                .iter()
                .map(|n| Item {
                    label: n.clone(),
                    detail: String::new(),
                    action: Action::Package(n.clone()),
                })
                .collect(),
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
                    self.set_cur(bi);
                }
                let pg = self.page;
                if let Some(b) = self.buf_mut() {
                    b.goto(pos);
                    center(b, pg);
                }
            }
            Action::Buffer(i) => {
                if i < self.buffers.len() {
                    self.set_cur(i);
                    self.focus = Focus::Editor;
                }
            }
            Action::Project(last) => match last {
                Last::Project(f) => self.open_project_file(&f),
                Last::Folders(fs) => self.switch_project(Project::from_folders(fs)),
            },
            Action::Indent(a) => {
                let Some(b) = self.buf_mut() else { return };
                let note = match a {
                    IndentAction::Spaces => {
                        let w = b.tab_size;
                        b.set_indent(crate::editor::Indent::Spaces(w));
                        format!("Indent using {w} spaces")
                    }
                    IndentAction::Tabs => {
                        b.set_indent(crate::editor::Indent::Tabs);
                        "Indent using tabs".into()
                    }
                    IndentAction::Width(n) => {
                        b.set_tab_width(n);
                        format!("Tab width: {n}")
                    }
                    IndentAction::Guess => match b.guess_indentation() {
                        Some(crate::editor::Indent::Tabs) => "Detected: tabs".into(),
                        Some(crate::editor::Indent::Spaces(n)) => format!("Detected: {n} spaces"),
                        None => "No indentation to detect".into(),
                    },
                    IndentAction::ToSpaces => format!(
                        "Converted {} line(s) to spaces",
                        b.convert_indentation(false)
                    ),
                    IndentAction::ToTabs => {
                        format!("Converted {} line(s) to tabs", b.convert_indentation(true))
                    }
                };
                self.msg(note);
            }
            Action::Syntax(n) => {
                let i = self.cur;
                if i < self.buffers.len() {
                    self.set_syntax_at(i, &n);
                    let lang = self.buffers[i]
                        .lang
                        .clone()
                        .unwrap_or_else(|| "no language".into());
                    self.msg(format!("Syntax: {n} ({lang})"));
                }
            }
            Action::Package(n) => {
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Move package {n} to the Trash?"),
                    kind: ConfirmKind::RemovePackage(n),
                }));
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
                self.set_cur(i);
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
        let i = self.push_buffer(b);
        self.set_buffer_path(i, path);
        if std::fs::metadata(&self.buffers[i].path.clone().unwrap())
            .is_ok_and(|m| m.permissions().readonly())
        {
            self.buffers[i].read_only = true;
        }
        if self.buffers[i].path.as_deref() == Some(self.defaults_settings_path().as_path()) {
            self.buffers[i].read_only = true;
        }
        if focus {
            self.set_cur(i);
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
        let i = self.push_buffer(b);
        self.set_cur(i);
        self.set_buffer_path(self.cur, path);
        self.focus = Focus::Editor;
    }

    /// Open a file at a 1-based line/column (CLI `file:line:col`).
    pub fn open_at(&mut self, path: &Path, line: Option<usize>, col: Option<usize>) {
        let i = if path.exists() {
            self.open_file(path, true)
        } else {
            self.open_new_path(path.to_path_buf());
            Some(self.cur)
        };
        if let (Some(i), Some(l)) = (i, line) {
            let pg = self.page;
            let b = &mut self.buffers[i];
            b.goto(Pos::new(
                l.saturating_sub(1),
                col.unwrap_or(1).saturating_sub(1),
            ));
            center(b, pg);
        }
    }

    #[cfg(test)]
    pub fn untitled_for_test(&mut self, text: &str) {
        self.open_untitled(text);
    }

    /// A new untitled buffer with some text (e.g. piped in on stdin).
    pub fn open_untitled(&mut self, text: &str) {
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let first = text.lines().next().unwrap_or("");
        let syntax = self.syn.detect(None, first);
        let mut b = Buffer::from_text(id, None, text, HlCache::new(syntax));
        b.modified = !text.is_empty();
        let i = self.push_buffer(b);
        self.set_cur(i);
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
                let in_config =
                    saved
                        .as_ref()
                        .zip(self.config.config_dir())
                        .is_some_and(|(p, d)| {
                            p.starts_with(&d) && p.extension().is_some_and(|e| e == "toml")
                        });
                if saved.is_some() && saved == self.project.file {
                    self.reload_project_file();
                } else if in_config {
                    self.reload_config();
                }
                self.tree.refresh();
            }
            Err(e) => self.msg(format!("Save failed: {e}")),
        }
    }

    fn close_buffer_at(&mut self, i: usize) {
        let was_cur = self.buffers.get(self.cur).map(|b| b.id);
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
            self.group_active = [None; MAX_GROUPS];
        } else if was_cur == Some(b.id) {
            self.reselect_after_close(b.group, i);
        } else if let Some(i) = was_cur.and_then(|id| self.buf_index(id)) {
            self.cur = i;
        }
        if self.group_active[b.group] == Some(b.id) {
            self.group_active[b.group] = None;
        }
        // Closing either side of the settings view closes the view (like closing
        // Sublime's settings window): the other side goes too unless it has unsaved
        // changes, and the previous layout comes back.
        if let Some(v) = self
            .settings_view
            .take_if(|v| v.defaults_id == b.id || v.user_id == b.id)
        {
            let other = if v.defaults_id == b.id {
                v.user_id
            } else {
                v.defaults_id
            };
            if let Some(oi) = self.buf_index(other)
                && !self.buffers[oi].modified
            {
                self.close_buffer_at(oi);
            }
            if self.layout != v.prev_layout {
                self.set_layout(v.prev_layout);
            }
        }
    }

    // ===================================================================== editor

    fn editor_key_inner(&mut self, k: Key) {
        if self.buf().is_none() {
            return;
        }
        // Completion list: Enter/Tab accept only an explicitly selected item (Ctrl+N/P);
        // otherwise keys do their normal thing and the list closes.
        if let Some(c) = &self.completion {
            let selected = c.sel.is_some();
            match k.code {
                KeyCode::Enter | KeyCode::Tab if !k.ctrl && !k.shift && selected => {
                    return self.comp_accept();
                }
                KeyCode::Esc => {
                    self.completion = None;
                    return;
                }
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Enter
                | KeyCode::Tab => {
                    self.completion = None;
                }
                _ => {}
            }
        }
        if self.hover.take().is_some() && k.code == KeyCode::Esc {
            return;
        }
        if let Some(cmd) = self.keymap.lookup(k, Ctx::Editor) {
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

    // ===================================================================== keys, menus, servers, packages

    /// Rebuild the keymap and menus from config; returns a combined error message.
    fn rebuild_keys_and_menus(&mut self) -> Option<String> {
        let (km, mut errs) = Keymap::from_config(&self.config);
        let (menus, merrs) = menu::build(&self.config.menus);
        errs.extend(merrs);
        let (contexts, cerrs) = menu::build_context(&self.config.context_menus);
        errs.extend(cerrs);
        self.contexts = contexts;
        self.keymap = km;
        self.menus = menus;
        (!errs.is_empty()).then(|| errs.join("; "))
    }

    /// System clipboard text (falls back to the editor's own clipboard).
    pub fn clipboard_text(&self) -> String {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .unwrap_or_else(|_| self.clip.clone())
    }

    /// A native menu item fired. If it has a shortcut, replay that key so the focused pane
    /// decides (Cmd+C copies files in the sidebar, text in the editor); otherwise run it.
    pub fn on_menu_command(&mut self, id: &str) {
        let Some(cmd) = crate::commands::from_id(id) else {
            return;
        };
        if !self.enabled(cmd) {
            return;
        }
        let key = self
            .keymap
            .primary(cmd)
            .filter(|b| b.ctx != Ctx::Sidebar)
            .map(|b| keymap::key_of(&b.spec));
        match key {
            Some(k)
                if self
                    .keymap
                    .lookup(k, Ctx::Global)
                    .or(self.keymap.lookup(k, Ctx::Editor))
                    .is_some() =>
            {
                self.on_key(k);
            }
            _ => self.run(cmd),
        }
    }

    pub fn server_status(&self, s: &crate::config::ServerConfig) -> ServerStatus {
        if self.installing.contains(&s.name) {
            return ServerStatus::Installing;
        }
        if let Some(d) = &self.config.data_dir
            && let Some(b) = installer::installed_bin(d, s)
        {
            return ServerStatus::Installed(b);
        }
        if let Some(e) = self.install_failed.get(&s.name) {
            return ServerStatus::Failed(e.clone());
        }
        match crate::lsp::find_server(&s.command) {
            Some(p) => ServerStatus::OnPath(p),
            None => ServerStatus::Missing,
        }
    }

    /// Servers in the manager's order: those for the current file's language first.
    pub fn server_rows(&self) -> Vec<crate::config::ServerConfig> {
        let cur = self
            .buf()
            .and_then(|b| b.lang.clone())
            .and_then(|l| self.config.language(&l).cloned());
        let mut v = self.config.servers.clone();
        v.sort_by_key(|s| {
            let rank = cur
                .as_ref()
                .and_then(|l| l.servers.iter().position(|n| n == &s.name))
                .unwrap_or(usize::MAX);
            (rank, s.name.clone())
        });
        v
    }

    fn servers_sel_of(&self, name: &str) -> usize {
        self.server_rows()
            .iter()
            .position(|s| s.name == name)
            .unwrap_or(0)
    }

    pub fn install_server(&mut self, name: &str) {
        let Some(s) = self.config.server(name).cloned() else {
            return;
        };
        if s.install.is_none() {
            return self.msg(format!(
                "{name} can't be installed automatically — install `{}` yourself",
                s.command
            ));
        }
        let Some(data) = self.config.data_dir.clone() else {
            return self.msg("No data folder to install into (set $HOME or $FARSUB_DATA)");
        };
        if !self.installing.insert(name.to_string()) {
            return;
        }
        self.install_failed.remove(name);
        self.install_log.insert(
            name.to_string(),
            vec![format!(
                "installing {name} ({})",
                s.install.as_ref().unwrap().source()
            )],
        );
        self.msg(format!("Installing {name}…"));
        installer::install(data, s, self.tx.clone());
    }

    fn on_install(&mut self, name: String, ev: InstallEvent) {
        match ev {
            InstallEvent::Log(l) => {
                crate::log::info(&format!("install:{name}"), &l);
                let log = self.install_log.entry(name).or_default();
                log.push(l);
                if log.len() > 200 {
                    log.remove(0);
                }
            }
            InstallEvent::Done(res) => {
                self.installing.remove(&name);
                match res {
                    Ok(_) => {
                        self.msg(format!("{name} installed"));
                        self.lsp.reset_server(&name);
                        self.reopen_lsp_docs();
                    }
                    Err(e) => {
                        crate::log::error(&format!("install:{name}"), &e);
                        self.msg(format!("{name}: install failed — {e}"));
                        self.install_failed.insert(name, e);
                    }
                }
            }
        }
    }

    /// Start servers for open files that don't have one yet (after installs/uninstalls).
    fn reopen_lsp_docs(&mut self) {
        self.lsp_warned.clear();
        for i in 0..self.buffers.len() {
            let alive = self.buffers[i]
                .lsp_client
                .and_then(|c| self.lsp.clients.get(&c))
                .is_some_and(|c| c.alive);
            if !alive {
                self.buffers[i].lsp_client = None;
                self.lsp_open(i);
            }
        }
    }

    fn servers_key(&mut self, k: Key, mut v: ServersView) {
        let n = self.server_rows().len();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Home => v.sel = 0,
            KeyCode::End => v.sel = n.saturating_sub(1),
            KeyCode::Enter => v.details = !v.details,
            _ => {}
        }
        self.modal = Some(Modal::Servers(v));
    }

    fn servers_char(&mut self, c: char) {
        let rows = self.server_rows();
        let Some(Modal::Servers(v)) = &mut self.modal else {
            return;
        };
        let n = rows.len();
        let sel = v.sel.min(n.saturating_sub(1));
        match c {
            'j' => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            'k' => v.sel = v.sel.saturating_sub(1),
            ' ' => v.details = !v.details,
            'i' | 'u' => {
                if let Some(s) = rows.get(sel) {
                    let name = s.name.clone();
                    self.install_server(&name);
                }
            }
            'x' | 'X' => {
                let Some(s) = rows.get(sel) else { return };
                let managed = self
                    .config
                    .data_dir
                    .as_ref()
                    .is_some_and(|d| installer::installed_bin(d, s).is_some());
                if !managed {
                    return self.msg(format!("{} wasn't installed by farsub", s.name));
                }
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Uninstall {}?", s.name),
                    kind: ConfirmKind::UninstallServer(s.name.clone()),
                }));
            }
            _ => {}
        }
    }

    fn on_packages(&mut self, l: packages::Loaded) {
        let n_syn = l.syntaxes.syntaxes().len();
        self.syn.ss = l.syntaxes;
        self.syn.user_themes = l.themes;
        self.syn.set_theme(&self.config.editor.theme);
        self.packages = l.packages;
        // Re-detect syntaxes for open files (a package may provide a better grammar).
        for i in 0..self.buffers.len() {
            match self.buffers[i].path.clone() {
                Some(p) => self.set_buffer_path(i, p),
                None => self.buffers[i].hl.invalidate(0),
            }
        }
        for e in &l.errors {
            crate::log::error("packages", e);
        }
        if !l.errors.is_empty() {
            self.msg(format!("Package errors: {}", l.errors.join("; ")));
        } else if !self.packages.is_empty() {
            self.msg(format!(
                "Loaded {} package(s), {n_syn} syntaxes",
                self.packages.len()
            ));
        }
    }

    // ===================================================================== syntax & LSP info

    /// The language config that goes with a syntax (an explicit `syntax = …` wins, then
    /// shared file extensions).
    fn language_for_syntax(&self, syntax: &str) -> Option<crate::config::LanguageConfig> {
        if let Some(l) = self
            .config
            .languages
            .iter()
            .rev()
            .find(|l| l.syntax.as_deref() == Some(syntax))
        {
            return Some(l.clone());
        }
        let sr = self.syn.ss.find_syntax_by_name(syntax)?;
        self.config
            .languages
            .iter()
            .rev()
            .find(|l| {
                l.extensions
                    .iter()
                    .any(|e| sr.file_extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
            })
            .cloned()
    }

    /// Set a buffer's syntax by hand; its language (comments, indentation, LSP) follows.
    pub fn set_syntax_at(&mut self, i: usize, name: &str) {
        if self.syn.ss.find_syntax_by_name(name).is_none() {
            return;
        }
        let lang = self.language_for_syntax(name);
        let b = &mut self.buffers[i];
        b.hl.reset(name.to_string());
        b.syntax_override = true;
        let new_lang = lang.as_ref().map(|l| l.name.clone());
        if new_lang != b.lang {
            self.close_lsp_doc(i);
            let b = &mut self.buffers[i];
            b.apply_language(lang.as_ref(), &self.config.editor);
            b.lang = new_lang;
            self.lsp_warned.clear();
            self.lsp_open(i);
        }
    }

    /// Everything LSP Info shows, as styled lines (kept here so it's testable).
    pub fn lsp_info(&self) -> Vec<(Tone, String)> {
        use Tone::*;
        let mut out: Vec<(Tone, String)> = vec![];
        let row = |k: &str, v: String| format!("  {k:<14}{v}");
        let b = self.buf();
        out.push((Heading, "This file".into()));
        let Some(b) = b else {
            out.push((Dim, "  No file open.".into()));
            return self.lsp_info_running(out);
        };
        out.push((
            Text,
            row(
                "file",
                b.path
                    .as_ref()
                    .map(|p| self.rel(p))
                    .unwrap_or_else(|| "untitled".into()),
            ),
        ));
        out.push((
            Text,
            row(
                "syntax",
                format!(
                    "{}{}",
                    b.hl.syntax,
                    if b.syntax_override {
                        "  (set by hand)"
                    } else {
                        ""
                    }
                ),
            ),
        ));
        let lang = b.lang.as_ref().and_then(|l| self.config.language(l));
        match lang {
            None => {
                out.push((
                    Warn,
                    row(
                        "language",
                        "none — no language config matches this file".into(),
                    ),
                ));
                out.push((
                    Dim,
                    "  Pick a syntax (click it in the status bar) or add languages/<name>.toml."
                        .into(),
                ));
            }
            Some(l) => {
                out.push((
                    Text,
                    row(
                        "language",
                        format!("{}  (languageId \"{}\")", l.name, l.language_id()),
                    ),
                ));
                if l.servers.is_empty() {
                    out.push((
                        Dim,
                        "  No language servers are configured for this language.".into(),
                    ));
                } else {
                    out.push((Text, String::new()));
                    out.push((
                        Heading,
                        "Servers for this language (first available is used)".into(),
                    ));
                    let attached = b.lsp_client.and_then(|c| self.lsp.clients.get(&c));
                    for name in &l.servers {
                        let Some(s) = self.config.server(name) else {
                            out.push((
                                Bad,
                                row(name, "not defined (no servers/<name>.toml)".into()),
                            ));
                            continue;
                        };
                        let running = self
                            .lsp
                            .clients
                            .values()
                            .any(|c| c.alive && c.server == *name);
                        let (tone, what) = if attached.is_some_and(|c| c.server == *name && c.alive)
                        {
                            (Good, "running — attached to this file".to_string())
                        } else if running {
                            (Good, "running (another project root)".to_string())
                        } else {
                            match self.server_status(s) {
                                ServerStatus::Installed(p) => {
                                    (Text, format!("installed  {}", p.display()))
                                }
                                ServerStatus::OnPath(p) => {
                                    (Text, format!("on PATH  {}", p.display()))
                                }
                                ServerStatus::Installing => (Warn, "installing…".into()),
                                ServerStatus::Failed(e) => (Bad, format!("install failed: {e}")),
                                ServerStatus::Missing => match &s.install {
                                    Some(i) => (
                                        Warn,
                                        format!(
                                            "not installed — press i to install ({})",
                                            i.source()
                                        ),
                                    ),
                                    None => (
                                        Bad,
                                        format!("not found — install `{}` yourself", s.command),
                                    ),
                                },
                            }
                        };
                        out.push((tone, row(name, what)));
                    }
                }
            }
        }
        if let Some(c) = b.lsp_client.and_then(|c| self.lsp.clients.get(&c)) {
            out.push((Text, String::new()));
            out.push((Heading, format!("Attached server: {}", c.server)));
            let state = if !c.alive {
                (Bad, "exited".to_string())
            } else if !c.ready {
                (Warn, "starting…".to_string())
            } else {
                (Good, "ready".to_string())
            };
            let up = c.started.elapsed().as_secs();
            out.push((
                state.0,
                row(
                    "status",
                    format!(
                        "{}  · pid {} · up {}m{:02}s",
                        state.1,
                        c.pid,
                        up / 60,
                        up % 60
                    ),
                ),
            ));
            out.push((Text, row("command", c.path.display().to_string())));
            out.push((Text, row("root", c.root.display().to_string())));
            if let Some(p) = &c.progress {
                out.push((Warn, row("progress", p.clone())));
            }
            out.push((
                Text,
                row(
                    "sync",
                    if c.sync_kind == 2 {
                        "incremental".into()
                    } else {
                        "full document".into()
                    },
                ),
            ));
            let caps: Vec<&str> = [
                ("completionProvider", "completion"),
                ("hoverProvider", "hover"),
                ("definitionProvider", "definition"),
                ("referencesProvider", "references"),
                ("renameProvider", "rename"),
                ("documentFormattingProvider", "formatting"),
                ("documentSymbolProvider", "symbols"),
                ("workspaceSymbolProvider", "project symbols"),
                ("codeActionProvider", "code actions"),
                ("signatureHelpProvider", "signature help"),
            ]
            .iter()
            .filter(|(k, _)| c.has_cap(k))
            .map(|(_, v)| *v)
            .collect();
            out.push((
                Text,
                row(
                    "capabilities",
                    if caps.is_empty() {
                        "(waiting for initialize)".into()
                    } else {
                        caps.join(", ")
                    },
                ),
            ));
            if let Some(ds) = b.path.as_ref().and_then(|p| self.diagnostics.get(p)) {
                let e = ds.iter().filter(|d| d.severity == 1).count();
                let w = ds.iter().filter(|d| d.severity == 2).count();
                let pl = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
                out.push((
                    Text,
                    row(
                        "diagnostics",
                        format!(
                            "{}, {}, {} other in this file",
                            pl(e, "error"),
                            pl(w, "warning"),
                            ds.len() - e - w
                        ),
                    ),
                ));
            }
            let err = c.stderr.lock().unwrap();
            if !err.is_empty() {
                out.push((Text, String::new()));
                out.push((Heading, "Server output (stderr, newest last)".into()));
                for l in err
                    .iter()
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                {
                    out.push((Dim, format!("  {}", l.replace('\t', "    "))));
                }
            }
        }
        self.lsp_info_running(out)
    }

    fn lsp_info_running(&self, mut out: Vec<(Tone, String)>) -> Vec<(Tone, String)> {
        out.push((Tone::Text, String::new()));
        out.push((Tone::Heading, "All running servers".into()));
        let mut cs: Vec<_> = self.lsp.clients.iter().collect();
        cs.sort_by_key(|(id, _)| **id);
        if cs.is_empty() {
            out.push((Tone::Dim, "  none".into()));
        }
        for (id, c) in cs {
            let files = self
                .buffers
                .iter()
                .filter(|b| b.lsp_client == Some(*id))
                .count();
            let state = if !c.alive {
                "exited"
            } else if !c.ready {
                "starting"
            } else {
                "ready"
            };
            let tone = if c.alive { Tone::Text } else { Tone::Bad };
            out.push((
                tone,
                format!(
                    "  {:<14}{:<9}{} file(s)   {}",
                    c.server,
                    state,
                    files,
                    c.root.display()
                ),
            ));
        }
        out
    }

    /// The first configured server for this file that isn't installed but could be.
    fn installable_for_current(&self) -> Option<String> {
        let lang = self
            .buf()?
            .lang
            .as_ref()
            .and_then(|l| self.config.language(l))?;
        lang.servers
            .iter()
            .find(|n| {
                self.config.server(n).is_some_and(|s| {
                    s.install.as_ref().is_some_and(|i| i.supported_here())
                        && matches!(
                            self.server_status(s),
                            ServerStatus::Missing | ServerStatus::Failed(_)
                        )
                })
            })
            .cloned()
    }

    fn lsp_info_key(&mut self, k: Key, mut v: LspInfoView) {
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Up => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::Down => v.scroll += 1,
            KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(15),
            KeyCode::PageDown => v.scroll += 15,
            KeyCode::Home => v.scroll = 0,
            _ => {}
        }
        self.modal = Some(Modal::LspInfo(v));
    }

    fn lsp_info_char(&mut self, c: char) {
        match c {
            'i' => match self.installable_for_current() {
                Some(n) => self.install_server(&n),
                None => self.msg("Nothing to install for this file"),
            },
            'r' => {
                self.restart_lsp();
                self.msg("Language servers restarted");
            }
            's' => self.run(Command::ManageServers),
            'q' => self.modal = None,
            _ => {}
        }
    }

    // ===================================================================== bottom panel

    pub fn console_shown(&self) -> bool {
        self.panel.visible && self.panel.tab == PanelTab::Console
    }

    /// The terminal shown in the panel, if the panel is showing one.
    pub fn terminal_shown(&self) -> Option<usize> {
        match self.panel.tab {
            PanelTab::Terminal(id)
                if self.panel.visible && self.terminals.iter().any(|t| t.id == id) =>
            {
                Some(id)
            }
            _ => None,
        }
    }

    pub fn current_terminal_mut(&mut self) -> Option<&mut crate::terminal::Terminal> {
        let id = self.terminal_shown()?;
        self.terminals.iter_mut().find(|t| t.id == id)
    }

    pub fn show_terminal(&mut self, id: usize) {
        self.panel.visible = true;
        self.panel.tab = PanelTab::Terminal(id);
        self.focus = Focus::Terminal;
        self.completion = None;
        self.hover = None;
    }

    pub fn new_terminal(&mut self) {
        let cwd = self.base_dir();
        self.new_terminal_in(&cwd);
    }

    pub fn new_terminal_in(&mut self, cwd: &Path) {
        let id = self.next_term_id;
        self.next_term_id += 1;
        let cwd = cwd.to_path_buf();
        match crate::terminal::Terminal::spawn(
            id,
            &cwd,
            24,
            80,
            self.config.editor.terminal_shell.as_deref(),
            self.tx.clone(),
        ) {
            Ok(t) => {
                self.terminals.push(t);
                self.show_terminal(id);
            }
            Err(e) => {
                crate::log::error("terminal", &e);
                self.msg(format!("Couldn't start a terminal: {e}"));
            }
        }
    }

    /// Close terminals whose shell exited cleanly (like VS Code); failed ones stay open
    /// showing their exit code until a key is pressed.
    pub fn reap_terminals(&mut self) {
        let done: Vec<(usize, Option<u32>)> = self
            .terminals
            .iter()
            .filter(|t| t.is_exited())
            .map(|t| (t.id, *t.exit_code.lock().unwrap()))
            .collect();
        for (id, code) in done {
            if code == Some(0) {
                crate::log::info("terminal", "shell exited");
                self.close_terminal(id);
            }
        }
    }

    pub fn close_terminal(&mut self, id: usize) {
        let Some(i) = self.terminals.iter().position(|t| t.id == id) else {
            return;
        };
        self.terminals.remove(i);
        let next = self
            .terminals
            .get(i.saturating_sub(1))
            .or(self.terminals.first())
            .map(|t| t.id);
        if self.panel.tab == PanelTab::Terminal(id) {
            match next {
                Some(n) => self.panel.tab = PanelTab::Terminal(n),
                None => {
                    self.panel.visible = false;
                    self.panel.tab = PanelTab::Console;
                    if self.focus == Focus::Terminal {
                        self.focus = Focus::Editor;
                    }
                }
            }
        }
    }

    /// Keys while a terminal has focus go to the shell, except a few app shortcuts.
    fn terminal_key(&mut self, k: Key) -> bool {
        use Command::*;
        let Some(id) = self.terminal_shown() else {
            return false;
        };
        let (exited, rows, app_cursor) = self
            .terminals
            .iter()
            .find(|t| t.id == id)
            .map(|t| (t.is_exited(), t.size.0 as isize, t.application_cursor()))
            .unwrap_or((false, 10, false));
        if exited && !k.ctrl && !k.sup {
            // "Press any key to close", like VS Code.
            self.close_terminal(id);
            return true;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab cycle terminal tabs while a terminal has focus.
        if k.code == KeyCode::Tab && k.ctrl && !k.alt {
            let n = self.terminals.len();
            if let Some(i) = self.terminals.iter().position(|t| t.id == id)
                && n > 1
            {
                let next = if k.shift {
                    (i + n - 1) % n
                } else {
                    (i + 1) % n
                };
                let nid = self.terminals[next].id;
                self.show_terminal(nid);
            }
            return true;
        }
        let primary = |c: char| {
            k.code == KeyCode::Char(c) && ((k.sup && !k.shift) || (k.ctrl && k.shift && !k.sup))
        };
        if primary('c') {
            if let Some(t) = self.current_terminal_mut()
                && let Some(text) = t.selection_text()
            {
                t.sel = None;
                self.pending_copy = Some(text);
            }
            return true;
        }
        if primary('v') {
            let text = self.clipboard_text();
            if let Some(t) = self.current_terminal_mut() {
                t.paste(&text);
            }
            return true;
        }
        const ALWAYS: [Command; 6] = [
            ToggleTerminal,
            NewTerminal,
            ShowCommands,
            GotoFile,
            FocusSidebar,
            ToggleConsole,
        ];
        if let Some(cmd) = self.keymap.lookup(k, Ctx::Global)
            && (k.sup || ALWAYS.contains(&cmd))
        {
            self.run(cmd);
            return true;
        }
        if k.sup {
            return true; // other Cmd shortcuts never reach the shell
        }
        if k.shift && matches!(k.code, KeyCode::PageUp | KeyCode::PageDown) {
            if let Some(t) = self.current_terminal_mut() {
                t.scroll(if k.code == KeyCode::PageUp {
                    rows - 1
                } else {
                    1 - rows
                });
            }
            return true;
        }
        if let Some(bytes) = crate::terminal::key_bytes(k, app_cursor)
            && let Some(t) = self.current_terminal_mut()
        {
            t.sel = None;
            t.write(&bytes);
            return true;
        }
        // Plain text keys arrive as text events.
        false
    }

    // ===================================================================== context menus

    /// Open the right-click menu for a place, at a screen position (or the keyboard spot).
    pub fn open_context_menu(&mut self, kind: menu::ContextKind, pos: (f32, f32)) {
        self.open_context_menu_at(kind, Some(pos));
    }

    pub fn open_context_menu_at(&mut self, kind: menu::ContextKind, pos: Option<(f32, f32)>) {
        let items = self.contexts.get(kind.name()).cloned().unwrap_or_default();
        if items.is_empty() {
            return;
        }
        self.completion = None;
        self.hover = None;
        self.menu = None;
        let sel = pos
            .is_none()
            .then(|| {
                items
                    .iter()
                    .position(|i| i.as_ref().is_some_and(|i| self.enabled(i.cmd)))
            })
            .flatten();
        self.context_menu = Some(menu::ContextMenu {
            kind,
            items,
            pos,
            sel,
        });
    }

    pub fn context_activate(&mut self, i: usize) {
        let Some(cm) = self.context_menu.take() else {
            return;
        };
        if let Some(Some(it)) = cm.items.get(i)
            && self.enabled(it.cmd)
        {
            self.run(it.cmd);
        }
    }

    fn context_key(&mut self, k: Key) {
        let Some(cm) = &self.context_menu else { return };
        let n = cm.items.len();
        let enabled: Vec<bool> = cm
            .items
            .iter()
            .map(|i| i.as_ref().is_some_and(|i| self.enabled(i.cmd)))
            .collect();
        let step = |from: Option<usize>, d: isize| -> Option<usize> {
            let mut i = from
                .map(|x| x as isize)
                .unwrap_or(if d > 0 { -1 } else { n as isize });
            for _ in 0..n {
                i = (i + d).rem_euclid(n as isize);
                if enabled[i as usize] {
                    return Some(i as usize);
                }
            }
            from
        };
        let sel = cm.sel;
        match k.code {
            KeyCode::Esc => self.context_menu = None,
            KeyCode::Up => self.context_menu.as_mut().unwrap().sel = step(sel, -1),
            KeyCode::Down => self.context_menu.as_mut().unwrap().sel = step(sel, 1),
            KeyCode::Enter => {
                if let Some(i) = sel {
                    self.context_activate(i);
                }
            }
            _ => {}
        }
    }

    fn context_char(&mut self, c: char) {
        let Some(cm) = &self.context_menu else { return };
        let c = c.to_ascii_lowercase();
        let hit = cm.items.iter().position(|i| {
            i.as_ref()
                .is_some_and(|i| self.enabled(i.cmd) && i.label.to_lowercase().starts_with(c))
        });
        if let Some(i) = hit {
            self.context_activate(i);
        }
    }

    /// The file a context command acts on: the sidebar selection, else the current file.
    fn context_path(&self) -> Option<PathBuf> {
        if self.focus == Focus::Sidebar
            && let Some(p) = self.tree.selected_path()
        {
            return Some(p.to_path_buf());
        }
        self.buf().and_then(|b| b.path.clone())
    }

    // ===================================================================== mouse hover

    pub fn diagnostics_at(&self, p: Pos) -> Vec<(u8, String)> {
        let Some(path) = self.buf().and_then(|b| b.path.as_ref()) else {
            return vec![];
        };
        let mut v: Vec<(u8, String)> = self
            .diagnostics
            .get(path)
            .map(|ds| {
                ds.iter()
                    .filter(|d| d.start <= p && (p < d.end || (d.start == d.end && p == d.start)))
                    .map(|d| (d.severity, d.message.clone()))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Called by the GUI every frame with the buffer position under the pointer (or
    /// `None`). After the pointer rests on a word for `HOVER_DELAY`, asks the language
    /// server for hover info at that spot (diagnostics there are shown too).
    pub fn mouse_hover(&mut self, at: Option<Pos>, over_popup: bool) -> Option<Duration> {
        const HOVER_DELAY: Duration = Duration::from_millis(500);
        let blocked = self.modal.is_some()
            || self.completion.is_some()
            || self.context_menu.is_some()
            || self.menu.is_some();
        let Some(at) = at.filter(|_| !blocked) else {
            if !over_popup {
                self.hover_track = None;
                if self.hover.as_ref().is_some_and(|h| h.anchor.is_some()) {
                    self.hover = None;
                }
            }
            return None;
        };
        let Some(b) = self.buf() else { return None };
        let word = b
            .word_bounds(at)
            .unwrap_or((at, Pos::new(at.line, at.col + 1)));
        let id = b.id;
        let same = self
            .hover_track
            .as_ref()
            .is_some_and(|t| t.buf == id && t.word == word);
        if !same {
            if self.hover.as_ref().is_some_and(|h| h.anchor.is_some()) && !over_popup {
                self.hover = None;
            }
            self.hover_track = Some(HoverTrack {
                buf: id,
                word,
                pos: at,
                since: Instant::now(),
                requested: false,
            });
            return Some(HOVER_DELAY);
        }
        let t = self.hover_track.as_mut().unwrap();
        if t.requested {
            return None;
        }
        let left = HOVER_DELAY.saturating_sub(t.since.elapsed());
        if !left.is_zero() {
            return Some(left);
        }
        t.requested = true;
        let pos = t.pos;
        let sent = self.lsp_request_at(
            "textDocument/hover",
            json!({}),
            Pending::Hover {
                buf: id,
                pos,
                mouse: true,
            },
            "hoverProvider",
            pos,
            false,
        );
        if !sent {
            let diags = self.diagnostics_at(pos);
            if !diags.is_empty() {
                self.hover = Some(Hover {
                    lines: vec![],
                    diagnostics: diags,
                    anchor: Some(pos),
                });
            }
        }
        None
    }

    // ===================================================================== console selection

    /// The rendered text of a log line in the console.
    pub fn console_line(e: &crate::log::Entry) -> String {
        format!(
            "{} [{}] {}",
            &e.time[..8.min(e.time.len())],
            e.source,
            e.text
        )
    }

    pub fn console_entries(&self) -> Vec<crate::log::Entry> {
        let min = if self.console.show_debug {
            crate::log::Level::Debug
        } else {
            crate::log::Level::Info
        };
        crate::log::entries(min, &self.console.filter)
    }

    pub fn console_select_all(&mut self) {
        let es = self.console_entries();
        if let (Some(a), Some(b)) = (es.first(), es.last()) {
            self.console.sel = Some(((a.id, 0), (b.id, Self::console_line(b).chars().count())));
        }
    }

    pub fn console_selection_text(&self) -> Option<String> {
        let (a, b) = self.console.sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = vec![];
        for e in self
            .console_entries()
            .iter()
            .filter(|e| e.id >= a.0 && e.id <= b.0)
        {
            let line: Vec<char> = Self::console_line(e).chars().collect();
            let s = if e.id == a.0 { a.1.min(line.len()) } else { 0 };
            let t = if e.id == b.0 {
                b.1.min(line.len())
            } else {
                line.len()
            };
            out.push(line[s..t.max(s)].iter().collect::<String>());
        }
        let text = out.join("\n");
        (!text.is_empty()).then_some(text)
    }

    // ===================================================================== console

    fn console_key(&mut self, k: Key) {
        let c = &mut self.console;
        match k.code {
            KeyCode::Esc => {
                if c.sel.take().is_none() {
                    self.panel.visible = false;
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut c.input);
                c.cursor = 0;
                c.hist_pos = None;
                if !line.trim().is_empty() && c.history.last() != Some(&line) {
                    c.history.push(line.clone());
                }
                self.console_exec(line.trim());
            }
            KeyCode::Backspace if c.cursor > 0 => {
                let b = crate::editor::byte_idx(&c.input, c.cursor - 1);
                c.input.remove(b);
                c.cursor -= 1;
            }
            KeyCode::Delete if c.cursor < char_len(&c.input) => {
                let b = crate::editor::byte_idx(&c.input, c.cursor);
                c.input.remove(b);
            }
            KeyCode::Left => c.cursor = c.cursor.saturating_sub(1),
            KeyCode::Right => c.cursor = (c.cursor + 1).min(char_len(&c.input)),
            KeyCode::Home => c.cursor = 0,
            KeyCode::End => c.cursor = char_len(&c.input),
            KeyCode::Up | KeyCode::Down if !c.history.is_empty() => {
                let n = c.history.len();
                let pos = match (c.hist_pos, k.code) {
                    (None, KeyCode::Up) => Some(n - 1),
                    (Some(p), KeyCode::Up) => Some(p.saturating_sub(1)),
                    (Some(p), _) if p + 1 < n => Some(p + 1),
                    _ => None,
                };
                c.hist_pos = pos;
                c.input = pos.map(|p| c.history[p].clone()).unwrap_or_default();
                c.cursor = char_len(&c.input);
            }
            KeyCode::PageUp => c.scroll += 10,
            KeyCode::PageDown => c.scroll = c.scroll.saturating_sub(10),
            KeyCode::Char('u') if k.ctrl => {
                c.input.clear();
                c.cursor = 0;
            }
            KeyCode::Char('l') if k.ctrl => crate::log::clear(),
            _ => {}
        }
    }

    /// Run a console line: a few built-ins, otherwise a command id (like Sublime's
    /// `window.run_command`).
    pub fn console_exec(&mut self, line: &str) {
        use crate::log;
        if line.is_empty() {
            return;
        }
        log::info("console", format!("> {line}"));
        let (word, rest) = line
            .split_once(' ')
            .map(|(a, b)| (a, b.trim()))
            .unwrap_or((line, ""));
        match word {
            "help" => log::info(
                "console",
                "help · clear · debug (toggle server output) · filter TEXT · wrap (toggle line wrap) · log (open log file) · lsp (LSP info) · \
                 <command id> runs a command, e.g. restart_lsp, goto_file, toggle_sidebar (see Help › Keyboard Shortcuts)",
            ),
            "clear" => log::clear(),
            "wrap" => {
                self.console.wrap = !self.console.wrap;
                self.console.hscroll = 0.0;
                log::info(
                    "console",
                    format!(
                        "line wrapping {}",
                        if self.console.wrap { "on" } else { "off" }
                    ),
                );
            }
            "debug" => {
                self.console.show_debug = !self.console.show_debug;
                log::info(
                    "console",
                    format!(
                        "debug output {}",
                        if self.console.show_debug {
                            "shown"
                        } else {
                            "hidden"
                        }
                    ),
                );
            }
            "filter" => {
                self.console.filter = rest.to_string();
                log::info(
                    "console",
                    if rest.is_empty() {
                        "filter cleared".into()
                    } else {
                        format!("showing lines matching \"{rest}\"")
                    },
                );
            }
            "log" => match self.config.data_dir.clone() {
                Some(d) => {
                    let f = d.join("logs").join("farsub.log");
                    self.open_file(&f, true);
                }
                None => log::warn("console", "no data folder"),
            },
            "lsp" => {
                for (_, l) in self.lsp_info() {
                    log::info("console", if l.is_empty() { " ".into() } else { l });
                }
            }
            id => match crate::commands::from_id(id) {
                Some(cmd) => {
                    self.focus = Focus::Editor;
                    self.run(cmd);
                    if self.focus == Focus::Editor && self.modal.is_none() {
                        self.focus = Focus::Console;
                    }
                }
                None => log::error("console", format!("unknown command `{id}` (type help)")),
            },
        }
    }

    // ===================================================================== menu & keymap

    /// Whether a command can do anything right now (menus grey out the rest).
    pub fn enabled(&self, cmd: Command) -> bool {
        use Command::*;
        let has_buf = self.buf().is_some();
        let lsp = self.buf().is_some_and(|b| b.lsp_client.is_some());
        let in_panel = matches!(self.focus, Focus::Terminal | Focus::Console);
        match cmd {
            Copy | Paste | SelectAll if in_panel => true,
            CloseOtherTabs => has_buf && self.group_tabs(self.group).len() > 1,
            CloseTabsToRight => {
                has_buf
                    && self
                        .group_tabs(self.group)
                        .last()
                        .is_some_and(|&l| l > self.cur)
            }
            FocusGroup2 if self.layout < 2 => false,
            FocusGroup3 if self.layout < 3 => false,
            FocusGroup4 if self.layout < 4 => false,
            MoveToGroup1 | MoveToGroup2 | MoveToGroup3 | MoveToGroup4 => has_buf,
            FocusNextGroup => self.layout > 1,
            CopyFilePath | RevealInFileManager => {
                (self.focus == Focus::Sidebar && self.tree.selected_path().is_some())
                    || self.buf().is_some_and(|b| b.path.is_some())
            }
            ClearTerminal => self.terminal_shown().is_some(),
            Copy | Paste | SelectAll => has_buf,
            SwitchBuffer => !self.buffers.is_empty(),
            c if needs_buffer(c) => has_buf,
            Completion | Hover | GotoDefinition | FindReferences | Rename | Format => lsp,
            SaveAll => self.any_modified(),
            ReopenClosed => !self.closed.is_empty(),
            JumpBack => !self.jumps.is_empty(),
            SidebarPaste => self.file_clip.is_some(),
            RemovePackage => !self.packages.is_empty(),
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
        let cmd = self
            .menus
            .get(menu)
            .and_then(|m| m.items.get(item))
            .and_then(|i| i.as_ref())
            .map(|i| i.cmd);
        if let Some(cmd) = cmd {
            if self.enabled(cmd) {
                self.menu = None;
                self.run(cmd);
            }
        }
    }

    fn menu_key(&mut self, k: Key) {
        let Some(mut m) = self.menu else { return };
        let n_menus = self.menus.len().max(1);
        let items: Vec<Option<Command>> = self
            .menus
            .get(m.menu)
            .map(|mm| mm.items.iter().map(|i| i.as_ref().map(|i| i.cmd)).collect())
            .unwrap_or_default();
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
        let Some(menu) = self.menus.get(m.menu) else {
            return;
        };
        let hits: Vec<usize> = menu
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.as_ref()
                    .is_some_and(|i| self.enabled(i.cmd) && i.label.to_lowercase().starts_with(c))
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
        let n = keymap_rows(&self.keymap, &v.query).len();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                if let Some(cmd) = keymap_rows(&self.keymap, &v.query)
                    .get(v.sel)
                    .and_then(|r| r.cmd)
                {
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
        // Editing commands need a file in the focused group (it may be an empty column).
        if needs_buffer(cmd) && self.buf().is_none() {
            return;
        }
        match cmd {
            LayoutSingle => self.set_layout(1),
            LayoutColumns2 => self.set_layout(2),
            LayoutColumns3 => self.set_layout(3),
            LayoutColumns4 => self.set_layout(4),
            FocusGroup1 => self.focus_group(0),
            FocusGroup2 => self.focus_group(1),
            FocusGroup3 => self.focus_group(2),
            FocusGroup4 => self.focus_group(3),
            MoveToGroup1 => self.move_to_group(0),
            MoveToGroup2 => self.move_to_group(1),
            MoveToGroup3 => self.move_to_group(2),
            MoveToGroup4 => self.move_to_group(3),
            FocusNextGroup => self.focus_group((self.group + 1) % self.layout),
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
                let i = self.push_buffer(b);
                self.set_cur(i);
                self.focus = Focus::Editor;
            }
            OpenFolder => {
                let def = format!("{}/", self.base_dir().display());
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
                if self.config.editor.hot_exit {
                    // Unsaved changes are kept in the workspace and restored next time.
                    self.save_workspace();
                    self.quit = true;
                } else if n > 0 {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!("{n} file(s) have unsaved changes. Quit anyway?"),
                        kind: ConfirmKind::Quit,
                    }));
                } else {
                    self.quit = true;
                }
            }
            NextBuffer | PrevBuffer => {
                if self.buf().is_some() {
                    self.cycle_tab(if cmd == NextBuffer { 1 } else { -1 });
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
                self.tree
                    .set_roots(self.project.folders.clone(), self.show_hidden);
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
            OpenProject => {
                let def = format!("{}/", self.base_dir().display());
                self.prompt(
                    "Open project (.farsub-project) or folder  (Tab completes)",
                    &def,
                    PromptKind::OpenProject,
                );
            }
            SwitchProject => {
                let mut p = Palette::new(Mode::Projects, "");
                p.title = "Recent projects and folders".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            SaveProjectAs => {
                let base = self.base_dir();
                let name = self
                    .project
                    .file
                    .as_ref()
                    .map(|f| f.display().to_string())
                    .unwrap_or_else(|| {
                        let stem = base
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        base.join(format!("{stem}.{}", project::PROJECT_EXT))
                            .display()
                            .to_string()
                    });
                self.prompt("Save project as", &name, PromptKind::SaveProjectAs);
            }
            CloseProject => self.switch_project(Project::default()),
            AddFolder => {
                let def = format!(
                    "{}/",
                    self.base_dir().parent().unwrap_or(Path::new("/")).display()
                );
                self.prompt(
                    "Add folder to project  (Tab completes)",
                    &def,
                    PromptKind::AddFolder,
                );
            }
            RemoveFolder => {
                let target = self
                    .tree
                    .selected_path()
                    .and_then(|p| self.tree.root_of(p))
                    .map(|f| f.path.clone())
                    .or_else(|| {
                        (self.project.folders.len() == 1)
                            .then(|| self.project.folders[0].path.clone())
                    });
                match target {
                    Some(p) if self.project.remove_folder(&p) => {
                        self.project_folders_changed();
                        self.msg(format!("Removed {} from the project", p.display()));
                    }
                    _ => self.msg("Select a project folder in the sidebar first"),
                }
            }
            EditProject => match self.project.file.clone() {
                Some(f) => {
                    self.open_file(&f, true);
                }
                None => {
                    self.msg("This window has no project file yet — Project › Save Project As…")
                }
            },
            Indentation => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                let mut p = Palette::new(Mode::Indentation, "");
                p.title = "Indentation for this file".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            SetSyntax => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                let mut p = Palette::new(Mode::Syntaxes, "");
                p.title = "Set syntax for this file".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
                let cur = self.buf().map(|b| b.hl.syntax.clone()).unwrap_or_default();
                if let Some(Modal::Palette(p)) = &mut self.modal
                    && let Some(i) = p.shown.iter().position(|(i, _)| p.items[*i].label == cur)
                {
                    p.sel = i;
                }
            }
            OpenContextMenu => {
                let kind = match self.focus {
                    Focus::Sidebar if self.sidebar_visible => menu::ContextKind::Sidebar,
                    Focus::Terminal => menu::ContextKind::Terminal,
                    Focus::Console => menu::ContextKind::Console,
                    _ => menu::ContextKind::Editor,
                };
                self.open_context_menu_at(kind, None);
            }
            Copy | Cut | Paste | SelectAll
                if matches!(self.focus, Focus::Terminal | Focus::Console) =>
            {
                match cmd {
                    Copy => self.on_copy(false),
                    Cut => self.on_copy(true),
                    Paste => {
                        let t = self.clipboard_text();
                        if self.focus == Focus::Terminal {
                            if let Some(term) = self.current_terminal_mut() {
                                term.paste(&t);
                            }
                        } else {
                            self.on_paste(&t);
                        }
                    }
                    _ => {
                        if self.focus == Focus::Console {
                            self.console_select_all();
                        }
                    }
                }
            }
            CopyFilePath => {
                let p = self.context_path();
                if let Some(p) = p {
                    self.msg(format!("Copied {}", p.display()));
                    self.pending_copy = Some(p.display().to_string());
                }
            }
            RevealInFileManager => {
                if let Some(p) = self.context_path() {
                    reveal_in_file_manager(&p);
                }
            }
            OpenTerminalHere => {
                let dir = if self.focus == Focus::Sidebar {
                    self.tree.target_dir()
                } else {
                    self.buf()
                        .and_then(|b| b.path.as_ref())
                        .and_then(|p| p.parent())
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.base_dir())
                };
                self.new_terminal_in(&dir);
            }
            CloseOtherTabs | CloseTabsToRight => {
                let Some(keep_id) = self.buf().map(|b| b.id) else {
                    return;
                };
                let keep = self.cur;
                let ids: Vec<usize> = self
                    .group_tabs(self.group)
                    .into_iter()
                    .filter(|&i| {
                        if cmd == CloseOtherTabs {
                            i != keep
                        } else {
                            i > keep
                        }
                    })
                    .map(|i| self.buffers[i].id)
                    .collect();
                let mut skipped = 0;
                for id in ids {
                    let Some(i) = self.buf_index(id) else {
                        continue;
                    };
                    if self.buffers[i].modified && !self.config.editor.hot_exit {
                        skipped += 1;
                        continue;
                    }
                    if self.buffers[i].modified {
                        // Hot exit would lose these on close; keep them open instead.
                        skipped += 1;
                        continue;
                    }
                    self.close_buffer_at(i);
                }
                if let Some(i) = self.buf_index(keep_id) {
                    self.set_cur(i);
                }
                if skipped > 0 {
                    self.msg(format!("Kept {skipped} tab(s) with unsaved changes"));
                }
            }
            ClearTerminal => {
                if let Some(t) = self.current_terminal_mut() {
                    t.write(b"\x0c");
                }
            }
            ConsoleClear => crate::log::clear(),
            ConsoleToggleWrap => {
                self.console.wrap = !self.console.wrap;
                self.console.hscroll = 0.0;
            }
            ToggleConsole => {
                if self.console_shown() && self.focus == Focus::Console {
                    self.panel.visible = false;
                    self.focus = Focus::Editor;
                } else {
                    self.panel.visible = true;
                    self.panel.tab = PanelTab::Console;
                    self.console.scroll = 0;
                    self.focus = Focus::Console;
                }
            }
            ToggleTerminal => {
                let showing = self.panel.visible && matches!(self.panel.tab, PanelTab::Terminal(_));
                if showing && self.focus == Focus::Terminal {
                    self.panel.visible = false;
                    self.focus = Focus::Editor;
                } else {
                    let id = match self.panel.tab {
                        PanelTab::Terminal(id) if self.terminals.iter().any(|t| t.id == id) => {
                            Some(id)
                        }
                        _ => self.terminals.last().map(|t| t.id),
                    };
                    match id {
                        Some(id) => self.show_terminal(id),
                        None => self.new_terminal(),
                    }
                }
            }
            NewTerminal => self.new_terminal(),
            CloseTerminal => {
                if let PanelTab::Terminal(id) = self.panel.tab {
                    self.close_terminal(id);
                }
            }
            LspInfo => {
                self.completion = None;
                self.hover = None;
                self.modal = Some(Modal::LspInfo(LspInfoView { scroll: 0 }));
            }
            ManageServers => {
                self.completion = None;
                self.hover = None;
                self.modal = Some(Modal::Servers(ServersView {
                    sel: 0,
                    details: false,
                }));
            }
            InstallPackage => self.prompt(
                "Install package from git URL (Sublime packages with .sublime-syntax / .tmTheme)",
                "https://github.com/",
                PromptKind::PackageUrl,
            ),
            RemovePackage => {
                if self.packages.is_empty() {
                    return self.msg("No packages installed");
                }
                let mut p = Palette::new(Mode::Packages, "");
                p.title = "Remove package".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            BrowsePackages => {
                if let Some(d) = self.config.packages_dir() {
                    packages::reveal(&d);
                }
            }
            ReloadPackages => {
                // Icon packages live in the config (icons.toml files).
                self.reload_config();
                let dir = self.config.packages_dir();
                self.packages = dir.as_deref().map(packages::list).unwrap_or_default();
                packages::load_async(dir, self.tx.clone());
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
                let (cur, group) = (self.cur, self.group);
                for i in 0..self.buffers.len() {
                    if self.buffers[i].modified && self.buffers[i].path.is_some() {
                        self.set_cur(i);
                        self.save_current();
                    }
                }
                self.cur = cur;
                self.group = group;
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
        if matches!(k.code, KeyCode::Char(_))
            && !k.ctrl
            && self.keymap.lookup(k, Ctx::Editor).is_none()
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
            Paste => {
                let t = self.clipboard_text();
                return self.paste_text(t);
            }
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
                    Pending::Hover {
                        buf: id,
                        pos,
                        mouse: false,
                    },
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
            c.sel = Some(match c.sel {
                None if d > 0 => 0,
                None => (n - 1) as usize,
                Some(s) if d.abs() == 1 => (s as isize + d).rem_euclid(n) as usize,
                Some(s) => (s as isize + d).clamp(0, n - 1) as usize,
            });
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
        c.sel = None;
        if c.shown.is_empty() {
            self.completion = None;
        }
    }

    fn comp_accept(&mut self) {
        let Some(c) = self.completion.take() else {
            return;
        };
        let Some(item) = c
            .sel
            .and_then(|s| c.shown.get(s))
            .and_then(|&i| c.items.get(i))
            .cloned()
        else {
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

    /// The first configured server for a language that is installed or on PATH.
    fn pick_server(
        &self,
        lang: &crate::config::LanguageConfig,
    ) -> Option<(crate::config::ServerConfig, PathBuf)> {
        let data = self.config.data_dir.as_deref();
        lang.servers
            .iter()
            .filter_map(|n| self.config.server(n))
            .find_map(|s| installer::resolve(data, s).map(|bin| (s.clone(), bin)))
    }

    fn lsp_open(&mut self, i: usize) {
        let b = &self.buffers[i];
        let (Some(path), Some(lang)) = (b.path.clone(), b.lang.clone()) else {
            return;
        };
        let Some(cfg) = self.config.language(&lang).cloned() else {
            return;
        };
        if cfg.servers.is_empty() {
            return;
        }
        let Some((server, bin)) = self.pick_server(&cfg) else {
            if self.lsp_warned.insert(lang.clone()) {
                let installable: Vec<String> = cfg
                    .servers
                    .iter()
                    .filter(|n| {
                        self.config
                            .server(n)
                            .is_some_and(|s| s.install.as_ref().is_some_and(|i| i.supported_here()))
                    })
                    .cloned()
                    .collect();
                if installable.is_empty() {
                    let cmds: Vec<String> = cfg
                        .servers
                        .iter()
                        .filter_map(|n| self.config.server(n))
                        .map(|s| s.command.clone())
                        .collect();
                    self.msg(format!(
                        "No {lang} language server found (install {})",
                        cmds.join(" or ")
                    ));
                } else {
                    self.msg(format!(
                        "No {lang} language server installed — Tools › Language Servers can install {}",
                        installable.join(", ")
                    ));
                }
            }
            return;
        };
        // Managed installs expose their folder as ${install_dir} to the server config.
        let install_dir = self
            .config
            .data_dir
            .as_deref()
            .filter(|d| installer::installed_bin(d, &server).as_ref() == Some(&bin))
            .map(|d| installer::server_dir(d, &server.name));
        match self
            .lsp
            .client_for(&cfg, &server, &bin, install_dir.as_deref(), &path)
        {
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
        let Some(at) = self.buf().map(|b| b.cursor) else {
            return false;
        };
        self.lsp_request_at(method, extra, p, cap, at, true)
    }

    /// Like `lsp_position_request` at an explicit position; `loud` reports missing servers.
    fn lsp_request_at(
        &mut self,
        method: &str,
        extra: Value,
        p: Pending,
        cap: &str,
        at: Pos,
        loud: bool,
    ) -> bool {
        self.sync_lsp();
        let Some(b) = self.buf() else { return false };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            if loud {
                self.msg("No language server for this file");
            }
            return false;
        };
        let at = b.clamp(at);
        let pos = to_lsp(&b.lines[at.line], at);
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return false;
        };
        if c.ready && !c.has_cap(cap) {
            if loud {
                let name = c.cmd.clone();
                self.msg(format!("{name} does not support {method}"));
            }
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
                    if matches!(pending, Pending::Initialize) {
                        // The server refused to start: mark it failed instead of "starting…" forever.
                        let m = m.to_string();
                        if let Some(c) = self.lsp.clients.get_mut(&cid) {
                            c.init_error = Some(m.clone());
                            let name = c.server.clone();
                            c.shutdown();
                            crate::log::error(
                                &format!("lsp:{name}"),
                                format!("initialize failed: {m}"),
                            );
                            self.msg(format!("{name} failed to start: {m}"));
                        }
                        return;
                    }
                    crate::log::warn("lsp", format!("request failed: {m}"));
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
            "window/showMessage" | "window/logMessage" => {
                if let Some(m) = params.get("message").and_then(|m| m.as_str()) {
                    let t = params.get("type").and_then(|t| t.as_i64()).unwrap_or(3);
                    let src = format!(
                        "lsp:{}",
                        self.lsp
                            .clients
                            .get(&cid)
                            .map(|c| c.server.as_str())
                            .unwrap_or("?")
                    );
                    let level = match t {
                        1 => crate::log::Level::Error,
                        2 => crate::log::Level::Warn,
                        3 => crate::log::Level::Info,
                        _ => crate::log::Level::Debug,
                    };
                    crate::log::push(level, &src, m);
                    if method == "window/showMessage" && t <= 2 {
                        self.status = Some((format!("LSP: {m}"), Instant::now()));
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
                    sel: None,
                });
                self.comp_refilter();
            }
            Pending::Hover { buf, pos, mouse } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                if mouse {
                    // Only if the pointer is still on the same word.
                    let still = self.hover_track.as_ref().is_some_and(|t| {
                        t.buf == buf && t.pos >= t.word.0 && pos >= t.word.0 && pos <= t.word.1
                    });
                    if bi != self.cur || !still {
                        return;
                    }
                } else if bi != self.cur || self.buffers[bi].cursor != pos {
                    return;
                }
                let diags = self.diagnostics_at(pos);
                let h = serde_json::from_value::<Option<lt::Hover>>(result)
                    .ok()
                    .flatten();
                let Some(h) = h else {
                    if mouse {
                        if !diags.is_empty() {
                            self.hover = Some(Hover {
                                lines: vec![],
                                diagnostics: diags,
                                anchor: Some(pos),
                            });
                        }
                        return;
                    }
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
                if lines.is_empty() && diags.is_empty() {
                    if !mouse {
                        self.msg("No hover info");
                    }
                    return;
                }
                self.hover = Some(Hover {
                    lines,
                    diagnostics: diags,
                    anchor: mouse.then_some(pos),
                });
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
                                    // Only the name and its kind, like Sublime (no signature).
                                    detail: kind_name(kind_num(&s.kind)).to_string(),
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
        self.rel(path)
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
        let (cur, focus, group) = (self.cur, self.focus, self.group);
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
        self.group = group;
        self.focus = focus;
        (n, files)
    }

    pub fn lsp_status(&self) -> String {
        let Some(b) = self.buf() else {
            return String::new();
        };
        match b.lsp_client.and_then(|c| self.lsp.clients.get(&c)) {
            Some(c) if !c.alive => format!("{} (dead)", c.server),
            Some(c) if !c.ready => format!("{} starting…", c.server),
            Some(c) => match &c.progress {
                Some(p) => format!("{} · {}", c.server, p.chars().take(40).collect::<String>()),
                None => c.server.clone(),
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
                    label: t.trim_start_matches('#').trim().to_string(),
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
        if let Some(k) = KEYWORDS.iter().find(|k| s.starts_with(*k)) {
            let indent = line.len() - t.len();
            let Some(name) = symbol_title(k, &s[k.len()..]) else {
                continue;
            };
            out.push(Item {
                label: format!("{}{}", " ".repeat(indent.min(8)), name),
                detail: k.trim_end_matches([' ', '<', '!']).to_string(),
                action: Action::Goto(Pos::new(i, indent)),
            });
        }
    }
    out
}

/// The title Goto Symbol shows for a declaration: just the name (`add`, `Point`), or
/// for an impl block its subject (`impl Display for Point`), never the signature.
fn symbol_title(keyword: &str, rest: &str) -> Option<String> {
    if keyword.starts_with("impl") {
        let mut r = rest;
        if keyword == "impl<" {
            // Skip the generic parameter list: impl<T: Clone> Foo<T>
            let mut depth = 1;
            let cut = r.char_indices().find(|&(_, c)| {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })?;
            r = r[cut.0 + 1..].trim_start();
        }
        let end = r.find(" where").or_else(|| r.find('{')).unwrap_or(r.len());
        let subject = r[..end].trim();
        return (!subject.is_empty()).then(|| format!("impl {subject}"));
    }
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Commands that act on the focused group's current file.
fn needs_buffer(cmd: Command) -> bool {
    use Command::*;
    matches!(
        cmd,
        Save | SaveAs
            | CloseBuffer
            | Undo
            | Redo
            | Cut
            | SelectLine
            | SelectWord
            | DeleteLine
            | DuplicateLine
            | MoveLineUp
            | MoveLineDown
            | ToggleComment
            | Indent
            | Outdent
            | InsertLineAfter
            | InsertLineBefore
            | JoinLines
            | UpperCase
            | LowerCase
            | SortLines
            | Find
            | FindNext
            | FindPrev
            | Replace
            | GotoSymbol
            | GotoLine
            | NextBuffer
            | PrevBuffer
            | NextDiagnostic
            | PrevDiagnostic
            | RevealInSidebar
            | SetSyntax
            | Indentation
            | CloseOtherTabs
            | CloseTabsToRight
    )
}

fn first_enabled(app: &App, menu: usize) -> usize {
    app.menus
        .get(menu)
        .and_then(|m| {
            m.items
                .iter()
                .position(|c| c.as_ref().is_some_and(|c| app.enabled(c.cmd)))
        })
        .unwrap_or(0)
}

/// Show a file or folder in the OS file manager (selecting it where supported).
fn reveal_in_file_manager(p: &Path) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(p);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", p.display()));
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if p.is_dir() {
            p
        } else {
            p.parent().unwrap_or(p)
        });
        c
    };
    if let Err(e) = cmd.spawn() {
        crate::log::warn("app", format!("couldn't open the file manager: {e}"));
    }
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
