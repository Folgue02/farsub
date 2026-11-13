//! Application state and all input/command handling.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEventKind};
use lsp_types as lt;
use serde_json::{Value, json};

use crate::clipboard;
use crate::commands::{COMMANDS, Command};
use crate::editor::{Buffer, Pos, char_len, is_word};
use crate::highlight::{HlCache, Syntax};
use crate::keys::Key;
use crate::lsp::{self, Manager, Pending, from_lsp, path_to_uri, to_lsp, uri_to_path};
use crate::palette::{Action, Item, Mode, Palette};
use crate::panels::{self, Panel};

pub enum AppEvent {
    Lsp(usize, Value),
    LspExit(usize),
    Files(PathBuf, Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Panels,
    Editor,
}

pub enum PromptKind {
    Copy(Vec<PathBuf>),
    Move(Vec<PathBuf>),
    Mkdir,
    NewFile,
    SaveAs,
    Find { origin: Pos },
    ReplaceFind,
    ReplaceWith(String),
    Rename,
}

pub struct Prompt {
    pub title: String,
    pub input: String,
    pub cursor: usize,
    pub kind: PromptKind,
}

pub enum ConfirmKind {
    Delete(Vec<PathBuf>),
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
    pub view: View,
    pub panels: [Panel; 2],
    pub active: usize,
    pub cmdline: String,
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
    pub diagnostics: HashMap<PathBuf, Vec<Diag>>,
    pub files: HashMap<PathBuf, Vec<String>>,
    indexing: Option<PathBuf>,
    pub search: Option<String>,
    clip: String,
    clip_line: bool,
    jumps: Vec<(PathBuf, Pos)>,
    pub quit: bool,
    pub pending_shell: Option<(String, PathBuf)>,
    pub page: usize,
    pub panel_page: usize,
    tx: Sender<AppEvent>,
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
    pub fn new(start: PathBuf, tx: Sender<AppEvent>) -> Self {
        let show_hidden = false;
        App {
            view: View::Panels,
            panels: [
                Panel::new(start.clone(), show_hidden),
                Panel::new(start, show_hidden),
            ],
            active: 0,
            cmdline: String::new(),
            show_hidden,
            buffers: vec![],
            cur: 0,
            next_buf_id: 1,
            syn: Syntax::load(),
            modal: None,
            completion: None,
            hover: None,
            status: None,
            lsp: Manager::new(tx.clone()),
            diagnostics: HashMap::new(),
            files: HashMap::new(),
            indexing: None,
            search: None,
            clip: String::new(),
            clip_line: false,
            jumps: vec![],
            quit: false,
            pending_shell: None,
            page: 20,
            panel_page: 20,
            tx,
        }
    }

    pub fn msg(&mut self, s: impl Into<String>) {
        self.status = Some((s.into(), Instant::now()));
    }

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

    fn buf_mut(&mut self) -> Option<&mut Buffer> {
        self.buffers.get_mut(self.cur)
    }

    fn buf_index(&self, id: usize) -> Option<usize> {
        self.buffers.iter().position(|b| b.id == id)
    }

    pub fn panel(&mut self) -> &mut Panel {
        &mut self.panels[self.active]
    }

    pub fn reload_panels(&mut self) {
        let h = self.show_hidden;
        for p in &mut self.panels {
            p.reload(h);
        }
    }

