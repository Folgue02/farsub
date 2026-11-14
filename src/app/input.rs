//! App: input entry points.

use super::*;

impl App {
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

    pub(super) fn paste_text(&mut self, text: String) {
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
}
