//! Application state and all input/command handling, independent of the GUI toolkit.
//! The GUI (gui/) feeds keys/text/clipboard events in and renders this state. Each area
//! of behaviour lives in its own submodule as another `impl App` block.

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
use crate::lineedit::LineEdit;
use crate::lsp::{Manager, Pending, from_lsp, path_to_uri, to_lsp, uri_to_path};
use crate::menu::{self, Menu, MenuState};
use crate::packages;
use crate::palette::{Action, IndentAction, Item, Mode, Palette};
use crate::project::{self, Last, Project, Recent};
use crate::tree::Tree;
use crate::workspace::{BufferState, Workspace};

pub use timers::{FieldKind, TimerDialog, TimerKind};
pub use find::{FindField, FindMode, FindOpt, FindPanel, opt_command, opt_name};
use helpers::*;

mod buffers;
mod completion;
mod console;
mod context_menu;
mod editing;
mod find;
mod groups;
mod helpers;
mod hover;
mod info;
mod input;
mod lsp_client;
mod menus;
mod modal;
mod palettes;
mod timers;
mod panel;
mod run;
mod servers;
mod session;
mod settings;
mod sidebar;

pub enum AppEvent {
    Lsp(usize, Value),
    LspExit(usize),
    Files(PathBuf, Vec<String>),
    Install(String, InstallEvent),
    PackageInstalled(Result<String, String>),
    /// A native menu item was chosen (command id; macOS only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Menu(String),
    /// Progress of a grammar install (grammar name).
    Grammar(String, InstallEvent),
    /// Find in Files results (search generation).
    Search(u64, crate::search::SearchEvent),
    /// The open files as found on disk (`start_disk_check`).
    Disk(Vec<buffers::DiskState>),
    /// Background WakaTime work finished (wakatime.rs).
    WakaTime(crate::wakatime::Event),
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
    /// The find panel's fields.
    Find,
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
    pub input: LineEdit,
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
            input: LineEdit::default(),
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
    RenameSymbol,
    PackageUrl,
    OpenProject,
    SaveProjectAs,
    AddFolder,
    /// The WakaTime API key (a password field).
    WakaTimeKey,
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
    pub input: LineEdit,
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
    /// Reload a buffer changed on disk (dropping its unsaved edits).
    Reload(usize),
    /// Save a buffer over a file that changed on disk.
    Overwrite(usize),
    /// Replace what the last Find in Files found.
    ReplaceInFiles,
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
    /// Setup for the clock, pomodoro or alarm widget.
    Timers(TimerDialog),
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
    pub query: LineEdit,
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
    /// The diagnostic as the server sent it (sent back when asking for code actions).
    pub raw: Value,
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

/// A file or folder the GUI should ask for with the system's picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRequest {
    PackageFile,
    PackageFolder,
    GrammarFile,
    GrammarFolder,
    /// A folder for the find panel's Where field.
    FindFolder,
    /// File › Open File… / Open Folder…
    OpenFile,
    OpenFolder,
    OpenProject,
    AddFolder,
    /// Save the buffer with this id under a new name.
    SaveAs(usize),
    SaveProjectAs,
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
    /// The find panel (Find, Replace, Find in Files).
    pub find: FindPanel,
    pub find_opts: crate::search::FindOptions,
    matcher_cache: Option<(
        String,
        crate::search::FindOptions,
        Result<crate::search::Matcher, String>,
    )>,
    find_counted: Option<find::FindCount>,
    file_run: Option<find::FileRun>,
    search_gen: u64,
    clip: String,
    clip_line: bool,
    /// What each cursor copied (pasted back one per cursor when the counts match).
    clip_parts: Option<Vec<String>>,
    /// The code actions on offer: (server, buffer id, the action as the server sent it).
    code_actions: Vec<(usize, usize, Value)>,
    /// Buffers (ids) being formatted before they're saved.
    format_saving: std::collections::HashSet<usize>,
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
    /// The font before Select Font started previewing (Esc puts it back).
    font_before: Option<String>,
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
    /// Editor groups (layout columns), 1–4.
    pub layout: usize,
    /// Focused group.
    pub group: usize,
    /// Last active buffer id per group.
    pub group_active: [Option<usize>; MAX_GROUPS],
    /// Open files (buffer ids), most recently used first, for Ctrl+Tab.
    pub recent_files: Vec<usize>,
    /// While Ctrl is held after Ctrl+Tab: the order it started from and how far along it is.
    pub recent_cycle: Option<(Vec<usize>, usize)>,
    /// Width of each group as a fraction of the editor area (sums to 1).
    pub group_split: Vec<f32>,
    /// Open while "Preferences: Settings" shows defaults beside the user file.
    /// Open two-pane views (Preferences: Settings, Key Bindings).
    pub settings_view: Vec<SettingsView>,
    /// This process is a settings window (`--settings`).
    pub settings_window: bool,
    /// The config file as last loaded (another window may save it).
    /// The settings file and keymap.toml as last loaded (to notice either changing).
    config_stamp: [Option<fileops::DiskStamp>; 2],
    /// The project file on disk when last read or written here.
    pub(super) project_stamp: Option<fileops::DiskStamp>,
    /// A background check of the open files is running.
    disk_check_running: bool,
    /// When open files were last checked against the disk (deleted files).
    last_disk_check: Instant,
    /// Set by commands that need a file/folder from the user; the GUI shows the picker.
    pub file_request: Option<FileRequest>,
    /// Tests: the path the next system file picker "chooses" (dialogs can't be clicked).
    #[cfg(test)]
    pub test_pick: Option<PathBuf>,
    /// Tests: windows that would have been started (their arguments).
    #[cfg(test)]
    pub spawned: Vec<Vec<std::ffi::OsString>>,
    /// Columns the focused editor wraps at (set by the GUI each frame), None if not
    /// wrapping: Up/Down then move by visual rows.
    pub wrap_cols: Option<usize>,
    pub keymap: Keymap,
    pub menus: Vec<Menu>,
    /// Server name → install log (present while installing or after a failure).
    pub install_log: HashMap<String, Vec<String>>,
    pub installing: HashSet<String>,
    /// Grammars being installed.
    pub installing_grammars: HashSet<String>,
    /// Grammars we already told the user can be installed (once per session).
    grammar_hinted: HashSet<String>,
    /// The focused buffer (id, version) last checked for a typed shebang.
    shebang_seen: Option<(usize, i32)>,
    pub install_failed: HashMap<String, String>,
    /// Installed packages (folder names in Packages/).
    pub packages: Vec<String>,
    /// The clock, pomodoro and alarm at the right of the tab strip.
    pub timers: crate::timers::Timers,
    /// WakaTime time tracking (`wakatime` setting).
    pub wakatime: crate::wakatime::WakaTime,
    tx: EventTx,
}

