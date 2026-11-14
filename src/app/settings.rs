//! App: settings (Preferences: Settings, reloading the config, themes).

use super::*;

impl App {
    // ===================================================================== settings

    /// Where the read-only default settings reference is written.
    pub fn defaults_settings_path(&self) -> PathBuf {
        self.config
            .data_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir)
            .join("Default Settings.toml")
    }

    /// Preferences: Settings — like Sublime: the full default settings (read-only) on the
    /// left, the user's file on the right, in a two-column layout.
    pub(super) fn open_settings(&mut self) {
        let Some(path) = self.config.path.clone().or_else(config::config_path) else {
            return self.msg("No config location (set $HOME or $FARSUB_CONFIG)");
        };
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, config::TEMPLATE) {
                return self.msg(format!("Could not create {}: {e}", path.display()));
            }
        }
        let defaults = self.defaults_settings_path();
        if let Some(parent) = defaults.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Always rewritten, so it matches this version of farsub.
        if let Err(e) = std::fs::write(&defaults, config::DEFAULT_SETTINGS) {
            return self.msg(format!("Could not write {}: {e}", defaults.display()));
        }
        let prev_layout = self
            .settings_view
            .take()
            .map(|v| v.prev_layout)
            .unwrap_or(self.layout);
        if self.layout < 2 {
            self.set_layout(2);
        }
        let place = |app: &mut App, p: &Path, g: usize| -> Option<usize> {
            app.group = g;
            let i = app.open_file(p, true)?;
            app.buffers[i].group = g;
            app.set_cur(i);
            Some(app.buffers[i].id)
        };
        let Some(defaults_id) = place(self, &defaults, 0) else {
            return;
        };
        if let Some(i) = self.buf_index(defaults_id) {
            self.buffers[i].read_only = true;
        }
        let Some(user_id) = place(self, &path, 1) else {
            return;
        };
        self.focus = Focus::Editor;
        self.settings_view = Some(SettingsView {
            defaults_id,
            user_id,
            prev_layout,
        });
    }

    pub(super) fn reload_config(&mut self) {
        let mut new = Config::load_from(self.config.path.clone());
        if let Some(snippet) = self.project.config_snippet() {
            let _ = new.merge_str(&snippet);
        }
        if let Ok(ed) = project::apply_settings(&new.editor, &self.project.settings) {
            new.editor = ed;
        }
        if let Some(e) = &new.error {
            return self.msg(format!("Config error: {e}"));
        }
        let langs_changed =
            new.languages != self.config.languages || new.servers != self.config.servers;
        let data = self.config.data_dir.clone();
        self.config = new;
        if self.config.data_dir.is_none() {
            self.config.data_dir = data;
        }
        if let Some(e) = self.rebuild_keys_and_menus() {
            self.msg(format!("Config error: {e}"));
        }
        self.reload_syntax();
        for b in &mut self.buffers {
            b.hl.invalidate(0);
        }
        self.font_size = self.config.editor.font_size;
        self.tree.anim = Duration::from_millis(if self.config.editor.animations {
            160
        } else {
            0
        });
        for i in 0..self.buffers.len() {
            if let Some(p) = self.buffers[i].path.clone() {
                let first = self.buffers[i].lines[0].clone();
                let lang = self.config.language_for(&p, &first).cloned();
                let b = &mut self.buffers[i];
                b.apply_language(lang.as_ref(), &self.config.editor);
            }
        }
        if langs_changed {
            self.restart_lsp();
        }
        if self
            .status
            .as_ref()
            .is_none_or(|s| !s.0.starts_with("Unknown theme"))
        {
            self.msg("Settings reloaded");
        }
    }

    pub(super) fn restart_lsp(&mut self) {
        self.lsp.restart();
        self.lsp_warned.clear();
        self.diagnostics.clear();
        for i in 0..self.buffers.len() {
            self.buffers[i].lsp_client = None;
            self.lsp_open(i);
        }
    }

    pub(super) fn apply_theme(&mut self, name: &str) {
        self.syn.set_theme(name);
        for b in &mut self.buffers {
            b.hl.invalidate(0);
        }
    }
}
