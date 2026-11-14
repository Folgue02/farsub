//! App: modal.

use super::*;

impl App {
    // ===================================================================== modal

    pub(super) fn prompt(&mut self, title: &str, def: &str, kind: PromptKind) {
        self.completion = None;
        self.hover = None;
        self.modal = Some(Modal::Prompt(Prompt {
            title: title.to_string(),
            input: LineEdit::new(def),
            kind,
        }));
    }

    pub(super) fn modal_key(&mut self, k: Key) {
        match self.modal.take() {
            Some(Modal::Palette(p)) => {
                self.modal = Some(Modal::Palette(p));
                self.palette_key(k);
            }
            Some(Modal::Prompt(p)) => self.prompt_key(k, p),
            Some(Modal::Keymap(v)) => self.keymap_key(k, v),
            Some(Modal::Servers(v)) => self.servers_key(k, v),
            Some(Modal::LspInfo(v)) => self.lsp_info_key(k, v),
            Some(Modal::Confirm(c)) => match k.code {
                KeyCode::Enter => self.confirm(c.kind),
                KeyCode::Esc => {}
                _ => self.modal = Some(Modal::Confirm(c)),
            },
            None => {}
        }
    }

    pub(super) fn confirm(&mut self, kind: ConfirmKind) {
        match kind {
            ConfirmKind::Trash(paths) => match fileops::trash(&paths) {
                Ok(()) => {
                    self.tree.marked.clear();
                    self.refresh_tree();
                    self.msg(format!("Moved {} item(s) to the Trash", paths.len()));
                }
                Err(e) => self.msg(format!("Move to Trash failed: {e}")),
            },
            ConfirmKind::CloseBuffer(id) => {
                if let Some(i) = self.buf_index(id) {
                    self.close_buffer_at(i);
                }
            }
            ConfirmKind::Quit => self.quit = true,
            ConfirmKind::UninstallServer(name) => {
                let Some(data) = self.config.data_dir.clone() else {
                    return;
                };
                crate::lsp::clear_find_cache();
                match installer::uninstall(&data, &name) {
                    Ok(()) => {
                        self.install_log.remove(&name);
                        self.lsp.reset_server(&name);
                        self.msg(format!("Uninstalled {name}"));
                        self.reopen_lsp_docs();
                    }
                    Err(e) => self.msg(format!("Uninstall failed: {e}")),
                }
                self.modal = Some(Modal::Servers(ServersView {
                    sel: self.servers_sel_of(&name),
                    details: false,
                }));
            }
            ConfirmKind::RemovePackage(name) => {
                let Some(dir) = self.config.packages_dir() else {
                    return;
                };
                match fileops::trash(&[dir.join(&name)]) {
                    Ok(()) => {
                        self.msg(format!("Moved package {name} to the Trash"));
                        self.run(Command::ReloadPackages);
                    }
                    Err(e) => self.msg(format!("Remove failed: {e}")),
                }
            }
        }
    }

    pub(super) fn prompt_key(&mut self, k: Key, mut p: Prompt) {
        let is_find = matches!(p.kind, PromptKind::Find { .. });
        match k.code {
            KeyCode::Esc => {
                if let PromptKind::Find { origin } = p.kind
                    && let Some(b) = self.buf_mut()
                {
                    b.goto(origin);
                }
                return;
            }
            KeyCode::Enter if is_find => {
                // Sublime: Enter = next match, Shift+Enter = previous; the panel stays open.
                self.search = Some(p.input.text().to_string());
                self.find_next(!k.shift);
            }
            KeyCode::Enter => return self.prompt_accept(p),
            KeyCode::Tab if p.kind.is_path() => {
                let base = match &p.kind {
                    PromptKind::NewFile(d) | PromptKind::NewFolder(d) => d.clone(),
                    PromptKind::RenameMove(_, base) => base.clone(),
                    _ => self.base_dir(),
                };
                let done = complete_path(&base, p.input.text());
                p.input.set(&done);
            }
            KeyCode::F(3) | KeyCode::Down if is_find => {
                self.search = Some(p.input.text().to_string());
                self.find_next(!k.shift);
            }
            KeyCode::Up if is_find => {
                self.search = Some(p.input.text().to_string());
                self.find_next(false);
            }
            _ => {}
        }
        self.modal = Some(Modal::Prompt(p));
    }

    /// Incremental find while typing in the find panel.
    pub(super) fn prompt_changed(&mut self) {
        let Some(Modal::Prompt(p)) = &self.modal else {
            return;
        };
        let PromptKind::Find { origin } = p.kind else {
            return;
        };
        let q = p.input.text().to_string();
        self.search = Some(q.clone());
        let pg = self.page;
        if let Some(b) = self.buf_mut() {
            if let Some((s, e)) = b.find(&q, origin, true, false) {
                b.anchor = Some(s);
                b.cursor = e;
                if b.cursor.line < b.scroll_y || b.cursor.line >= b.scroll_y + pg {
                    center(b, pg);
                }
            } else {
                b.goto(origin);
            }
        }
    }