    // ===================================================================== events

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(Key::from_event(&k)),
            Event::Paste(s) => self.on_paste(&s),
            _ => {}
        }
    }

    fn on_paste(&mut self, s: &str) {
        match &mut self.modal {
            Some(Modal::Prompt(p)) => {
                let line = s.lines().next().unwrap_or("");
                let b = crate::editor::byte_idx(&p.input, p.cursor);
                p.input.insert_str(b, line);
                p.cursor += char_len(line);
            }
            Some(Modal::Palette(p)) => {
                p.query.push_str(s.lines().next().unwrap_or(""));
                self.palette_query_changed();
            }
            Some(_) => {}
            None => match self.view {
                View::Editor => {
                    if let Some(b) = self.buf_mut() {
                        b.insert_text(s);
                    }
                }
                View::Panels => self.cmdline.push_str(s.lines().next().unwrap_or("")),
            },
        }
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

    fn on_key(&mut self, k: Key) {
        if self.modal.is_some() {
            self.modal_key(k);
            return;
        }
        if let Some(cmd) = self.global_binding(k) {
            self.run(cmd);
            return;
        }
        match self.view {
            View::Panels => self.panels_key(k),
            View::Editor => self.editor_key(k),
        }
    }

    fn global_binding(&self, k: Key) -> Option<Command> {
        use Command::*;
        let ed = self.view == View::Editor;
        Some(match (k.code, k.ctrl, k.shift) {
            (KeyCode::Char('p'), true, false) => GotoFile,
            (KeyCode::Char('p'), true, true) | (KeyCode::F(1), _, _) => CommandPalette,
            (KeyCode::Char('r'), true, true) => GotoWorkspaceSymbol,
            (KeyCode::Char('r'), true, false) if ed => GotoSymbol,
            (KeyCode::Char('g'), true, false) if ed => GotoLine,
            (KeyCode::Char('b'), true, false) => SwitchBuffer,
            (KeyCode::Char('o'), true, false) => TogglePanels,
            (KeyCode::Char('q'), true, false) => Quit,
            (KeyCode::Char('n'), true, false) => NewFile,
            (KeyCode::PageDown, true, _) => NextBuffer,
            (KeyCode::PageUp, true, _) => PrevBuffer,
            (KeyCode::Char('-'), true, _) => JumpBack,
            (KeyCode::Char('-'), _, _) if k.alt => JumpBack,
            _ => return None,
        })
    }

    // ===================================================================== panels

    fn panels_key(&mut self, k: Key) {
        let h = self.show_hidden;
        let page = self.panel_page.max(1) as isize;
        match (k.code, k.ctrl, k.shift) {
            (KeyCode::Tab, false, false) => self.active ^= 1,
            (KeyCode::Up, _, s) => {
                if s {
                    self.panel().toggle_mark();
                }
                self.panel().move_sel(-1)
            }
            (KeyCode::Down, _, s) => {
                if s {
                    self.panel().toggle_mark();
                }
                self.panel().move_sel(1)
            }
            (KeyCode::PageUp, false, _) => self.panel().move_sel(-page),
            (KeyCode::PageDown, false, _) => self.panel().move_sel(page),
            (KeyCode::Home, _, _) if self.cmdline.is_empty() => self.panel().sel = 0,
            (KeyCode::End, _, _) if self.cmdline.is_empty() => {
                let n = self.panel().entries.len();
                self.panel().sel = n.saturating_sub(1)
            }
            (KeyCode::Insert, _, _) | (KeyCode::Char(' '), true, _) => {
                self.panel().toggle_mark();
                self.panel().move_sel(1);
            }
            (KeyCode::Enter, _, _) => {
                if !self.cmdline.trim().is_empty() {
                    let cmd = std::mem::take(&mut self.cmdline);
                    self.exec_cmdline(cmd.trim().to_string());
                } else {
                    self.panel_enter();
                }
            }
            (KeyCode::Backspace, _, _) => {
                if self.cmdline.pop().is_none()
                    && let Some(parent) = self.panel().dir.parent().map(|p| p.to_path_buf())
                {
                    self.panel().chdir(parent, h);
                }
            }
            (KeyCode::Esc, _, _) => self.cmdline.clear(),
            (KeyCode::F(3), _, _) => self.panel_open(true),
            (KeyCode::F(4), _, false) => self.panel_open(false),
            (KeyCode::F(4), _, true) => self.run(Command::NewFile),
            (KeyCode::F(5), _, _) => self.panel_copy_move(false),
            (KeyCode::F(6), _, _) => self.panel_copy_move(true),
            (KeyCode::F(7), _, _) => {
                self.prompt("Make folder", "", PromptKind::Mkdir);
            }
            (KeyCode::F(8), _, _) => {
                let t = self.panel().targets();
                if !t.is_empty() {
                    let what = if t.len() == 1 {
                        t[0].file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        format!("{} items", t.len())
                    };
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!("Delete {what}? This cannot be undone."),
                        kind: ConfirmKind::Delete(t),
                    }));
                }
            }
            (KeyCode::F(10), _, _) => self.run(Command::Quit),
            (KeyCode::Char('h'), true, _) => self.run(Command::ToggleHidden),
            (KeyCode::Char('r'), true, false) => self.run(Command::RefreshPanels),
            (KeyCode::Char('u'), true, _) => self.run(Command::SwapPanels),
            _ => {
                if let Some(c) = k.text_char() {
                    self.cmdline.push(c);
                }
            }
        }
    }

    fn panel_enter(&mut self) {
        let h = self.show_hidden;
        let Some(e) = self.panel().current().cloned() else {
            return;
        };
        if e.is_dir {
            self.panel().chdir(e.path, h);
        } else {
            self.open_file(&e.path, true);
        }
    }

    fn panel_open(&mut self, view_only: bool) {
        let Some(e) = self.panel().current().cloned() else {
            return;
        };
        if e.is_dir {
            return;
        }
        if let Some(i) = self.open_file(&e.path, true) {
            self.buffers[i].read_only = view_only && !self.buffers[i].modified;
            if !view_only {
                self.buffers[i].read_only = false;
            }
        }
    }

    fn panel_copy_move(&mut self, mv: bool) {
        let t = self.panel().targets();
        if t.is_empty() {
            return;
        }
        let other = self.panels[self.active ^ 1].dir.clone();
        let label = if mv { "Rename/Move" } else { "Copy" };
        let title = if t.len() == 1 {
            format!(
                "{label} {} to",
                t[0].file_name().unwrap_or_default().to_string_lossy()
            )
        } else {
            format!("{label} {} items to", t.len())
        };
        let mut def = other.to_string_lossy().into_owned();
        if mv && t.len() == 1 && other == self.panel().dir {
            def = t[0].to_string_lossy().into_owned();
        }
        let kind = if mv {
            PromptKind::Move(t)
        } else {
            PromptKind::Copy(t)
        };
        self.prompt(&title, &def, kind);
    }

    fn exec_cmdline(&mut self, cmd: String) {
        let h = self.show_hidden;
        let dir = self.panel().dir.clone();
        if cmd == "cd" || cmd.starts_with("cd ") {
            let arg = cmd[2..].trim();
            let target = if arg.is_empty() || arg == "~" {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or(dir.clone())
            } else if let Some(rest) = arg.strip_prefix("~/") {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join(rest)
            } else {
                dir.join(arg)
            };
            match target.canonicalize() {
                Ok(p) if p.is_dir() => self.panel().chdir(p, h),
                _ => self.msg(format!("cd: no such directory: {arg}")),
            }
            return;
        }
        self.pending_shell = Some((cmd, dir));
    }

    // ===================================================================== modal

    fn prompt(&mut self, title: &str, def: &str, kind: PromptKind) {
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
            Some(Modal::Confirm(c)) => match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.confirm(c.kind),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Confirm(c)),
            },
            None => {}
        }
    }

    fn confirm(&mut self, kind: ConfirmKind) {
        match kind {
            ConfirmKind::Delete(paths) => {
                let mut n = 0;
                for p in &paths {
                    match panels::delete_path(p) {
                        Ok(()) => n += 1,
                        Err(e) => self.msg(format!("delete {}: {e}", p.display())),
                    }
                }
                self.panel().marked.clear();
                self.reload_panels();
                self.files.clear();
                if n == paths.len() {
                    self.msg(format!("Deleted {n} item(s)"));
                }
            }
            ConfirmKind::CloseBuffer(id) => {
                if let Some(i) = self.buf_index(id) {
                    self.close_buffer_at(i);
                }
            }
            ConfirmKind::Quit => self.quit = true,
        }
    }

    fn prompt_key(&mut self, k: Key, mut p: Prompt) {
        match k.code {
            KeyCode::Esc => {
                if let PromptKind::Find { origin } = p.kind
                    && let Some(b) = self.buf_mut()
                {
                    b.goto(origin);
                }
                return;
            }
            KeyCode::Enter => {
                self.prompt_accept(p);
                return;
            }
            KeyCode::Backspace => {
                if p.cursor > 0 {
                    let b = crate::editor::byte_idx(&p.input, p.cursor - 1);
                    p.input.remove(b);
                    p.cursor -= 1;
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
            KeyCode::Char('u') if k.ctrl => {
                p.input.clear();
                p.cursor = 0;
            }
            KeyCode::F(3) | KeyCode::Down if matches!(p.kind, PromptKind::Find { .. }) => {
                self.search = Some(p.input.clone());
                self.find_next(!k.shift);
            }
            KeyCode::Up if matches!(p.kind, PromptKind::Find { .. }) => {
                self.search = Some(p.input.clone());
                self.find_next(false);
            }
            _ => {
                if let Some(c) = k.text_char() {
                    let b = crate::editor::byte_idx(&p.input, p.cursor);
                    p.input.insert(b, c);
                    p.cursor += 1;
                }
            }
        }
        // Incremental find.
        if let PromptKind::Find { origin } = p.kind
            && !matches!(k.code, KeyCode::F(3) | KeyCode::Up | KeyCode::Down)
        {
            let q = p.input.clone();
            self.search = Some(q.clone());
            if let Some(b) = self.buf_mut() {
                if let Some((s, e)) = b.find(&q, origin, true, false) {
                    b.anchor = Some(s);
                    b.cursor = e;
                } else {
                    b.goto(origin);
                }
            }
        }
        self.modal = Some(Modal::Prompt(p));
    }

    fn prompt_accept(&mut self, p: Prompt) {
        let input = p.input.trim().to_string();
        let h = self.show_hidden;
        let resolve = |base: &Path, s: &str| -> PathBuf {
            if let Some(rest) = s.strip_prefix("~/") {
                return std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_default()
                    .join(rest);
            }
            let p = PathBuf::from(s);
            if p.is_absolute() { p } else { base.join(p) }
        };
        let dir = self.panels[self.active].dir.clone();
        match p.kind {
            PromptKind::Copy(srcs) | PromptKind::Move(srcs) if input.is_empty() => {
                let _ = srcs;
            }
            PromptKind::Copy(srcs) => {
                let dest = resolve(&dir, &input);
                let multi = srcs.len() > 1;
                let mut ok = 0;
                for s in &srcs {
                    let d = panels::dest_for(s, &dest, multi);
                    match panels::copy_recursive(s, &d) {
                        Ok(()) => ok += 1,
                        Err(e) => self.msg(format!("copy {}: {e}", s.display())),
                    }
                }
                self.panel().marked.clear();
                self.reload_panels();
                self.files.clear();
                if ok == srcs.len() {
                    self.msg(format!("Copied {ok} item(s)"));
                }
            }
            PromptKind::Move(srcs) => {
                let dest = resolve(&dir, &input);
                let multi = srcs.len() > 1;
                let mut ok = 0;
                for s in &srcs {
                    let d = panels::dest_for(s, &dest, multi);
                    match panels::move_path(s, &d) {
                        Ok(()) => {
                            ok += 1;
                            for b in &mut self.buffers {
                                if b.path.as_deref() == Some(s.as_path()) {
                                    b.set_path(d.clone());
                                }
                            }
                        }
                        Err(e) => self.msg(format!("move {}: {e}", s.display())),
                    }
                }
                self.panel().marked.clear();
                self.reload_panels();
                self.files.clear();
                if ok == srcs.len() {
                    self.msg(format!("Moved {ok} item(s)"));
                }
            }
            PromptKind::Mkdir => {
                if input.is_empty() {
                    return;
                }
                let d = resolve(&dir, &input);
                match std::fs::create_dir_all(&d) {
                    Ok(()) => {
                        self.reload_panels();
                        let name = d
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        self.panel().select_name(&name);
                    }
                    Err(e) => self.msg(format!("mkdir: {e}")),
                }
                let _ = h;
            }
            PromptKind::NewFile => {
                if input.is_empty() {
                    let id = self.next_buf_id;
                    self.next_buf_id += 1;
                    let b = Buffer::from_text(id, None, "", HlCache::new("Plain Text".into()));
                    self.buffers.push(b);
                    self.cur = self.buffers.len() - 1;
                    self.view = View::Editor;
                    return;
                }
                let base = if self.view == View::Editor {
                    self.buf()
                        .and_then(|b| b.path.as_ref())
                        .and_then(|p| p.parent())
                        .map(|p| p.to_path_buf())
                        .unwrap_or(dir)
                } else {
                    dir
                };
                let path = resolve(&base, &input);
                if !path.exists() {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(&path, "") {
                        self.msg(format!("create: {e}"));
                        return;
                    }
                }
                self.files.clear();
                self.reload_panels();
                self.open_file(&path, true);
            }
            PromptKind::SaveAs => {
                if input.is_empty() {
                    return;
                }
                let base = self
                    .buf()
                    .and_then(|b| b.path.as_ref())
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
                    .unwrap_or(dir);
                let path = resolve(&base, &input);
                let first = self.buf().map(|b| b.lines[0].clone()).unwrap_or_default();
                let syntax = self.syn.detect(Some(&path), &first);
                if let Some(b) = self.buf_mut() {
                    b.set_path(path);
                    b.hl.reset(syntax);
                    b.lsp_client = None;
                }
                self.save_current();
                let cur = self.cur;
                self.lsp_open(cur);
                self.files.clear();
                self.reload_panels();
            }
            PromptKind::Find { .. } => {
                self.search = Some(p.input.clone());
                if let Some(b) = self.buf_mut()
                    && b.selection().is_none()
                {
                    self.msg("Not found");
                }
            }
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
            PromptKind::Rename => {
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

    // ===================================================================== palette

    fn open_palette(&mut self, mode: Mode) {
        let mut p = Palette::new(mode, Palette::prefix(mode));
        if matches!(mode, Mode::Symbols | Mode::Line)
            && self.view == View::Editor
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
        if self.view == View::Editor
            && let Some(p) = self.buf().and_then(|b| b.path.as_ref())
        {
            return lsp::find_root(p);
        }
        let d = &self.panels[self.active].dir;
        let r = lsp::find_root(d);
        // Only climb to a project root if the panel is inside a real project.
        if r.join(".git").exists() || r != *d {
            r
        } else {
            d.clone()
        }
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
                title = format!("Files in {}", root.display());
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
                .map(|(c, name, key)| Item {
                    label: name.to_string(),
                    detail: key.to_string(),
                    action: Action::Command(*c),
                })
                .collect(),
            Mode::Symbols => {
                title = "Symbols".into();
                let items = self.buf().map(heuristic_symbols).unwrap_or_default();
                if self.view == View::Editor && self.request_doc_symbols() {
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
                        + if b.modified { " ●" } else { "" },
                    action: Action::Buffer(i),
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
                if let Some((bi, pos, sy)) = p.origin
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
            KeyCode::Backspace => {
                if p.query.pop().is_none() && p.fixed {
                    return;
                }
                self.palette_query_changed();
            }
            _ => {
                if let Some(c) = k.text_char() {
                    p.query.push(c);
                    self.palette_query_changed();
                }
            }
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
                    self.view = View::Editor;
                }
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
                self.view = View::Editor;
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
        let b = Buffer::from_text(id, Some(path), &text, HlCache::new(syntax));
        self.buffers.push(b);
        let i = self.buffers.len() - 1;
        if focus {
            self.cur = i;
            self.view = View::Editor;
        }
        self.lsp_open(i);
        Some(i)
    }

    pub fn open_new_path(&mut self, path: PathBuf) {
        let syntax = self.syn.detect(Some(&path), "");
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let b = Buffer::from_text(id, Some(path), "", HlCache::new(syntax));
        self.buffers.push(b);
        self.cur = self.buffers.len() - 1;
        self.view = View::Editor;
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
                let h = self.show_hidden;
                for p in &mut self.panels {
                    p.reload(h);
                }
            }
            Err(e) => self.msg(format!("Save failed: {e}")),
        }
    }

    fn close_buffer_at(&mut self, i: usize) {
        let b = self.buffers.remove(i);
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
            self.view = View::Panels;
        } else if self.cur >= self.buffers.len() || self.cur > i {
            self.cur = self.cur.saturating_sub(1).min(self.buffers.len() - 1);
        }
    }

    // ===================================================================== editor

    fn editor_key(&mut self, k: Key) {
        if self.buffers.is_empty() {
            if k.code == KeyCode::Esc {
                self.view = View::Panels;
            }
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
        if let Some(cmd) = editor_binding(k) {
            self.run(cmd);
            return;
        }
        let page = self.page.max(1) as isize;
        let Some(b) = self.buf_mut() else { return };
        let sel = k.shift;
        let word = k.alt || (k.ctrl && !k.sup);
        let mut typed: Option<char> = None;
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
            _ => {
                if let Some(c) = k.text_char() {
                    if b.read_only {
                        self.msg("Read-only (F4 in panels to edit)");
                        return;
                    }
                    b.insert_char(c);
                    typed = Some(c);
                }
            }
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
                .map(|c| c.trigger_chars())
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

    pub fn run(&mut self, cmd: Command) {
        use Command::*;
        let page = self.page;
        match cmd {
            GotoFile => self.open_palette(Mode::Files),
            CommandPalette => self.open_palette(Mode::Commands),
            GotoSymbol => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                self.view = View::Editor;
                self.open_palette(Mode::Symbols)
            }
            GotoWorkspaceSymbol => self.open_palette(Mode::WorkspaceSymbols),
            GotoLine => {
                if self.buffers.is_empty() {
                    return;
                }
                self.view = View::Editor;
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
            TogglePanels => {
                self.view = match self.view {
                    View::Panels if !self.buffers.is_empty() => View::Editor,
                    View::Panels => {
                        self.msg("No open files — Ctrl+P or F4 to open one");
                        View::Panels
                    }
                    View::Editor => View::Panels,
                };
                self.completion = None;
                self.hover = None;
            }
            NewFile => self.prompt("New file (empty = untitled)", "", PromptKind::NewFile),
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
                    self.view = View::Editor;
                    self.completion = None;
                }
            }
            ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.reload_panels();
                let s = if self.show_hidden { "shown" } else { "hidden" };
                self.msg(format!("Hidden files {s}"));
            }
            RefreshPanels => {
                self.reload_panels();
                self.files.clear();
            }
            SwapPanels => self.panels.swap(0, 1),
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
                self.lsp.restart();
                self.diagnostics.clear();
                for i in 0..self.buffers.len() {
                    self.buffers[i].lsp_client = None;
                    self.lsp_open(i);
                }
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

    fn run_editor(&mut self, cmd: Command) {
        use Command::*;
        if self.buffers.is_empty() || self.view != View::Editor {
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
                clipboard::set(&text);
                self.clip = text;
                self.clip_line = line;
                return;
            }
            Paste => {
                let text = clipboard::get().unwrap_or_else(|| self.clip.clone());
                let line_mode = self.clip_line && text == self.clip;
                let b = &mut self.buffers[self.cur];
                if line_mode && b.selection().is_none() {
                    let c = b.cursor;
                    b.goto(Pos::new(c.line, 0));
                    b.insert_text(&text);
                    b.goto(Pos::new(c.line + 1, c.col));
                } else {
                    b.insert_text(&text);
                }
                return;
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
                return self.prompt("Replace all: find", &def, PromptKind::ReplaceFind);
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
                return self.prompt("Rename symbol to", &w, PromptKind::Rename);
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
        let b = &mut self.buffers[i];
        let (Some(path), Some(lang)) = (b.path.clone(), b.lang) else {
            return;
        };
        match self.lsp.client_for(lang, &path) {
            Ok(cid) => {
                b.lsp_client = Some(cid);
                b.lsp_dirty = false;
                let text = b.text();
                let version = b.version;
                if let Some(c) = self.lsp.clients.get_mut(&cid) {
                    c.notify(
                        "textDocument/didOpen",
                        json!({"textDocument": {"uri": path_to_uri(&path), "languageId": lang,
                               "version": version, "text": text}}),
                    );
                }
            }
            Err(e) => self.msg(format!("LSP: {e}")),
        }
    }

    /// Push full-document changes for every buffer edited since the last sync.
    pub fn sync_lsp(&mut self) {
        for b in &mut self.buffers {
            if !b.lsp_dirty {
                continue;
            }
            b.lsp_dirty = false;
            let (Some(cid), Some(path)) = (b.lsp_client, &b.path) else {
                continue;
            };
            let Some(c) = self.lsp.clients.get_mut(&cid) else {
                continue;
            };
            c.notify(
                "textDocument/didChange",
                json!({"textDocument": {"uri": path_to_uri(path), "version": b.version},
                       "contentChanges": [{"text": b.text()}]}),
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
                let n = params
                    .get("items")
                    .and_then(|i| i.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                Value::Array(vec![Value::Null; n])
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
                if bi != self.cur || self.view != View::Editor || self.modal.is_some() {
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
        let (cur, view) = (self.cur, self.view);
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
        self.view = view;
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

fn editor_binding(k: Key) -> Option<Command> {
    use Command::*;
    use KeyCode::*;
    Some(match (k.code, k.ctrl, k.shift, k.alt) {
        (Char('s'), true, false, _) => Save,
        (Char('s'), true, true, _) => SaveAs,
        (Char('z'), true, false, _) => Undo,
        (Char('z'), true, true, _) | (Char('y'), true, _, _) => Redo,
        (Char('c'), true, false, _) => Copy,
        (Char('x'), true, false, _) => Cut,
        (Char('v'), true, false, _) => Paste,
        (Char('a'), true, false, _) => SelectAll,
        (Char('l'), true, false, _) => SelectLine,
        (Char('d'), true, false, _) => SelectWord,
        (Char('d'), true, true, _) => DuplicateLine,
        (Char('k'), true, true, _) => DeleteLine,
        (Char('k'), true, false, _) => Hover,
        (Char('/'), true, _, _) => ToggleComment,
        (Char(']'), true, _, _) => Indent,
        (Char('j'), true, false, _) => JoinLines,
        (Char('f'), true, false, false) => Find,
        (Char('f'), _, true, true) => Format,
        (Char('h'), true, _, _) => Replace,
        (Char(' '), true, _, _) => Completion,
        (Char('w'), true, false, _) => CloseBuffer,
        (Up, false, false, true) | (Up, true, true, false) => MoveLineUp,
        (Down, false, false, true) | (Down, true, true, false) => MoveLineDown,
        (Enter, true, false, _) => InsertLineAfter,
        (Enter, true, true, _) => InsertLineBefore,
        (F(2), _, _, _) => Rename,
        (F(3), _, false, _) => FindNext,
        (F(3), _, true, _) => FindPrev,
        (F(8), _, false, _) => NextDiagnostic,
        (F(8), _, true, _) => PrevDiagnostic,
        (F(9), _, _, _) => SortLines,
        (F(10), _, _, _) => CloseBuffer,
        (F(12), _, false, _) => GotoDefinition,
        (F(12), _, true, _) => FindReferences,
        _ => return None,
    })
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
