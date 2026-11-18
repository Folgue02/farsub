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

    /// Where the read-only default key bindings reference is written.
    pub fn defaults_keymap_path(&self) -> PathBuf {
        self.defaults_settings_path()
            .with_file_name("Default Key Bindings.toml")
    }

    /// The settings file and keymap.toml on disk now.
    pub(super) fn config_stamp_now(&self) -> [Option<fileops::DiskStamp>; 2] {
        let Some(p) = self.config.path.as_deref() else {
            return [None, None];
        };
        [
            fileops::DiskStamp::of(p),
            fileops::DiskStamp::of(&config::keymap_path(p)),
        ]
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

    /// The settings file or keymap.toml changed on disk (saved in the settings window, or
    /// by hand): reload the config. True if it did.
    pub(super) fn check_config_file(&mut self) -> bool {
        let Some(path) = self.config.path.clone() else {
            return false;
        };
        let now = self.config_stamp_now();
        if now == self.config_stamp {
            return false;
        }
        self.config_stamp = now;
        // An open, edited copy is saved by this window itself: leave it alone.
        let files = [config::keymap_path(&path), path];
        if self
            .buffers
            .iter()
            .any(|b| b.modified && b.path.as_ref().is_some_and(|p| files.contains(p)))
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
                self.config_stamp = self.config_stamp_now();
                self.msg(format!("{label} (saved in your settings)"));
            }
            Err(e) => self.msg(format!("{label}; couldn't save it: {e}")),
        }
    }

    /// Preferences: Key Bindings: like Preferences: Settings, a window of its own
    /// (`--keymap`) with the default bindings on the left and your keymap.toml on the right.
    pub(super) fn open_keymap(&mut self) {
        if self.settings_window {
            return self.open_keymap_here();
        }
        let args = self.window_args("--keymap");
        self.spawn_window(args);
    }

    /// Project › Edit Project: like Preferences: Settings, a window of its own
    /// (`--edit-project FILE`) with an example project on the left and the project file on
    /// the right. Saving it there applies it in the project's window.
    pub(super) fn open_project_editor(&mut self, file: &Path) {
        if self.settings_window {
            return self.open_project_here(file);
        }
        let mut args = self.window_args("--edit-project");
        args.insert(1, file.into());
        self.spawn_window(args);
    }

    /// The two-pane project view in this window (the Edit Project window shows it).
    pub fn open_project_here(&mut self, file: &Path) {
        let defaults = self.example_project_path();
        self.open_two_panes(file.to_path_buf(), "", defaults, project::EXAMPLE_PROJECT);
    }

    /// Project › New Project: the Edit Project window with a new, unsaved project file
    /// (`--new-project`); saving asks where to put it.
    pub(super) fn open_new_project(&mut self) {
        if self.settings_window {
            return self.open_new_project_here();
        }
        let args = self.window_args("--new-project");
        self.spawn_window(args);
    }

    pub fn open_new_project_here(&mut self) {
        let defaults = self.example_project_path();
        self.open_pair(None, project::NEW_PROJECT, defaults, project::EXAMPLE_PROJECT);
    }

    /// The unsaved project file of a New Project window.
    pub(super) fn is_new_project(&self, id: usize) -> bool {
        self.settings_view.iter().any(|v| v.user_id == id)
            && self.buf_index(id).is_some_and(|i| self.buffers[i].path.is_none())
    }

    fn example_project_path(&self) -> PathBuf {
        self.defaults_settings_path()
            .with_file_name(format!("Example Project.{}", project::PROJECT_EXT))
    }

    /// The project file to open in a new window: the one selected in the sidebar (when
    /// it has focus), else the current file, if it's a `.farsub-project`.
    pub fn project_file_target(&self) -> Option<PathBuf> {
        let p = if self.focus == Focus::Sidebar {
            self.tree.selected_path().map(Path::to_path_buf)
        } else {
            self.buf().and_then(|b| b.path.clone())
        }?;
        (p.extension().is_some_and(|e| e == project::PROJECT_EXT) && p.is_file()).then_some(p)
    }

    /// Open This Project in New Window: another farsub window with that project.
    pub(super) fn open_project_in_new_window(&mut self) {
        let Some(file) = self.project_file_target() else {
            return self.msg("Select a .farsub-project file first");
        };
        let mut args = self.window_args("--");
        args[0] = file.into();
        self.spawn_window(args);
    }

    /// The two-pane settings view in this window (the settings window shows it).
    pub fn open_settings_here(&mut self) {
        let Some(path) = self.config.path.clone().or_else(config::config_path) else {
            return self.no_config_location();
        };
        let defaults = self.defaults_settings_path();
        self.open_two_panes(path, config::TEMPLATE, defaults, config::DEFAULT_SETTINGS);
    }

    /// The two-pane key bindings view in this window (the key bindings window shows it).
    pub fn open_keymap_here(&mut self) {
        let Some(path) = self.config.path.clone().or_else(config::config_path) else {
            return self.no_config_location();
        };
        let defaults = self.defaults_keymap_path();
        let reference = config::default_keymap_reference();
        self.open_two_panes(
            config::keymap_path(&path),
            config::KEYMAP_TEMPLATE,
            defaults,
            &reference,
        );
    }

    fn no_config_location(&mut self) {
        self.msg("No config location: no home folder found (set FARSUB_CONFIG to a file path)")
    }

    /// A read-only reference on the left, the user's file on the right (created from
    /// `template` if missing). Closing either side closes both.
    fn open_two_panes(&mut self, path: PathBuf, template: &str, defaults: PathBuf, text: &str) {
        self.open_pair(Some(path), template, defaults, text);
    }

    /// `open_two_panes`, or with no `path` an unsaved buffer holding `template` (New
    /// Project), saved wherever you choose.
    fn open_pair(&mut self, path: Option<PathBuf>, template: &str, defaults: PathBuf, text: &str) {
        if let Some(path) = &path
            && !path.exists()
        {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(path, template) {
                return self.msg(format!("Could not create {}: {e}", path.display()));
            }
        }
        if let Some(parent) = defaults.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Always rewritten, so it matches this version of farsub.
        if let Err(e) = std::fs::write(&defaults, text) {
            return self.msg(format!("Could not write {}: {e}", defaults.display()));
        }
        // With another pair already open (settings and key bindings), closing the last
        // one brings back the layout from before the first.
        let prev_layout = self.settings_view.first().map_or(self.layout, |v| v.prev_layout);
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
        let user_id = match &path {
            Some(path) => {
                let Some(id) = place(self, path, 1) else {
                    return;
                };
                id
            }
            None => {
                self.group = 1;
                let id = self.next_buf_id;
                self.next_buf_id += 1;
                let b = Buffer::from_text(id, None, template, HlCache::new("Plain Text".into()));
                let i = self.push_buffer(b);
                self.buffers[i].group = 1;
                self.set_cur(i);
                self.set_syntax_at(i, "toml");
                // Ready to add settings under [settings].
                self.buffers[i].move_doc(true, false);
                id
            }
        };
        self.focus = Focus::Editor;
        if !self.settings_view.iter().any(|v| v.user_id == user_id) {
            self.settings_view.push(SettingsView {
                defaults_id,
                user_id,
                prev_layout,
            });
        }
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
        self.config_stamp = self.config_stamp_now();
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
                // The highlighter (or the grammar) may have changed.
                let grammar = lang.as_ref().and_then(|l| self.grammar_of(l));
                let b = &mut self.buffers[i];
                b.apply_language(lang.as_ref(), &self.config.editor);
                if let Some(l) = &lang
                    && !b.syntax_override
                    && b.hl.grammar != grammar
                {
                    b.hl.reset(l.name.clone(), grammar);
                }
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
