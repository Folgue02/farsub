//! App: keys, menus, servers, packages.

use super::*;

impl App {
    // ===================================================================== keys, menus, servers, packages

    /// Rebuild the keymap and menus from config; returns a combined error message.
    pub(super) fn rebuild_keys_and_menus(&mut self) -> Option<String> {
        let (km, mut errs) = Keymap::from_config(&self.config);
        let (menus, merrs) = menu::build(&self.config.menus);
        errs.extend(merrs);
        let (contexts, cerrs) = menu::build_context(&self.config.context_menus);
        errs.extend(cerrs);
        self.contexts = contexts;
        self.keymap = km;
        self.menus = menus;
        (!errs.is_empty()).then(|| errs.join("; "))
    }

    /// System clipboard text (falls back to the editor's own clipboard).
    pub fn clipboard_text(&self) -> String {
        arboard::Clipboard::new()
            .and_then(|mut c| c.get_text())
            .unwrap_or_else(|_| self.clip.clone())
    }

    /// A native menu item fired. If it has a shortcut, replay that key so the focused pane
    /// decides (Cmd+C copies files in the sidebar, text in the editor); otherwise run it.
    pub fn on_menu_command(&mut self, id: &str) {
        let Some(cmd) = crate::commands::from_id(id) else {
            return;
        };
        if !self.enabled(cmd) {
            return;
        }
        let key = self
            .keymap
            .primary(cmd)
            .filter(|b| b.ctx != Ctx::Sidebar)
            .map(|b| keymap::key_of(&b.spec));
        match key {
            Some(k)
                if self
                    .keymap
                    .lookup(k, Ctx::Global)
                    .or(self.keymap.lookup(k, Ctx::Editor))
                    .is_some() =>
            {
                self.on_key(k);
            }
            _ => self.run(cmd),
        }
    }

    pub fn server_status(&self, s: &crate::config::ServerConfig) -> ServerStatus {
        if self.installing.contains(&s.name) {
            return ServerStatus::Installing;
        }
        if let Some(d) = &self.config.data_dir
            && let Some(b) = installer::installed_bin(d, s)
        {
            return ServerStatus::Installed(b);
        }
        if let Some(e) = self.install_failed.get(&s.name) {
            return ServerStatus::Failed(e.clone());
        }
        match crate::lsp::find_server(&s.command) {
            Some(p) => ServerStatus::OnPath(p),
            None => ServerStatus::Missing,
        }
    }

    /// Servers in the manager's order: those for the current file's language first.
    pub fn server_rows(&self) -> Vec<crate::config::ServerConfig> {
        let cur = self
            .buf()
            .and_then(|b| b.lang.clone())
            .and_then(|l| self.config.language(&l).cloned());
        let mut v = self.config.servers.clone();
        v.sort_by_key(|s| {
            let rank = cur
                .as_ref()
                .and_then(|l| l.servers.iter().position(|n| n == &s.name))
                .unwrap_or(usize::MAX);
            (rank, s.name.clone())
        });
        v
    }

    pub(super) fn servers_sel_of(&self, name: &str) -> usize {
        self.server_rows()
            .iter()
            .position(|s| s.name == name)
            .unwrap_or(0)
    }

    pub fn install_server(&mut self, name: &str) {
        let Some(s) = self.config.server(name).cloned() else {
            return;
        };
        if s.install.is_none() {
            return self.msg(format!(
                "{name} can't be installed automatically — install `{}` yourself",
                s.command
            ));
        }
        let Some(data) = self.config.data_dir.clone() else {
            return self.msg("No data folder to install into (set $HOME or $FARSUB_DATA)");
        };
        if !self.installing.insert(name.to_string()) {
            return;
        }
        self.install_failed.remove(name);
        self.install_log.insert(
            name.to_string(),
            vec![format!(
                "installing {name} ({})",
                s.install.as_ref().unwrap().source()
            )],
        );
        self.msg(format!("Installing {name}…"));
        installer::install(data, s, self.tx.clone());
    }

    pub(super) fn on_install(&mut self, name: String, ev: InstallEvent) {
        match ev {
            InstallEvent::Log(l) => {
                crate::log::info(&format!("install:{name}"), &l);
                let log = self.install_log.entry(name).or_default();
                log.push(l);
                if log.len() > 200 {
                    log.remove(0);
                }
            }
            InstallEvent::Done(res) => {
                crate::lsp::clear_find_cache();
                self.installing.remove(&name);
                match res {
                    Ok(_) => {
                        self.msg(format!("{name} installed"));
                        self.lsp.reset_server(&name);
                        self.reopen_lsp_docs();
                    }
                    Err(e) => {
                        crate::log::error(&format!("install:{name}"), &e);
                        self.msg(format!("{name}: install failed — {e}"));
                        self.install_failed.insert(name, e);
                    }
                }
            }
        }
    }

    /// Start servers for open files that don't have one yet (after installs/uninstalls).
    pub(super) fn reopen_lsp_docs(&mut self) {
        self.lsp_warned.clear();
        for i in 0..self.buffers.len() {
            let alive = self.buffers[i]
                .lsp_client
                .and_then(|c| self.lsp.clients.get(&c))
                .is_some_and(|c| c.alive);
            if !alive {
                self.buffers[i].lsp_client = None;
                self.lsp_open(i);
            }
        }
    }

    pub(super) fn servers_key(&mut self, k: Key, mut v: ServersView) {
        let n = self.server_rows().len();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Home => v.sel = 0,
            KeyCode::End => v.sel = n.saturating_sub(1),
            KeyCode::Enter => v.details = !v.details,
            _ => {}
        }
        self.modal = Some(Modal::Servers(v));
    }

    pub(super) fn servers_char(&mut self, c: char) {
        let rows = self.server_rows();
        let Some(Modal::Servers(v)) = &mut self.modal else {
            return;
        };
        let n = rows.len();
        let sel = v.sel.min(n.saturating_sub(1));
        match c {
            'j' => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            'k' => v.sel = v.sel.saturating_sub(1),
            ' ' => v.details = !v.details,
            'i' | 'u' => {
                if let Some(s) = rows.get(sel) {
                    let name = s.name.clone();
                    self.install_server(&name);
                }
            }
            'x' | 'X' => {
                let Some(s) = rows.get(sel) else { return };
                let managed = self
                    .config
                    .data_dir
                    .as_ref()
                    .is_some_and(|d| installer::installed_bin(d, s).is_some());
                if !managed {
                    return self.msg(format!("{} wasn't installed by farsub", s.name));
                }
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Uninstall {}?", s.name),
                    kind: ConfirmKind::UninstallServer(s.name.clone()),
                }));
            }
            _ => {}
        }
    }

    pub(super) fn on_packages(&mut self, l: packages::Loaded) {
        let n_syn = l.syntaxes.syntaxes().len();
        self.syn.ss = l.syntaxes;
        self.syn.user_themes = l.themes;
        self.syn.set_theme(&self.config.editor.theme);
        self.packages = l.packages;
        // Re-detect syntaxes for open files (a package may provide a better grammar).
        for i in 0..self.buffers.len() {
            match self.buffers[i].path.clone() {
                Some(p) => self.set_buffer_path(i, p),
                None => self.buffers[i].hl.invalidate(0),
            }
        }
        for e in &l.errors {
            crate::log::error("packages", e);
        }
        if !l.errors.is_empty() {
            self.msg(format!("Package errors: {}", l.errors.join("; ")));
        } else if !self.packages.is_empty() {
            self.msg(format!(
                "Loaded {} package(s), {n_syn} syntaxes",
                self.packages.len()
            ));
        }
    }
}
