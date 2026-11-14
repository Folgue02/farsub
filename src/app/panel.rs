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
    pub(super) fn terminal_key(&mut self, k: Key) -> bool {
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
}
