//! App: sidebar.

use super::*;

impl App {
    // ===================================================================== sidebar

    pub(super) fn focus_sidebar(&mut self) {
        self.sidebar_visible = true;
        self.focus = Focus::Sidebar;
        self.completion = None;
        self.hover = None;
        self.tree
            .set_roots(self.project.folders.clone(), self.show_hidden);
        let cur = self.buf().and_then(|b| b.path.clone());
        if let Some(p) = cur
            && self.tree_revealed.as_ref() != Some(&p)
        {
            self.tree.reveal(&p);
            self.tree_revealed = Some(p);
        }
    }

    pub fn tree_click(&mut self, i: usize, open: bool) {
        self.focus = Focus::Sidebar;
        if i < self.tree.nodes.len() {
            self.tree.sel = i;
            if open {
                self.tree_open(false);
            }
        }
    }

    pub(super) fn tree_open(&mut self, keep_focus: bool) {
        let Some(n) = self.tree.selected() else {
            return;
        };
        if n.is_dir {
            self.tree.toggle();
            return;
        }
        let path = n.path.clone();
        if self.open_file(&path, true).is_some() {
            self.tree_revealed = Some(path);
            if keep_focus {
                self.focus = Focus::Sidebar;
            }
        }
    }

    /// Single-letter sidebar commands (arrive as text so Shift+A gives 'A').
    pub(super) fn tree_char(&mut self, c: char) {
        if let Some(cmd) = self.keymap.lookup(keymap::key_for_char(c), Ctx::Sidebar) {
            self.run(cmd);
        }
    }

    pub(super) fn sidebar_nav(&mut self, cmd: Command) {
        use Command::*;
        let page = self.page.max(1) as isize;
        match cmd {
            SidebarUp => self.tree.move_sel(-1),
            SidebarDown => self.tree.move_sel(1),
            SidebarPageUp => self.tree.move_sel(-page),
            SidebarPageDown => self.tree.move_sel(page),
            SidebarTop => self.tree.sel = 0,
            SidebarBottom => self.tree.sel = self.tree.nodes.len().saturating_sub(1),
            SidebarExpand => self.tree.expand(),
            SidebarCollapse => self.tree.collapse(),
            SidebarOpen => self.tree_open(false),
            SidebarOpenKeepFocus => self.tree_open(true),
            SidebarMark => {
                self.tree.toggle_mark();
                self.tree.move_sel(1);
            }
            SidebarMarkUp | SidebarMarkDown => {
                if self.tree.marked.is_empty() {
                    self.tree.toggle_mark();
                }
                self.tree
                    .move_sel(if cmd == SidebarMarkUp { -1 } else { 1 });
                if let Some(p) = self.tree.selected_path().map(Path::to_path_buf) {
                    self.tree.marked.insert(p);
                }
            }
            SidebarEscape => {
                if !self.tree.marked.is_empty() {
                    self.tree.marked.clear();
                } else if self.file_clip.is_some() {
                    self.file_clip = None;
                } else {
                    self.focus = Focus::Editor;
                }
            }
            _ => {}
        }
    }

