//! App: editing — keys, typing, mouse, editor commands, find, diagnostics.

use super::*;

impl App {
    // ===================================================================== editor

    pub(super) fn editor_key_inner(&mut self, k: Key) {
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

    pub(super) fn after_editor_key(&mut self, typed: Option<char>) {
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

    pub(super) fn editor_text(&mut self, c: char) {
        let Some(b) = self.buf_mut() else { return };
        if b.read_only {
            return;
        }
        b.insert_char(c);
        self.hover = None;
        self.after_editor_key(Some(c));
    }

    pub(super) fn editor_key(&mut self, k: Key) -> bool {
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

    pub(super) fn run_editor(&mut self, cmd: Command) {
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

    pub(super) fn find_next(&mut self, forward: bool) {
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

    pub(super) fn jump_diagnostic(&mut self, forward: bool) {
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
}
