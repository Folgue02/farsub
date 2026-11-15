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

    /// Preferences: Settings — like Sublime: a window of its own (another farsub process,
    /// `--settings`) with the defaults on the left and your file on the right. Saving there
    /// applies to every window (each one notices the file changed).
    pub(super) fn open_settings(&mut self) {
        if self.settings_window {
            return self.open_settings_here();
        }
        let args = self.window_args("--settings");
        self.spawn_window(args);
    }

    /// The config file changed on disk (saved in the settings window, or by hand): reload
    /// it. True if it did.
    pub(super) fn check_config_file(&mut self) -> bool {
        let Some(path) = self.config.path.clone() else {
            return false;
        };
        let now = fileops::DiskStamp::of(&path);
        if now == self.config_stamp {
            return false;
        }
        self.config_stamp = now;
        // An open, edited copy is saved by this window itself: leave it alone.
        if self
            .buffers
            .iter()
            .any(|b| b.path.as_deref() == Some(path.as_path()) && b.modified)
        {
            return false;
        }
        self.reload_config();
        true
    }

    /// Save a setting chosen in a picker into the user's settings file (other windows pick
    /// it up from there).
    pub(super) fn keep_setting(&mut self, key: &str, value: &str, label: &str) {
        let Some(path) = self.config.path.clone().or_else(config::config_path) else {
            return self.msg(format!("{label} (no settings file to keep it in)"));
        };
        match config::set_editor_setting(&path, key, &config::toml_string(value)) {
            Ok(()) => {
                // Already applied here: don't reload it as someone else's change.
                self.config_stamp = fileops::DiskStamp::of(&path);
                self.msg(format!("{label} (saved in your settings)"));
            }
            Err(e) => self.msg(format!("{label}; couldn't save it: {e}")),
        }
    }

    /// The two-pane settings view in this window (the settings window shows it).
    pub fn open_settings_here(&mut self) {
        let Some(path) = self.config.path.clone().or_else(config::config_path) else {
            return self.msg(
                "No config location: no home folder found (set FARSUB_CONFIG to a file path)",
            );
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
        self.config_stamp = self.config.path.as_deref().and_then(fileops::DiskStamp::of);
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
