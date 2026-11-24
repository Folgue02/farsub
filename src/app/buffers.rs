//! App: files & buffers.

use super::*;

impl App {
    // ===================================================================== files & buffers

    pub fn open_file(&mut self, path: &Path, focus: bool) -> Option<usize> {
        let path = crate::paths::canonical(&path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(i) = self
            .buffers
            .iter()
            .position(|b| b.path.as_deref() == Some(&path))
        {
            if focus {
                self.set_cur(i);
                self.focus = Focus::Editor;
                self.reveal_opened(&path);
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
        self.set_buffer_path(i, path.clone());
        if std::fs::metadata(&self.buffers[i].path.clone().unwrap())
            .is_ok_and(|m| m.permissions().readonly())
        {
            self.buffers[i].read_only = true;
        }
        if [self.defaults_settings_path(), self.defaults_keymap_path()]
            .iter()
            .any(|d| self.buffers[i].path.as_deref() == Some(d.as_path()))
        {
            self.buffers[i].read_only = true;
        }
        if focus {
            self.set_cur(i);
            self.focus = Focus::Editor;
            self.reveal_opened(&path);
        }
        self.lsp_open(i);
        Some(i)
    }

    /// Select an opened file in the sidebar and scroll to it (`reveal_opened_file`).
    fn reveal_opened(&mut self, path: &Path) {
        if self.config.editor.reveal_opened_file {
            self.tree.reveal(path);
            self.tree_revealed = Some(path.to_path_buf());
        }
    }

    /// Arguments for a new, empty window (same config and data folders as this one).
    pub fn new_window_args(&self) -> Vec<std::ffi::OsString> {
        self.window_args("--empty")
    }

    /// Arguments for another FarSub window of this setup (`--empty` or `--settings`).
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

    /// File › New Window: start another FarSub with a clean window (its own process).
    pub fn open_new_window(&mut self) {
        let args = self.new_window_args();
        self.spawn_window(args);
    }

    /// Start another FarSub process with these arguments, detached from this one.
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

    /// Look at the open files on disk in a background thread (a slow network drive mustn't
    /// freeze the window); `on_disk_states` handles what it finds. One check at a time.
    pub(super) fn start_disk_check(&mut self) {
        if self.disk_check_running {
            return;
        }
        let reqs: Vec<DiskRequest> = (self.buffers.iter())
            .filter_map(|b| {
                Some(DiskRequest {
                    id: b.id,
                    path: b.path.clone()?,
                    seen: b.disk,
                    deleted: b.deleted,
                })
            })
            .collect();
        if reqs.is_empty() {
            return;
        }
        self.disk_check_running = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let states = reqs.into_iter().map(read_disk_state).collect();
            let _ = tx.send(AppEvent::Disk(states));
        });
    }

    /// Open files that were deleted on disk (or came back): a deleted file
    /// counts as unsaved, so closing asks first and hot exit keeps its contents. Files
    /// another program changed: unedited ones reload in place (an undoable edit);
    /// one with unsaved edits asks first.
    pub(super) fn on_disk_states(&mut self, states: Vec<DiskState>) {
        self.disk_check_running = false;
        let mut deleted_changed = false;
        let mut gone = vec![];
        for st in states {
            let Some(i) = self.buf_index(st.req.id) else {
                continue;
            };
            let b = &mut self.buffers[i];
            // Saved, renamed or re-read since the check started: what it saw is old.
            if b.path.as_ref() != Some(&st.req.path)
                || b.disk != st.req.seen
                || b.deleted != st.req.deleted
            {
                continue;
            }
            if !st.exists && !b.deleted {
                b.deleted = true;
                b.modified = true;
                gone.push(b.name());
                deleted_changed = true;
                continue;
            }
            if st.exists && b.deleted {
                b.deleted = false;
                deleted_changed = true;
            }
            self.disk_changed(i, &st);
        }
        if deleted_changed {
            // The sidebar stops listing it.
            self.refresh_tree();
        }
        for name in gone {
            crate::log::info("files", format!("{name} was deleted on disk"));
            self.msg(format!(
                "{name} no longer exists on disk — save to recreate it"
            ));
        }
    }

    /// The file changed on disk (another program wrote it).
    fn disk_changed(&mut self, i: usize, st: &DiskState) {
        let (Some(seen), Some(now)) = (st.req.seen, st.now) else {
            return;
        };
        if now == seen {
            return;
        }
        let Some((text, enc)) = (st.bytes.as_deref()).and_then(crate::encoding::decode) else {
            self.buffers[i].disk = Some(now);
            return;
        };
        let b = &mut self.buffers[i];
        let norm = text.replace("\r\n", "\n");
        if norm == b.lines.join("\n") {
            // Only touched (or saved the same text): nothing to do.
            b.disk = Some(now);
            return;
        }
        if !b.modified {
            b.encoding = enc;
            b.reload_from(&text);
            let name = b.name();
            crate::log::info("files", format!("{name} changed on disk: reloaded"));
            self.msg(format!(
                "{name} changed on disk — reloaded (undo to get yours back)"
            ));
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
        }
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

    /// A Plain Text file (no language from its name) gets one as soon as its first line is
    /// a shebang that names one: typing `#!/usr/bin/env python3` into a new `run` file
    /// highlights it as Python. A language picked by hand stays.
    pub(super) fn redetect_shebang(&mut self) {
        let i = self.cur;
        let Some(b) = self.buffers.get(i) else { return };
        let key = Some((b.id, b.version));
        if self.shebang_seen == key {
            return;
        }
        self.shebang_seen = key;
        if b.hl.syntax != "Plain Text" || b.syntax_override || !b.lines[0].starts_with("#!") {
            return;
        }
        let first = b.lines[0].clone();
        let path = b.path.clone();
        let Some(lang) = (self.config)
            .language_for(path.as_deref().unwrap_or(Path::new("")), &first)
            .cloned()
        else {
            return;
        };
        match path {
            Some(p) => {
                self.set_buffer_path(i, p);
                self.lsp_open(i);
            }
            None => {
                let grammar = self.grammar_of(&lang);
                if grammar.is_none() {
                    self.hint_grammar(&lang);
                }
                let b = &mut self.buffers[i];
                b.apply_language(Some(&lang), &self.config.editor);
                b.hl.reset(lang.name.clone(), grammar);
            }
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
        // Format on save: ask the server first; the file is saved when its edits arrive.
        // Saving again while waiting saves right away (a server that never answers).
        let id = b.id;
        if self.config.editor.format_on_save
            && !self.format_saving.remove(&id)
            && self.request_format(true)
        {
            self.format_saving.insert(id);
            return self.msg("Formatting before saving…");
        }
        let Some(b) = self.buf_mut() else { return };
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
                self.wakatime_saved();
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
        // Closing a terminal tab kills its shell (Move Terminal to Panel takes `term`
        // out first to keep it).
        if let Some(t) = b.term {
            self.terminals.retain(|x| x.id != t);
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
        // a settings window): the other side goes too unless it has unsaved
        // changes, and the previous layout comes back.
        if let Some(k) = (self.settings_view.iter())
            .position(|v| v.defaults_id == b.id || v.user_id == b.id)
        {
            let v = self.settings_view.remove(k);
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
            if !self.settings_view.is_empty() {
                return;
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

/// What the background disk check looks at for one open file.
pub struct DiskRequest {
    pub id: usize,
    pub path: PathBuf,
    /// The buffer's last known stamp, and whether it was deleted, when the check started.
    pub seen: Option<fileops::DiskStamp>,
    pub deleted: bool,
}

/// What it found.
pub struct DiskState {
    pub req: DiskRequest,
    pub exists: bool,
    pub now: Option<fileops::DiskStamp>,
    /// The new contents, read only when the file changed.
    pub bytes: Option<Vec<u8>>,
}

fn read_disk_state(req: DiskRequest) -> DiskState {
    let now = fileops::DiskStamp::of(&req.path);
    let exists = now.is_some() || req.path.exists();
    let changed = req.seen.is_some() && now.is_some() && now != req.seen;
    let bytes = if changed { std::fs::read(&req.path).ok() } else { None };
    DiskState {
        req,
        exists,
        now,
        bytes,
    }
}