impl App {
    pub fn new(project: Project, config: Config, tx: EventTx) -> Self {
        let syn = Syntax::load(&config.editor.theme);
        let mut tree = Tree::new();
        tree.anim = Duration::from_millis(if config.editor.animations { 160 } else { 0 });
        tree.set_roots(project.folders.clone(), false);
        let has_folders = !project.folders.is_empty();
        let recent = Recent::load(config.data_dir.as_deref());
        let wake = tx.clone();
        let timers = crate::timers::Timers::load(config.data_dir.as_deref(), move || wake.wake());
        let mut app = App {
            project,
            recent,
            last_autosave: Instant::now(),
            focus: Focus::Editor,
            // A window with no folder has nothing for the sidebar: it starts hidden.
            sidebar_visible: config.editor.show_sidebar && has_folders,
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
            find: FindPanel::default(),
            find_opts: Default::default(),
            matcher_cache: None,
            find_counted: None,
            file_run: None,
            search_gen: 0,
            clip: String::new(),
            clip_line: false,
            clip_parts: None,
            code_actions: vec![],
            format_saving: Default::default(),
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
            font_before: None,
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
            recent_files: Vec::new(),
            recent_cycle: None,
            group_split: vec![1.0],
            settings_view: Vec::new(),
            settings_window: false,
            config_stamp: [None, None],
            project_stamp: None,
            disk_check_running: false,
            wrap_cols: None,
            last_disk_check: Instant::now(),
            file_request: None,
            #[cfg(test)]
            test_pick: None,
            #[cfg(test)]
            spawned: vec![],
            keymap: Keymap::default(),
            menus: vec![],
            install_log: HashMap::new(),
            installing: HashSet::new(),
            installing_grammars: HashSet::new(),
            grammar_hinted: HashSet::new(),
            shebang_seen: None,
            install_failed: HashMap::new(),
            packages: vec![],
            timers,
            wakatime: Default::default(),
            config,
            tx,
        };
        app.config_stamp = app.config_stamp_now();
        app.project_stamp = app.project_stamp_now();
        let tx = app.tx.clone();
        app.tree.waker = Some(Arc::new(move || tx.wake()));
        app.apply_project_config();
        app.recent.touch(&app.project);
        app.recent.save(app.config.data_dir.as_deref());
        let errs = app.rebuild_keys_and_menus();
        if let Some(e) = app.config.error.clone().or(errs) {
            app.msg(format!("Config error: {e}"));
        }
        // Themes and highlight queries from Packages/ and the config folder.
        app.reload_syntax();
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
        self.redetect_shebang();
        self.wakatime_tick(Instant::now());
        if self.timers.tick(crate::timers::now()) {
            return true;
        }
        if self.last_disk_check.elapsed() >= Duration::from_secs(1) {
            self.last_disk_check = Instant::now();
            if self.check_config_file() {
                return true;
            }
            if self.check_project_file() {
                return true;
            }
            if self.timers.reload_if_changed() {
                return true;
            }
            self.start_disk_check();
        }
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
}
