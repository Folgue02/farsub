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
            Some(Modal::Timers(d)) => self.timers_key(k, d),
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
            ConfirmKind::Reload(id) => self.reload_buffer(id),
            ConfirmKind::Overwrite(id) => {
                if let Some(i) = self.buf_index(id) {
                    // Accept what's on disk now as seen, then save over it.
                    let b = &mut self.buffers[i];
                    b.disk = b.path.as_deref().and_then(crate::fileops::DiskStamp::of);
                    self.set_cur(i);
                    self.save_current();
                }
            }
            ConfirmKind::ReplaceInFiles => self.replace_in_files(),
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
        match k.code {
            KeyCode::Esc => return,
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
            _ => {}
        }
        self.modal = Some(Modal::Prompt(p));
    }

    /// A path chosen in the system's file picker, handled like the typed prompt.
    pub fn path_picked(&mut self, req: FileRequest, path: &Path) {
        let kind = match req {
            FileRequest::OpenFile | FileRequest::OpenFolder => PromptKind::OpenPath,
            FileRequest::OpenProject => PromptKind::OpenProject,
            FileRequest::AddFolder => PromptKind::AddFolder,
            FileRequest::SaveProjectAs => PromptKind::SaveProjectAs,
            FileRequest::SaveAs(id) => match self.buf_index(id) {
                Some(i) => {
                    self.set_cur(i);
                    PromptKind::SaveAs
                }
                None => return,
            },
            _ => return,
        };
        self.prompt_accept(Prompt {
            title: String::new(),
            input: LineEdit::new(&path.display().to_string()),
            kind,
        });
    }

    /// Where a file picker starts, and the name it suggests (Save As).
    pub fn picker_start(&self, req: FileRequest) -> (PathBuf, Option<String>) {
        let base = self.base_dir();
        match req {
            FileRequest::SaveAs(id) => {
                let p = self
                    .buf_index(id)
                    .and_then(|i| self.buffers[i].path.clone());
                match p {
                    Some(p) => (
                        p.parent().map(Path::to_path_buf).unwrap_or(base),
                        p.file_name().map(|n| n.to_string_lossy().into_owned()),
                    ),
                    None if self.is_new_project(id) => {
                        (base, Some(format!("New Project.{}", project::PROJECT_EXT)))
                    }
                    None => (base, None),
                }
            }
            FileRequest::SaveProjectAs => {
                let name = match &self.project.file {
                    Some(f) => f.file_name().map(|n| n.to_string_lossy().into_owned()),
                    None => base
                        .file_name()
                        .map(|n| format!("{}.{}", n.to_string_lossy(), project::PROJECT_EXT)),
                };
                (base, name)
            }
            FileRequest::AddFolder => (base.parent().map(Path::to_path_buf).unwrap_or(base), None),
            _ => (base, None),
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
                let mut path = self.resolve(&self.base_dir(), &input);
                let new_project = self.is_new_project(self.buffers[self.cur].id);
                if new_project && path.extension().is_none_or(|e| e != project::PROJECT_EXT) {
                    path.set_extension(project::PROJECT_EXT);
                }
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let cur = self.cur;
                self.close_lsp_doc(cur);
                self.set_buffer_path(cur, path.clone());
                self.save_current();
                self.lsp_open(cur);
                self.refresh_tree();
                if new_project && self.buffers[cur].path.is_some() && !self.buffers[cur].modified {
                    self.msg(format!(
                        "Saved {} — Project: Open This Project in New Window opens it",
                        path.display()
                    ));
                }
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
