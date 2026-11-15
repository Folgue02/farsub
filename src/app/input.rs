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
            // Editing keys edit the modal's input like the editor; the rest are the modal's.
            if !self.input_key(k) {
                self.modal_key(k);
            }
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
        if self.focus == Focus::Find && self.find.open {
            // Editing keys edit the field; then the panel's own keys; then shortcuts (the
            // find family's editor bindings, such as F3, work here too).
            if self.input_key(k) || self.find_key(k) {
                return true;
            }
            let cmd = self.keymap.lookup(k, Ctx::Global).or_else(|| {
                self.keymap
                    .lookup(k, Ctx::Editor)
                    .filter(|c| crate::commands::is_find_command(*c))
            });
            if let Some(cmd) = cmd {
                self.run(cmd);
            }
            return true;
        }
        if k.code == KeyCode::Esc
            && self.find.open
            && self.focus == Focus::Editor
            && !k.shift
            && !k.ctrl
            && !k.alt
            && !k.sup
        {
            // Esc in the file closes the find panel (Sublime's hide_panel).
            self.close_find();
            return true;
        }
        if k.code == KeyCode::Enter
            && self.focus == Focus::Editor
            && !k.shift
            && !k.ctrl
            && !k.alt
            && !k.sup
            && let Some(l) = self
                .buf()
                .filter(|b| b.results.is_some())
                .map(|b| b.cursor.line)
            && self.open_result_line(l)
        {
            return true;
        }
        if self.focus == Focus::Console && self.console_shown() {
            if self.input_key(k) {
                return true;
            }
            // Select All with an empty input line selects the log text.
            if self.text_key_command(k) == Some(Command::SelectAll) {
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
            if let Some(input) = self.active_input() {
                input.insert(c.encode_utf8(&mut [0u8; 4]));
                self.input_changed();
                continue;
            }
            let in_term = self.focus == Focus::Terminal && self.terminal_shown().is_some();
            match &mut self.modal {
                Some(Modal::Servers(_)) => self.servers_char(c),
                Some(Modal::LspInfo(_)) => self.lsp_info_char(c),
                // Modals with an input were handled above.
                Some(Modal::Prompt(_) | Modal::Palette(_) | Modal::Keymap(_)) => {}
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
                None if self.focus == Focus::Sidebar && self.sidebar_visible => self.tree_char(c),
                None => self.editor_text(c),
            }
        }
    }

    pub fn on_paste(&mut self, s: &str) {
        if let Some(input) = self.active_input() {
            input.insert(s);
            self.input_changed();
            return;
        }
        let in_term = self.focus == Focus::Terminal && self.terminal_shown().is_some();
        match &mut self.modal {
            Some(_) => {}
            None if in_term => {
                if let Some(t) = self.current_terminal_mut() {
                    t.paste(s);
                }
            }
            None if self.focus == Focus::Sidebar && self.sidebar_visible => {
                self.run(Command::SidebarPaste)
            }
            None => self.paste_text(s.to_string()),
        }
    }

    pub fn on_copy(&mut self, cut: bool) {
        // A selection in a one-line input (find, palette, console line…) wins.
        if self.input_clipboard(if cut { Command::Cut } else { Command::Copy }) {
            return;
        }
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

    /// The one-line input that has keyboard focus, if any (modal inputs, the console line).
    pub(super) fn active_input(&mut self) -> Option<&mut LineEdit> {
        let console = self.focus == Focus::Console && self.console_shown();
        match &mut self.modal {
            Some(Modal::Prompt(p)) => Some(&mut p.input),
            Some(Modal::Palette(p)) => Some(&mut p.query),
            Some(Modal::Keymap(v)) => Some(&mut v.query),
            Some(_) => None,
            None if console => Some(&mut self.console.input),
            None if self.focus == Focus::Find && self.find.open => {
                let f = self.find.field;
                Some(self.find.field_mut(f))
            }
            None => None,
        }
    }

    /// The focused input's text changed: refresh what depends on it.
    pub(super) fn input_changed(&mut self) {
        match &mut self.modal {
            None if self.focus == Focus::Find => self.find_changed(),
            Some(Modal::Palette(_)) => self.palette_query_changed(),
            Some(Modal::Keymap(v)) => {
                v.sel = 0;
                v.scroll = 0;
            }
            _ => {}
        }
    }

    /// Copy / Cut the focused input's selection. False if there's no such selection.
    fn input_clipboard(&mut self, cmd: Command) -> bool {
        let Some(input) = self.active_input() else {
            return false;
        };
        let t = match cmd {
            Command::Cut => input.cut(),
            _ => input.selected_text(),
        };
        let Some(t) = t else { return false };
        self.clip = t.clone();
        self.clip_line = false;
        self.pending_copy = Some(t);
        if cmd == Command::Cut {
            self.input_changed();
        }
        true
    }

    /// The command a key runs when editing text: editor bindings first, then global ones
    /// (Select All, Undo, Copy… are global), as in the editor itself.
    fn text_key_command(&self, k: Key) -> Option<Command> {
        self.keymap
            .lookup(k, Ctx::Editor)
            .or_else(|| self.keymap.lookup(k, Ctx::Global))
    }

    /// Editing keys in a one-line input: looked up in the editor's keymap and run by the
    /// same code as in the editor (see lineedit.rs). Returns false if the key isn't an
    /// editing command for the input, so its owner handles it (Enter, Esc, Up/Down…).
    pub(super) fn input_key(&mut self, k: Key) -> bool {
        match self.text_key_command(k) {
            Some(cmd) => self.input_command(cmd),
            None => false,
        }
    }

    /// Whether a one-line input has keyboard focus (so text commands apply to it).
    pub fn has_input(&self) -> bool {
        match &self.modal {
            Some(Modal::Prompt(_) | Modal::Palette(_) | Modal::Keymap(_)) => true,
            Some(_) => false,
            None => {
                (self.focus == Focus::Console && self.console_shown())
                    || (self.focus == Focus::Find && self.find.open)
            }
        }
    }

    /// Run a text-editing command on the focused input. False if there's no input, the
    /// command isn't a text command, or it belongs to the input's owner right now.
    pub(super) fn input_command(&mut self, cmd: Command) -> bool {
        if !crate::lineedit::accepts(cmd) {
            return false;
        }
        let empty = match self.active_input() {
            Some(i) => i.is_empty(),
            None => return false,
        };
        match cmd {
            // In an empty input these belong to the owner: Backspace leaves a palette
            // mode, Select All in the console selects the log.
            Command::DeleteLeft | Command::SelectAll if empty => false,
            Command::Copy | Command::Cut => self.input_clipboard(cmd),
            Command::Paste => {
                let t = self.clipboard_text();
                if let Some(i) = self.active_input() {
                    i.insert(&t);
                }
                self.input_changed();
                true
            }
            _ => {
                let changed = self.active_input().and_then(|i| i.command(cmd));
                if changed == Some(true) {
                    self.input_changed();
                }
                changed.is_some()
            }
        }
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
            AppEvent::PackageInstalled(res) => match res {
                Ok(name) => {
                    self.msg(format!("Installed package {name} — loading syntaxes…"));
                    self.run(Command::ReloadPackages);
                }
                Err(e) => self.msg(format!("Package install failed: {e}")),
            },
            AppEvent::Grammar(name, ev) => self.on_grammar_event(name, ev),
            AppEvent::Search(id, ev) => self.on_search_event(id, ev),
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
