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

    /// Grammars, highlight queries and themes: the built-ins, then each package in
    /// Packages/, then the config folder (later ones override earlier ones).
    pub(super) fn reload_syntax(&mut self) {
        let mut syn = Syntax::load(&self.config.editor.theme);
        syn.set_languages(&self.config.languages);
        if let Some(data) = &self.config.data_dir {
            syn.add_installed(data);
        }
        self.packages = match self.config.packages_dir() {
            Some(dir) => {
                let names = packages::list(&dir);
                for n in &names {
                    syn.add_dir(&dir.join(n));
                }
                names
            }
            None => vec![],
        };
        if let Some(dir) = self.config.config_dir() {
            syn.add_dir(&dir);
        }
        if !syn.set_theme(&self.config.editor.theme) {
            self.msg(format!("Unknown theme '{}'", self.config.editor.theme));
        }
        for e in &syn.errors {
            crate::log::error("syntax", e);
        }
        if !syn.errors.is_empty() {
            self.msg(format!("Theme/query errors: {}", syn.errors.join("; ")));
        }
        self.syn = syn;
        // Re-parse open files: a grammar may have been installed, or a query overridden.
        for i in 0..self.buffers.len() {
            let name = self.buffers[i].hl.syntax.clone();
            let grammar = self
                .language_for_syntax(&name)
                .and_then(|l| self.grammar_of(&l));
            self.buffers[i].hl.reset(name, grammar);
        }
    }

    /// Progress / result of a background grammar install.
    pub(super) fn on_grammar_event(&mut self, name: String, ev: InstallEvent) {
        match ev {
            InstallEvent::Log(m) => crate::log::info("grammar", format!("{name}: {m}")),
            InstallEvent::Done(res) => {
                self.installing_grammars.remove(&name);
                match res {
                    Ok(lib) => {
                        // Installs from a file only learn the grammar's name at the end.
                        let real = lib
                            .parent()
                            .and_then(|d| d.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or(name);
                        self.reload_syntax();
                        self.msg(format!("Installed the {real} grammar"));
                    }
                    Err(e) => {
                        crate::log::error("grammar", format!("{name}: {e}"));
                        self.msg(format!("Couldn't install the {name} grammar: {e}"));
                    }
                }
            }
        }
    }

    /// Install a grammar from a source folder or archive on this computer (background).
    pub fn install_grammar_from(&mut self, path: PathBuf) {
        let Some(data) = self.config.data_dir.clone() else {
            return self.msg("No data folder to install grammars into");
        };
        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "grammar".into());
        self.msg(format!("Installing a grammar from {label}…"));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let log = |m: String| {
                tx.send(AppEvent::Grammar(label.clone(), InstallEvent::Log(m)));
            };
            let res =
                crate::grammar_install::install_from_path(&data, &path, &log).map(|(_, lib)| lib);
            if let Err(e) = &res {
                log(format!("error: {e}"));
            }
            tx.send(AppEvent::Grammar(label.clone(), InstallEvent::Done(res)));
        });
    }

    /// Install a package from a folder or archive on this computer.
    pub fn install_package_from(&mut self, path: PathBuf) {
        let Some(dir) = self.config.packages_dir() else {
            return self.msg("No config folder for packages");
        };
        match packages::install_from_path(&dir, &path) {
            Ok(name) => {
                self.reload_config();
                self.msg(format!("Installed package {name}"));
            }
            Err(e) => self.msg(format!("Couldn't install the package: {e}")),
        }
    }

    /// Start installing a grammar from its recipe (download + compile, in the background).
    pub fn install_grammar(&mut self, name: &str) {
        let Some(r) = self.config.grammar_recipe(name).cloned() else {
            return self.msg(format!("No recipe for a `{name}` grammar"));
        };
        let Some(data) = self.config.data_dir.clone() else {
            return self.msg("No data folder to install grammars into");
        };
        if !self.installing_grammars.insert(name.to_string()) {
            return;
        }
        self.msg(format!(
            "Installing the {name} grammar ({})…",
            r.source_desc()
        ));
        crate::grammar_install::install(data, r, self.tx.clone());
    }

    /// The language and grammar for a file, from the languages config (extensions, file
    /// names, first-line patterns); Plain Text if none matches.
    pub(super) fn syntax_for(&self, path: Option<&Path>, first: &str) -> (String, Option<String>) {
        let lang = self
            .config
            .language_for(path.unwrap_or(Path::new("")), first);
        match lang {
            Some(l) => (l.name.clone(), self.grammar_of(l)),
            None => ("Plain Text".to_string(), None),
        }
    }

    /// A file needs a grammar that isn't installed: install it in the background (with
    /// `auto_install`), or say once per session how to get it.
    pub(super) fn hint_grammar(&mut self, l: &crate::config::LanguageConfig) {
        let g = l.grammar_name().to_string();
        if g.is_empty()
            || self.config.grammar_recipe(&g).is_none()
            || !self.grammar_hinted.insert(g.clone())
        {
            return;
        }
        if self.config.editor.auto_install && self.config.data_dir.is_some() {
            crate::log::info(
                "grammar",
                format!("{} needs the {g} grammar: installing it", l.title()),
            );
            return self.install_grammar(&g);
        }
        let m = format!(
            "{} highlighting needs its grammar: Grammars: Install Grammar… ({g})",
            l.title()
        );
        crate::log::info("grammar", m.clone());
        self.msg(m);
    }

    /// A language's tree-sitter grammar, if farsub has it.
    pub(super) fn grammar_of(&self, l: &crate::config::LanguageConfig) -> Option<String> {
        let g = l.grammar_name();
        (!g.is_empty() && self.syn.has_grammar(g)).then(|| g.to_string())
    }

    /// Display name for a language name ("cpp" → "C++").
    pub fn language_title(&self, name: &str) -> String {
        self.config
            .language(name)
            .map(|l| l.title().to_string())
            .unwrap_or_else(|| name.to_string())
    }
}