    pub(super) fn run_sidebar(&mut self, cmd: Command) {
        use Command::*;
        let dir = self.tree.target_dir();
        let rel_dir = |app: &App| {
            let r = app.rel(&dir);
            if r.is_empty() {
                "./".to_string()
            } else {
                format!("{r}/")
            }
        };
        match cmd {
            SidebarNewFile => {
                let t = format!(
                    "New file in {}  (use a/b.txt for subfolders)",
                    rel_dir(self)
                );
                self.prompt(&t, "", PromptKind::NewFile(dir.clone()));
            }
            SidebarNewFolder => {
                let t = format!("New folder in {}", rel_dir(self));
                self.prompt(&t, "", PromptKind::NewFolder(dir.clone()));
            }
            SidebarRename => {
                let Some(src) = self.tree.selected_path().map(Path::to_path_buf) else {
                    return;
                };
                if self.tree.is_root(&src) {
                    return self.msg("That's a project folder — use Project › Remove Folder, or rename it on disk");
                }
                let base = self
                    .tree
                    .root_of(&src)
                    .map(|f| f.path.clone())
                    .unwrap_or_else(|| self.base_dir());
                let rel = src
                    .strip_prefix(&base)
                    .unwrap_or(&src)
                    .display()
                    .to_string();
                let title = format!(
                    "Rename / move to (relative to {})",
                    base.file_name().unwrap_or_default().to_string_lossy()
                );
                self.prompt(&title, &rel, PromptKind::RenameMove(src, base));
                // Put the cursor before the extension, like VS Code.
                if let Some(Modal::Prompt(p)) = &mut self.modal
                    && let Some(dot) = p.input.text().rfind('.')
                    && p.input.text().rfind('/').is_none_or(|s| s < dot)
                    && dot > 0
                {
                    let col = char_len(&p.input.text()[..dot]);
                    p.input.set_cursor(col);
                }
            }
            SidebarDelete => {
                let t = self.tree.targets();
                if t.is_empty() {
                    return;
                }
                let what = if t.len() == 1 {
                    self.rel(&t[0])
                } else {
                    format!("{} items", t.len())
                };
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Move {what} to the Trash?"),
                    kind: ConfirmKind::Trash(t),
                }));
            }
            SidebarCopy | SidebarCut => {
                let t = self.tree.targets();
                if t.is_empty() {
                    return;
                }
                let cut = cmd == SidebarCut;
                self.msg(format!(
                    "{} {} item(s) — select a folder and press p to paste",
                    if cut { "Cut" } else { "Copied" },
                    t.len()
                ));
                self.file_clip = Some((t, cut));
                self.tree.marked.clear();
            }
            SidebarPaste => {
                let Some((srcs, cut)) = self.file_clip.clone() else {
                    return self.msg("Nothing to paste — copy (c) or cut (x) files first");
                };
                let mut last = None;
                let mut n = 0;
                for src in &srcs {
                    let name = src
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if cut && src.parent() == Some(dir.as_path()) {
                        continue;
                    }
                    let dst = fileops::unique_dest(&dir, &name);
                    let res = if cut {
                        fileops::move_path(src, &dst).map(|_| self.rebind_paths(src, &dst))
                    } else {
                        fileops::copy_recursive(src, &dst)
                    };
                    match res {
                        Ok(()) => {
                            n += 1;
                            last = Some(dst);
                        }
                        Err(e) => self.msg(format!("{}: {e}", self.rel(src))),
                    }
                }
                if cut {
                    self.file_clip = None;
                }
                self.tree.expand_path(&dir);
                self.tree.expanded.insert(dir.clone());
                self.refresh_tree();
                if let Some(p) = last {
                    self.tree.reveal(&p);
                }
                self.msg(format!(
                    "{} {n} item(s) into {}",
                    if cut { "Moved" } else { "Pasted" },
                    rel_dir(self)
                ));
            }
            SidebarDuplicate => {
                let t = self.tree.targets();
                let mut last = None;
                for src in &t {
                    let parent = src
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.base_dir());
                    let name = src
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let dst = fileops::unique_dest(&parent, &name);
                    match fileops::copy_recursive(src, &dst) {
                        Ok(()) => last = Some(dst),
                        Err(e) => self.msg(format!("Duplicate failed: {e}")),
                    }
                }
                self.tree.marked.clear();
                self.refresh_tree();
                if let Some(p) = last {
                    self.tree.reveal(&p);
                }
            }
            SidebarCopyPath => {
                let t = self.tree.targets();
                let s = t
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !s.is_empty() {
                    self.msg(format!("Copied path: {s}"));
                    self.pending_copy = Some(s);
                }
            }
            _ => {}
        }
    }

    /// Point open buffers at a file's new location after a rename/move (also for folders).
    pub(super) fn rebind_paths(&mut self, src: &Path, dst: &Path) {
        for i in 0..self.buffers.len() {
            let Some(p) = self.buffers[i].path.clone() else {
                continue;
            };
            if let Ok(rest) = p.strip_prefix(src) {
                let np = if rest.as_os_str().is_empty() {
                    dst.to_path_buf()
                } else {
                    dst.join(rest)
                };
                self.close_lsp_doc(i);
                self.set_buffer_path(i, np);
                self.lsp_open(i);
            }
        }
        if let Some(r) = &self.tree_revealed
            && r.starts_with(src)
        {
            self.tree_revealed = None;
        }
    }

    pub(super) fn set_buffer_path(&mut self, i: usize, path: PathBuf) {
        let first = self.buffers[i].lines[0].clone();
        let lang = self.config.language_for(&path, &first).cloned();
        let (syntax, grammar) = match &lang {
            Some(l) => (l.name.clone(), self.grammar_of(l)),
            None => ("Plain Text".to_string(), None),
        };
        if grammar.is_none()
            && let Some(l) = &lang
        {
            self.hint_grammar(l);
        }
        let b = &mut self.buffers[i];
        b.set_path(path);
        b.apply_language(lang.as_ref(), &self.config.editor);
        b.hl.reset(syntax, grammar);
    }
}
