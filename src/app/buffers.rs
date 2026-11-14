//! App: files & buffers.

use super::*;

impl App {
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
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        // set_buffer_path picks the language and grammar.
        let b = Buffer::from_text(
            id,
            Some(path.clone()),
            &text,
            HlCache::new("Plain Text".into()),
        );
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
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let b = Buffer::from_text(
            id,
            Some(path.clone()),
            "",
            HlCache::new("Plain Text".into()),
        );
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
        let (syntax, grammar) = self.syntax_for(None, first);
        let mut b = Buffer::from_text(id, None, text, HlCache::new(String::new()));
        b.hl.reset(syntax, grammar);
        b.modified = !text.is_empty();
        let i = self.push_buffer(b);
        self.set_cur(i);
        self.focus = Focus::Editor;
    }

    pub(super) fn goto_location(&mut self, path: &Path, pos: Option<Pos>) {
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

    pub(super) fn push_jump(&mut self) {
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

    pub(super) fn save_current(&mut self) {
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

    pub(super) fn close_buffer_at(&mut self, i: usize) {
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
}
