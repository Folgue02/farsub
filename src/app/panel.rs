//! App: bottom panel.

use super::*;

impl App {
    // ===================================================================== bottom panel

    pub fn console_shown(&self) -> bool {
        self.panel.visible && self.panel.tab == PanelTab::Console
    }

    /// The terminal shown in the panel, if the panel is showing one.
    pub fn terminal_shown(&self) -> Option<usize> {
        match self.panel.tab {
            PanelTab::Terminal(id)
                if self.panel.visible
                    && self.terminals.iter().any(|t| t.id == id)
                    && self.tab_of_terminal(id).is_none() =>
            {
                Some(id)
            }
            _ => None,
        }
    }

    /// The terminal keys go to: the panel's when it has focus, or the current tab's
    /// when that tab is a terminal and the editor has focus.
    pub fn active_terminal(&self) -> Option<usize> {
        match self.focus {
            Focus::Terminal => self.terminal_shown(),
            Focus::Editor => self.buf().and_then(|b| b.term),
            _ => None,
        }
    }

    /// The buffer (tab) that shows terminal `id`, if it is in the editor area.
    pub fn tab_of_terminal(&self, id: usize) -> Option<usize> {
        self.buffers.iter().position(|b| b.term == Some(id))
    }

    /// The terminals listed in the panel (not the ones shown as editor tabs).
    pub fn panel_terminals(&self) -> Vec<usize> {
        (self.terminals.iter())
            .map(|t| t.id)
            .filter(|&id| self.tab_of_terminal(id).is_none())
            .collect()
    }

    pub fn current_terminal_mut(&mut self) -> Option<&mut crate::terminal::Terminal> {
        let id = self.active_terminal()?;
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
        if let Some(id) = self.spawn_terminal(cwd) {
            self.show_terminal(id);
        }
    }

    /// Terminal: New Terminal in Editor Area — a shell in a tab of the focused group.
    pub fn new_terminal_tab(&mut self) {
        let cwd = self.base_dir();
        if let Some(id) = self.spawn_terminal(&cwd) {
            self.terminal_to_tab(id);
        }
    }

    fn spawn_terminal(&mut self, cwd: &Path) -> Option<usize> {
        let id = self.next_term_id;
        self.next_term_id += 1;
        match crate::terminal::Terminal::spawn(
            id,
            cwd,
            24,
            80,
            self.config.editor.terminal_shell.as_deref(),
            self.tx.clone(),
        ) {
            Ok(t) => {
                self.terminals.push(t);
                Some(id)
            }
            Err(e) => {
                crate::log::error("terminal", &e);
                self.msg(format!("Couldn't start a terminal: {e}"));
                None
            }
        }
    }

    /// Show terminal `id` as a tab at the end of the focused group's tabs, and focus it.
    fn terminal_to_tab(&mut self, id: usize) {
        let Some(t) = self.terminals.iter().find(|t| t.id == id) else {
            return;
        };
        let folder = t
            .cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| t.cwd.display().to_string());
        let title = if folder.is_empty() {
            t.shell.clone()
        } else {
            format!("{} · {folder}", t.shell)
        };
        let bid = self.next_buf_id;
        self.next_buf_id += 1;
        let mut b = Buffer::from_text(bid, None, "", HlCache::new(String::new()));
        b.title = Some(title);
        b.read_only = true;
        b.term = Some(id);
        let i = self.push_buffer(b);
        self.set_cur(i);
        self.focus = Focus::Editor;
        self.completion = None;
        self.hover = None;
    }

    /// Terminal: Move Terminal into Editor Area — the panel's terminal becomes a tab.
    pub fn move_terminal_to_editor(&mut self) {
        let Some(id) = self.terminal_shown() else {
            return;
        };
        self.leave_panel(id);
        self.terminal_to_tab(id);
    }

    /// Terminal: Move Terminal to Panel — the current terminal tab goes back to the
    /// panel (its shell keeps running).
    pub fn move_terminal_to_panel(&mut self) {
        let Some(i) = self.buf().filter(|b| b.term.is_some()).map(|_| self.cur) else {
            return;
        };
        let id = self.buffers[i].term.take().unwrap();
        self.close_buffer_at(i);
        self.show_terminal(id);
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

    /// Kill terminal `id`: its editor tab closes, or the panel shows the next one.
    pub fn close_terminal(&mut self, id: usize) {
        if let Some(i) = self.tab_of_terminal(id) {
            // close_buffer_at kills a tab's terminal.
            self.close_buffer_at(i);
            return;
        }
        if !self.terminals.iter().any(|t| t.id == id) {
            return;
        }
        self.leave_panel(id);
        self.terminals.retain(|t| t.id != id);
    }

    /// Terminal `id` leaves the panel: the panel shows the panel terminal before it (else
    /// the first), or hides when it was the last one.
    fn leave_panel(&mut self, id: usize) {
        let list = self.panel_terminals();
        let Some(i) = list.iter().position(|&t| t == id) else {
            return;
        };
        let rest: Vec<usize> = list.into_iter().filter(|&t| t != id).collect();
        let next = rest.get(i.saturating_sub(1)).or(rest.first()).copied();
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
    pub(super) fn terminal_key(&mut self, k: Key) -> bool {
        use Command::*;
        let Some(id) = self.active_terminal() else {
            return false;
        };
        let in_tab = self.tab_of_terminal(id).is_some();
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
        // Ctrl+Tab / Ctrl+Shift+Tab cycle the panel's terminals while one has focus; in
        // a terminal tab they switch files like anywhere in the editor area.
        if k.code == KeyCode::Tab && k.ctrl && !k.alt && !in_tab {
            let list = self.panel_terminals();
            let n = list.len();
            if let Some(i) = list.iter().position(|&t| t == id)
                && n > 1
            {
                let next = if k.shift {
                    (i + n - 1) % n
                } else {
                    (i + 1) % n
                };
                self.show_terminal(list[next]);
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
        const ALWAYS: [Command; 7] = [
            ToggleTerminal,
            NewTerminal,
            NewTerminalTab,
            ShowCommands,
            GotoFile,
            FocusSidebar,
            ToggleConsole,
        ];
        // A terminal tab is in the editor area: switching tabs and groups works there too.
        const IN_TAB: [Command; 10] = [
            NextRecentFile,
            PrevRecentFile,
            NextBuffer,
            PrevBuffer,
            FocusGroup1,
            FocusGroup2,
            FocusGroup3,
            FocusGroup4,
            FocusNextGroup,
            SwitchBuffer,
        ];
        if let Some(cmd) = self.keymap.lookup(k, Ctx::Global)
            && (k.sup || ALWAYS.contains(&cmd) || (in_tab && IN_TAB.contains(&cmd)))
        {
            self.run(cmd);
            return true;
        }
        // Tab numbers (Cmd+1–9 / Alt+1–9) work in a terminal tab as in any tab.
        if in_tab && self.select_tab_key(k) {
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
}
