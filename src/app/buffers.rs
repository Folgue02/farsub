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
        let Some((text, encoding)) = crate::encoding::decode(&bytes) else {
            self.msg("Binary file — not opening");
            return None;
        };
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
        self.buffers[i].encoding = encoding;
        self.buffers[i].disk = crate::fileops::DiskStamp::of(&path);
        if !encoding.is_utf8() {
            self.msg(format!("Opened as {}", encoding.label()));
        }
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

    /// Arguments for a new, empty window (same config and data folders as this one).
    pub fn new_window_args(&self) -> Vec<std::ffi::OsString> {
        self.window_args("--empty")
    }

    /// Arguments for another farsub window of this setup (`--empty` or `--settings`).
    pub fn window_args(&self, kind: &str) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = vec![kind.into()];
        if let Some(c) = &self.config.path {
            args.push("--config".into());
            args.push(c.into());
        }
        if let Some(d) = &self.config.data_dir {
            args.push("--data-dir".into());
            args.push(d.into());
        }
        args
    }

    /// File › New Window: start another farsub with a clean window (its own process).
    pub fn open_new_window(&mut self) {
        let args = self.new_window_args();
        self.spawn_window(args);
    }

    /// Start another farsub process with these arguments, detached from this one.
    pub(super) fn spawn_window(&mut self, args: Vec<std::ffi::OsString>) {
        #[cfg(test)]
        {
            self.spawned.push(args);
            return;
        }
        #[cfg(not(test))]
        self.spawn_detached(args);
    }

    #[cfg(not(test))]
    fn spawn_detached(&mut self, args: Vec<std::ffi::OsString>) {
        let exe = std::env::current_exe().unwrap_or_else(|_| "farsub".into());
        let mut cmd = std::process::Command::new(exe);
        cmd.args(args)
            .env("FARSUB_DETACHED", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        if let Err(e) = cmd.spawn() {
            self.msg(format!("Couldn't open a new window: {e}"));
        }
    }

    /// Notice open files that were deleted on disk (or came back), like Sublime: a deleted
    /// file counts as unsaved, so closing asks first and hot exit keeps its contents.
    /// Returns true if anything changed.
    pub fn check_deleted_files(&mut self) -> bool {
        let mut changed = false;
        let mut gone = vec![];
        for b in &mut self.buffers {
            let Some(path) = &b.path else { continue };
            let exists = path.exists();
            if !exists && !b.deleted {
                b.deleted = true;
                b.modified = true;
                gone.push(b.name());
                changed = true;
            } else if exists && b.deleted {
                b.deleted = false;
                changed = true;
            }
        }
        if changed {
            // Like Sublime: the sidebar stops listing it.
            self.refresh_tree();
        }
        changed |= self.check_changed_files();
        for name in gone {
            crate::log::info("files", format!("{name} was deleted on disk"));
            self.msg(format!(
                "{name} no longer exists on disk — save to recreate it"
            ));
        }
        changed
    }

    /// Notice open files that another program changed. Unedited ones reload in place (an
    /// undoable edit, like Sublime); one with unsaved edits asks first. Returns true if
    /// anything changed.
    pub fn check_changed_files(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.buffers.len() {
            let b = &self.buffers[i];
            let (Some(path), Some(seen)) = (b.path.clone(), b.disk) else {
                continue;
            };
            if b.deleted {
                continue;
            }
            let Some(now) = crate::fileops::DiskStamp::of(&path) else {
                continue;
            };
            if now == seen {
                continue;
            }
            let Some((text, enc)) = std::fs::read(&path)
                .ok()
                .and_then(|bytes| crate::encoding::decode(&bytes))
            else {
                self.buffers[i].disk = Some(now);
                continue;
            };
            let b = &mut self.buffers[i];
            let norm = text.replace("\r\n", "\n");
            if norm == b.lines.join("\n") {
                // Only touched (or saved the same text): nothing to do.
                b.disk = Some(now);
                continue;
            }
            if !b.modified {
                b.encoding = enc;
                b.reload_from(&text);
                let name = b.name();
                crate::log::info("files", format!("{name} changed on disk: reloaded"));
                self.msg(format!(
                    "{name} changed on disk — reloaded (undo to get yours back)"
                ));
                changed = true;
            } else if self.modal.is_none() {
                // Asked once per change: "no" keeps the edits (saving then overwrites).
                b.disk = Some(now);
                let name = b.name();
                let id = b.id;
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!(
                        "{name} changed on disk and has unsaved changes here. Reload it and lose them? (y/n)"
                    ),
                    kind: ConfirmKind::Reload(id),
                }));
                changed = true;
            }
        }
        changed
    }

    /// Reload a buffer from disk (its unsaved edits go, but stay undoable).
    pub(super) fn reload_buffer(&mut self, id: usize) {
        let Some(b) = self.buffers.iter_mut().find(|b| b.id == id) else {
            return;
        };
        let Some(path) = b.path.clone() else { return };
        match std::fs::read(&path) {
            Ok(bytes) => match crate::encoding::decode(&bytes) {
                Some((text, enc)) => {
                    b.encoding = enc;
                    b.reload_from(&text);
                    let name = b.name();
                    self.msg(format!("Reloaded {name}"));
                }
                None => self.msg("It's a binary file now — not reloading"),
            },
            Err(e) => self.msg(format!("reload {}: {e}", path.display())),
        }
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
            let id = b.id;
            self.file_request = Some(FileRequest::SaveAs(id));
            return;
        }
        // Changed on disk since we read it (and not asked about yet): don't clobber it
        // silently.
        if !b.deleted
            && let Some(path) = &b.path
            && let Some(now) = crate::fileops::DiskStamp::of(path)
            && b.disk.is_some_and(|seen| seen != now)
        {
            let name = b.name();
            let id = b.id;
            self.modal = Some(Modal::Confirm(Confirm {
                message: format!(
                    "{name} changed on disk since it was opened. Overwrite it with this version? (y/n)"
                ),
                kind: ConfirmKind::Overwrite(id),
            }));
            return;
        }
        if b.deleted
            && let Some(dir) = b.path.as_ref().and_then(|p| p.parent())
        {
            // Saving a deleted file recreates it, folder included.
            let _ = std::fs::create_dir_all(dir);
        }
        match b.save() {
            Ok(()) => {
                b.deleted = false;
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
            // The settings window is only for the settings: closing them closes it.
            if self.settings_window && self.buffers.iter().all(|b| !b.modified) {
                self.quit = true;
            }
        }
    }
}
