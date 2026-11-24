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
        // Movement, selection and deletion keys are keymap commands (handled above via
        // run → run_editor → Buffer::apply_motion). Only structural keys remain here.
        let trim_ws = self.config.editor.trim_automatic_white_space;
        let Some(b) = self.buf_mut() else { return };
        let typed: Option<char> = None;
        match k.code {
            KeyCode::Enter => b.each(|b| b.newline(trim_ws)),
            KeyCode::Tab if k.shift => b.each_line(false, |b| b.outdent()),
            KeyCode::Tab => b.each(|b| {
                let lines = b.selection().is_some_and(|(a, c)| a.line != c.line);
                if lines { b.indent() } else { b.insert_indent() }
            }),
            KeyCode::Esc => {
                // Back to one cursor first, then drop the selection.
                if !b.single_selection() {
                    b.anchor = None;
                }
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
                // Auto-trigger after a few identifier characters.
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
        b.each(|b| b.insert_char(c));
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
        if !extend {
            b.single_selection();
        }
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

    /// Cmd+Click: add a cursor there (or remove the one clicked); Cmd+double-click adds
    /// the word.
    pub fn editor_add_caret(&mut self, pos: Pos, clicks: u32) {
        self.focus = Focus::Editor;
        self.completion = None;
        self.hover = None;
        let Some(b) = self.buf_mut() else { return };
        let pos = b.clamp(pos);
        if clicks >= 2 {
            // The first click of the double-click already added a caret here.
            if let Some((s, e)) = b.word_bounds(pos) {
                b.anchor = Some(s);
                b.cursor = e;
            }
        } else {
            b.toggle_caret(pos);
        }
    }

    /// Alt+drag: a selection per line between two (line, display column) points.
    pub fn editor_column(&mut self, from: (usize, usize), to: (usize, usize)) {
        self.focus = Focus::Editor;
        self.completion = None;
        self.hover = None;
        let Some(b) = self.buf_mut() else { return };
        let n = b.lines.len().saturating_sub(1);
        let (fl, tl) = (from.0.min(n), to.0.min(n));
        b.column_select(Pos::new(fl, 0), Pos::new(tl, 0), from.1, to.1);
    }

    /// Dragging the mouse after a press: `unit` 1 extends by characters, 2 by words, 3 by
    /// lines, always keeping what the press selected (`first`, the word or line).
    pub fn editor_drag(&mut self, pos: Pos, unit: u32, first: (Pos, Pos)) {
        let Some(b) = self.buf_mut() else { return };
        let pos = b.clamp(pos);
        let (s0, e0) = first;
        if unit == 1 {
            b.set_cursor(pos, true);
        } else {
            let (s, e) = if unit == 2 {
                b.word_bounds(pos).unwrap_or((pos, pos))
            } else if pos.line + 1 < b.lines.len() {
                (Pos::new(pos.line, 0), Pos::new(pos.line + 1, 0))
            } else {
                (Pos::new(pos.line, 0), b.last_pos())
            };
            let (anchor, cursor) = if s < s0 { (e0, s) } else { (s0, e.max(e0)) };
            b.set_cursor(anchor, false);
            b.set_cursor(cursor, true);
        }
        b.want_col = b.cursor.col;
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
        let wrap = self.wrap_cols;
        if let Some(b) = self.buf_mut()
            && b.apply_motion(cmd, page, wrap)
        {
            return self.after_editor_key(None);
        }
        match cmd {
            Save => return self.save_current(),
            SaveAs => {
                let id = self.buffers[self.cur].id;
                self.file_request = Some(FileRequest::SaveAs(id));
                return;
            }
            CloseBuffer => {
                let b = &self.buffers[self.cur];
                if self.terminal_running(self.cur) {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!(
                            "The terminal {} is still running. Kill it and close the tab?",
                            b.name()
                        ),
                        kind: ConfirmKind::CloseBuffer(b.id),
                    }));
                } else if b.deleted {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!(
                            "{} no longer exists on disk. Close it and lose its contents? (Save recreates the file.)",
                            b.name()
                        ),
                        kind: ConfirmKind::CloseBuffer(b.id),
                    }));
                } else if b.modified {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!("{} has unsaved changes. Close anyway?", b.name()),
                        kind: ConfirmKind::CloseBuffer(b.id),
                    }));
                } else {
                    self.close_buffer_at(self.cur);
                }
                return;
            }
            Copy | Cut if self.buffers[self.cur].multi() => {
                // Several cursors: each selection on its own line (whole lines where
                // nothing is selected).
                let b = &mut self.buffers[self.cur];
                let sels = b.sels();
                let line = sels.iter().all(|s| s.is_empty());
                let parts = b.selected_texts();
                let text = if line {
                    let mut seen = vec![];
                    sels.iter()
                        .filter(|s| {
                            let new = !seen.contains(&s.cursor.line);
                            seen.push(s.cursor.line);
                            new
                        })
                        .map(|s| format!("{}\n", b.lines[s.cursor.line]))
                        .collect::<String>()
                } else {
                    b.selected_texts().join("\n")
                };
                if cmd == Cut && !b.read_only {
                    if line {
                        b.each_line(false, |b| b.delete_line());
                    } else {
                        b.each(|b| b.insert_text(""));
                    }
                }
                self.clip_parts = (!line).then_some(parts);
                self.pending_copy = Some(text.clone());
                self.clip = text;
                self.clip_line = line;
                return;
            }
            Copy | Cut => {
                self.clip_parts = None;
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
            Completion => return self.request_completion(),
            CodeActions => return self.request_code_actions(),
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
                if !self.request_format(false) {
                    self.msg("No language server for this file");
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
            SelectLine => b.each(|b| b.select_line()),
            SelectWord => {
                if !b.add_next_occurrence() && b.selection().is_some() {
                    self.msg("No more occurrences");
                }
            }
            SelectAllOccurrences => {
                if !b.select_all_occurrences() {
                    self.msg("Nothing to select: put the cursor on a word or select text");
                }
            }
            SplitIntoLines => b.split_into_lines(),
            AddCursorAbove => b.add_cursor_line(true),
            AddCursorBelow => b.add_cursor_line(false),
            DeleteLine => b.each_line(false, |b| b.delete_line()),
            DuplicateLine => b.each_line(false, |b| b.duplicate()),
            MoveLineUp => b.each_line(false, |b| b.move_lines(true)),
            MoveLineDown => b.each_line(true, |b| b.move_lines(false)),
            ToggleComment => {
                let mut ok = true;
                b.each_line(false, |b| ok &= b.toggle_comment());
                if !ok {
                    self.msg("No comment syntax known for this file type");
                }
            }
            Indent => b.each_line(false, |b| b.indent()),
            Outdent => b.each_line(false, |b| b.outdent()),
            InsertLineAfter => b.each_line(false, |b| b.insert_line_after(false)),
            InsertLineBefore => b.each_line(false, |b| b.insert_line_after(true)),
            JoinLines => b.each_line(false, |b| b.join_lines()),
            UpperCase => b.each(|b| b.transform_selection(|s| s.to_uppercase())),
            LowerCase => b.each(|b| b.transform_selection(|s| s.to_lowercase())),
            SortLines => b.sort_lines(),
            _ => {}
        }
        let _ = page;
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
