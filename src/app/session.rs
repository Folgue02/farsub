//! App: projects & workspaces.

use super::*;

impl App {
    // ===================================================================== projects & workspaces

    /// Merge project `[[languages]]`/`[[servers]]` and `[settings]` over the loaded config.
    pub(super) fn apply_project_config(&mut self) {
        let mut errs = vec![];
        if let Some(snippet) = self.project.config_snippet()
            && let Err(e) = self.config.merge_str(&snippet)
        {
            errs.push(e);
        }
        if !self.project.settings.is_empty() {
            match project::apply_settings(&self.config.editor, &self.project.settings) {
                Ok(ed) => self.config.editor = ed,
                Err(e) => errs.push(e),
            }
        }
        self.font_size = self.config.editor.font_size;
        if !errs.is_empty() {
            self.msg(format!("Project settings: {}", errs.join("; ")));
        }
    }

    pub(super) fn open_project_file(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(p) => self.switch_project(p),
            Err(e) => self.msg(format!("Open project failed: {e}")),
        }
    }

    /// Save the current session, close everything, and open another project's session.
    pub fn switch_project(&mut self, p: Project) {
        self.save_workspace();
        for i in 0..self.buffers.len() {
            self.close_lsp_doc(i);
        }
        self.lsp.restart();
        self.buffers.clear();
        self.cur = 0;
        self.group_active = [None; MAX_GROUPS];
        self.diagnostics.clear();
        self.completion = None;
        self.hover = None;
        self.modal = None;
        self.files.clear();
        self.closed.clear();
        self.jumps.clear();
        self.tree.expanded.clear();
        self.tree_revealed = None;
        self.project = p;
        // Reload config so the previous project's settings don't leak into this one.
        let data = self.config.data_dir.clone();
        let path = self.config.path.clone();
        self.config = Config::load_from(path);
        if data.is_some() {
            self.config.data_dir = data;
        }
        self.apply_project_config();
        let _ = self.rebuild_keys_and_menus();
        self.syn.set_theme(&self.config.editor.theme);
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        self.restore_workspace();
        self.recent.touch(&self.project);
        self.recent.save(self.config.data_dir.as_deref());
        self.msg(format!("Opened {}", self.project.name()));
    }

    /// Folders were added/removed: update the sidebar and the project file (if any).
    pub(super) fn project_folders_changed(&mut self) {
        if let Some(f) = self.project.file.clone()
            && let Err(e) = self.project.save_as(&f)
        {
            self.msg(format!("Couldn't update {}: {e}", f.display()));
        }
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        self.files.clear();
        self.sidebar_visible = true;
    }

    /// The project file was edited in farsub: apply folders and settings in place.
    pub(super) fn reload_project_file(&mut self) {
        let Some(f) = self.project.file.clone() else {
            return;
        };
        match Project::load(&f) {
            Ok(p) => {
                self.project = p;
                self.reload_config();
                self.tree
                    .set_roots(self.project.folders.clone(), self.show_hidden);
                self.files.clear();
                self.msg("Project reloaded");
            }
            Err(e) => self.msg(format!("Project error: {e}")),
        }
    }

    pub fn capture_workspace(&self) -> Workspace {
        let hot = self.config.editor.hot_exit;
        let buffers = self
            .buffers
            .iter()
            .filter(|b| b.path.is_some() || (hot && b.modified))
            .map(|b| BufferState {
                group: b.group,
                word_wrap: b.word_wrap,
                path: b.path.clone(),
                cursor: (b.cursor.line, b.cursor.col),
                anchor: b.anchor.map(|a| (a.line, a.col)),
                scroll: (b.scroll_y, b.scroll_x),
                unsaved: (hot && b.modified).then(|| b.text()),
                syntax: b.syntax_override.then(|| b.hl.syntax.clone()),
                indent: b
                    .indent_override
                    .then(|| (b.indent_unit == "\t", b.tab_size)),
            })
            .collect();
        let mut expanded: Vec<PathBuf> = self
            .tree
            .expanded
            .iter()
            .filter(|p| !self.tree.is_root(p))
            .cloned()
            .collect();
        expanded.sort();
        let collapsed_roots = self
            .project
            .folders
            .iter()
            .filter(|f| !self.tree.expanded.contains(&f.path));
        let mut ws = Workspace {
            buffers,
            active: self.cur,
            sidebar_visible: self.sidebar_visible,
            show_hidden: self.show_hidden,
            expanded,
            search: self.search.clone(),
            closed: self.closed.iter().rev().take(20).rev().cloned().collect(),
            sidebar_width: Some(self.sidebar_width),
            console_visible: self.console_shown(),
            console_height: Some(self.panel.height),
            layout: self.layout,
            focused_group: self.group,
        };
        // Remember collapsed project folders as "!path" entries.
        ws.expanded
            .extend(collapsed_roots.map(|f| PathBuf::from(format!("!{}", f.path.display()))));
        ws
    }

    pub fn save_workspace(&mut self) {
        self.last_autosave = Instant::now();
        let Some(f) = self.project.workspace_file(self.config.data_dir.as_deref()) else {
            return;
        };
        if let Err(e) = self.capture_workspace().save(&f) {
            self.msg(format!("Couldn't save session: {e}"));
        }
    }

    /// Called by the GUI every frame; saves the session every 30 seconds.
    pub fn autosave_tick(&mut self) {
        if self.last_autosave.elapsed() > Duration::from_secs(30) {
            self.save_workspace();
        }
    }

    pub fn restore_workspace(&mut self) {
        let Some(f) = self.project.workspace_file(self.config.data_dir.as_deref()) else {
            return;
        };
        let Some(ws) = Workspace::load(&f) else {
            return;
        };
        self.set_layout(ws.layout.max(1));
        for bs in &ws.buffers {
            self.group = bs.group.min(self.layout - 1);
            let i = match (&bs.path, &bs.unsaved) {
                (Some(p), None) if p.is_file() => self.open_file(p, false),
                (_, None) => None,
                (path, Some(text)) => {
                    let id = self.next_buf_id;
                    self.next_buf_id += 1;
                    let b = Buffer::from_text(
                        id,
                        path.clone(),
                        text,
                        HlCache::new("Plain Text".into()),
                    );
                    let i = self.push_buffer(b);
                    if let Some(p) = path.clone() {
                        self.set_buffer_path(i, p);
                    }
                    self.buffers[i].modified = true;
                    self.lsp_open(i);
                    Some(i)
                }
            };
            if let Some(i) = i {
                let b = &mut self.buffers[i];
                b.group = bs.group.min(self.layout - 1);
                b.word_wrap = bs.word_wrap;
                b.cursor = b.clamp(Pos::new(bs.cursor.0, bs.cursor.1));
                b.anchor = bs.anchor.map(|a| b.clamp(Pos::new(a.0, a.1)));
                b.want_col = b.cursor.col;
                b.scroll_y = bs.scroll.0.min(b.lines.len().saturating_sub(1));
                b.scroll_x = bs.scroll.1;
                if let Some(sx) = &bs.syntax {
                    self.set_syntax_at(i, sx);
                }
                if let Some((tabs, w)) = bs.indent {
                    let b = &mut self.buffers[i];
                    b.set_tab_width(w);
                    b.set_indent(if tabs {
                        crate::editor::Indent::Tabs
                    } else {
                        crate::editor::Indent::Spaces(w)
                    });
                }
            }
        }
        // Each group shows its first tab, then the active file (and its group) is focused.
        for g in 0..self.layout {
            self.group_active[g] = self.group_tabs(g).first().map(|&i| self.buffers[i].id);
        }
        self.group = ws.focused_group.min(self.layout - 1);
        let active = ws.active.min(self.buffers.len().saturating_sub(1));
        if active < self.buffers.len() {
            self.set_cur(active);
        }
        self.sidebar_visible = ws.sidebar_visible;
        self.show_hidden = ws.show_hidden;
        for e in &ws.expanded {
            let s = e.to_string_lossy();
            if let Some(root) = s.strip_prefix('!') {
                self.tree.expanded.remove(Path::new(root));
            } else if e.is_dir() {
                self.tree.expanded.insert(e.clone());
            }
        }
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        self.tree.rebuild();
        self.search = ws.search.clone();
        self.closed = ws.closed.clone();
        if let Some(w) = ws.sidebar_width {
            self.sidebar_width = w.clamp(120.0, 900.0);
        }
        if let Some(h) = ws.console_height {
            self.panel.height = h.clamp(80.0, 1200.0);
        }
        if ws.console_visible {
            self.panel.visible = true;
            self.panel.tab = PanelTab::Console;
        }
    }
}