    pub(super) fn prompt_accept(&mut self, p: Prompt) {
        let input = p.input.text().trim().to_string();
        match p.kind {
            PromptKind::NewFile(dir) | PromptKind::NewFolder(dir) if input.is_empty() => {
                let _ = dir;
            }
            PromptKind::NewFile(dir) => {
                let path = self.resolve(&dir, &input);
                if path.exists() {
                    return self.msg(format!("{} already exists", self.rel(&path)));
                }
                if let Some(parent) = path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    return self.msg(format!("Create folder failed: {e}"));
                }
                if let Err(e) = std::fs::write(&path, "") {
                    return self.msg(format!("Create file failed: {e}"));
                }
                self.refresh_tree();
                self.tree.reveal(&path);
                self.open_file(&path, true);
            }
            PromptKind::NewFolder(dir) => {
                let path = self.resolve(&dir, &input);
                match std::fs::create_dir_all(&path) {
                    Ok(()) => {
                        self.refresh_tree();
                        self.tree.reveal(&path);
                        self.tree.expanded.insert(path);
                        self.tree.rebuild();
                    }
                    Err(e) => self.msg(format!("Create folder failed: {e}")),
                }
            }
            PromptKind::RenameMove(src, base) => {
                if input.is_empty() {
                    return;
                }
                let dst = self.resolve(&base, &input);
                if dst == src {
                    return;
                }
                if dst.exists() {
                    return self.msg(format!("{} already exists", self.rel(&dst)));
                }
                match fileops::move_path(&src, &dst) {
                    Ok(()) => {
                        self.rebind_paths(&src, &dst);
                        self.refresh_tree();
                        self.tree.reveal(&dst);
                        self.msg(format!("Moved to {}", self.rel(&dst)));
                    }
                    Err(e) => self.msg(format!("Rename failed: {e}")),
                }
            }
            PromptKind::SaveAs => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.base_dir(), &input);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let cur = self.cur;
                self.close_lsp_doc(cur);
                self.set_buffer_path(cur, path);
                self.save_current();
                self.lsp_open(cur);
                self.refresh_tree();
            }
            PromptKind::OpenPath => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.base_dir(), &input);
                let path = path.canonicalize().unwrap_or(path);
                if path.extension().is_some_and(|e| e == project::PROJECT_EXT) {
                    self.open_project_file(&path);
                } else if path.is_dir() {
                    self.switch_project(Project::from_folders(vec![path]));
                } else if path.is_file() {
                    self.open_file(&path, true);
                } else {
                    self.msg(format!("No such file or folder: {input}"));
                }
            }
            PromptKind::Find { .. } => {}
            PromptKind::ReplaceFind => {
                if !input.is_empty() {
                    let f = p.input.text().to_string();
                    self.prompt(
                        &format!("Replace '{f}' with"),
                        "",
                        PromptKind::ReplaceWith(f),
                    );
                }
            }
            PromptKind::ReplaceWith(f) => {
                let n = self
                    .buf_mut()
                    .map(|b| b.replace_all(&f, p.input.text()))
                    .unwrap_or(0);
                self.msg(format!("Replaced {n} occurrence(s)"));
            }
            PromptKind::OpenProject => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.base_dir(), &input);
                if path.is_dir() {
                    self.switch_project(Project::from_folders(vec![
                        path.canonicalize().unwrap_or(path),
                    ]));
                } else {
                    self.open_project_file(&path);
                }
            }
            PromptKind::SaveProjectAs => {
                if input.is_empty() {
                    return;
                }
                let mut path = self.resolve(&self.base_dir(), &input);
                if path.extension().is_none_or(|e| e != project::PROJECT_EXT) {
                    path.set_extension(project::PROJECT_EXT);
                }
                let old_ws = self.project.workspace_file(self.config.data_dir.as_deref());
                match self.project.save_as(&path) {
                    Ok(()) => {
                        if let Some(old) = old_ws
                            && self
                                .project
                                .file
                                .as_ref()
                                .is_some_and(|f| f.with_extension(project::WORKSPACE_EXT) != old)
                        {
                            let _ = std::fs::remove_file(old);
                        }
                        self.save_workspace();
                        self.recent.touch(&self.project);
                        self.recent.save(self.config.data_dir.as_deref());
                        self.refresh_tree();
                        self.msg(format!("Saved project {}", path.display()));
                    }
                    Err(e) => self.msg(format!("Save project failed: {e}")),
                }
            }
            PromptKind::AddFolder => {
                if input.is_empty() {
                    return;
                }
                let path = self.resolve(&self.base_dir(), &input);
                let path = path.canonicalize().unwrap_or(path);
                if !path.is_dir() {
                    return self.msg(format!("Not a folder: {input}"));
                }
                if self.project.add_folder(path.clone()) {
                    self.project_folders_changed();
                    self.tree.reveal(&path);
                }
            }
            PromptKind::PackageUrl => {
                if input.is_empty() {
                    return;
                }
                let Some(dir) = self.config.packages_dir() else {
                    return self.msg("No config folder for packages");
                };
                self.msg(format!("Cloning {input}…"));
                packages::install_git(dir, input, self.tx.clone());
            }
            PromptKind::RenameSymbol => {
                if input.is_empty() {
                    return;
                }
                self.lsp_position_request(
                    "textDocument/rename",
                    json!({"newName": input}),
                    Pending::Rename,
                    "renameProvider",
                );
            }
        }
    }
}
