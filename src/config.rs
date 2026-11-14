//! Configuration, all data-driven. Built-in defaults live in `runtime/` (embedded at build
//! time); the user's config folder overrides any of them:
//!
//! ```text
//! ~/.config/farsub/
//!   config.toml          [editor] settings (+ optional [[languages]] / [[servers]])
//!   languages/*.toml     one language per file (same name replaces a built-in)
//!   servers/*.toml       language servers, nvim-lspconfig style, with mason-style [install]
//!   keymap.toml          extra/overriding [[keys]] (like Sublime's user keymap)
//!   menus.toml           replaces the menu layout
//!   Packages/            Sublime-style packages: *.sublime-syntax, *.tmTheme
//! ~/.local/share/farsub/servers/<name>/   servers installed by the server manager
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

include!(concat!(env!("OUT_DIR"), "/runtime_files.rs"));

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct EditorConfig {
    pub font_size: f32,
    /// Scale for the whole interface (all fonts, widgets, spacing), like browser zoom.
    /// 1.0 = 100%. Separate from font_size, which only sizes the editor text.
    pub ui_scale: f32,
    pub tab_size: usize,
    pub use_tabs: bool,
    pub theme: String,
    pub show_sidebar: bool,
    pub show_menu_bar: bool,
    /// Animate sidebar folders opening/closing.
    pub animations: bool,
    /// Vertical lines at each indentation level (like Sublime's indent guides).
    pub indent_guides: bool,
    /// Highlight the line the cursor is on (off by default).
    pub highlight_current_line: bool,
    /// Shell for the integrated terminal (default: $SHELL, or cmd.exe on Windows).
    pub terminal_shell: Option<String>,
    /// File icons in the sidebar, tabs and Goto Anything.
    pub file_icons: bool,
    /// Use only this icon package (a folder in Packages/ with an icons.toml); empty or
    /// unset: built-in icons plus every installed icon package.
    pub icon_theme: Option<String>,
    /// How the caret animates: "smooth" (fades in and out), "blink", or "solid".
    pub caret_style: CaretStyle,
    /// Pixels added to the 1px caret width (Sublime's `caret_extra_width`).
    pub caret_extra_width: f32,
    /// Pixels the caret reaches above the line (Sublime's `caret_extra_top`).
    pub caret_extra_top: f32,
    /// Pixels the caret reaches below the line (Sublime's `caret_extra_bottom`).
    pub caret_extra_bottom: f32,
    /// Keep unsaved changes across restarts instead of asking on quit (like Sublime).
    pub hot_exit: bool,
    /// Start without arguments → reopen the last project/folders.
    pub restore_session: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            font_size: 14.0,
            ui_scale: 1.0,
            tab_size: 4,
            use_tabs: false,
            theme: "Monokai Extended".into(),
            show_sidebar: true,
            // macOS gets the native menu bar instead.
            show_menu_bar: !cfg!(target_os = "macos"),
            animations: true,
            indent_guides: true,
            highlight_current_line: false,
            terminal_shell: None,
            file_icons: true,
            icon_theme: None,
            caret_style: CaretStyle::Smooth,
            caret_extra_width: 1.5,
            caret_extra_top: 1.0,
            caret_extra_bottom: 1.0,
            hot_exit: true,
            restore_session: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CaretStyle {
    /// Fades out and back in (Sublime's default "smooth").
    #[default]
    Smooth,
    /// Hard on/off blinking.
    Blink,
    /// Always visible.
    Solid,
}

/// One file-icon rule from icons.toml.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct IconDef {
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub color: String,
    /// A PNG instead of the label badge (icon packages); resolved to an absolute path.
    #[serde(default)]
    pub image: Option<String>,
}

#[derive(Deserialize, Default)]
struct IconsFile {
    #[serde(default)]
    icons: Vec<IconDef>,
    #[serde(default)]
    default: Option<IconDef>,
    #[serde(default)]
    folder_color: Option<String>,
    #[serde(default)]
    folder_image: Option<String>,
    #[serde(default)]
    folder_open_image: Option<String>,
}

/// "#RRGGBB" → (r, g, b).
pub fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

impl Config {
    /// The icon rule for a file name: exact names first, then the longest matching
    /// extension (so `a.d.ts` can differ from `.ts`); later rules win ties.
    pub fn icon_for(&self, file_name: &str) -> Option<&IconDef> {
        if let Some(d) = self
            .icons
            .iter()
            .rev()
            .find(|d| d.names.iter().any(|n| n == file_name))
        {
            return Some(d);
        }
        let lower = file_name.to_lowercase();
        let mut best: Option<(&IconDef, usize)> = None;
        for d in self.icons.iter().rev() {
            for e in &d.extensions {
                let e = e.to_lowercase();
                if lower.len() > e.len()
                    && lower.ends_with(&e)
                    && lower.as_bytes()[lower.len() - e.len() - 1] == b'.'
                    && best.is_none_or(|(_, l)| e.len() > l)
                {
                    best = Some((d, e.len()));
                }
            }
        }
        best.map(|(d, _)| d).or(self.icon_default.as_ref())
    }
}

/// How to install a server (like a mason.nvim package). One source is used.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct InstallSpec {
    #[serde(default)]
    pub npm: Vec<String>,
    #[serde(default)]
    pub pip: Vec<String>,
    #[serde(default)]
    pub cargo: Option<String>,
    #[serde(default)]
    pub go: Option<String>,
    /// Shell script run in the install folder ($FARSUB_INSTALL_DIR); `bin` is relative to it.
    #[serde(default)]
    pub script: Option<String>,
    /// A prebuilt binary from a GitHub release (like Mason's github source).
    #[serde(default)]
    pub github: Option<GithubSource>,
    /// Executable name if it differs from the server's `command`.
    #[serde(default)]
    pub bin: Option<String>,
}

/// `[install.github]`: download a release asset, extract it, find the executable.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct GithubSource {
    /// "owner/name"
    pub repo: String,
    /// Release tag; the latest release when absent.
    #[serde(default)]
    pub tag: Option<String>,
    /// Asset name glob per platform: "<os>-<arch>" (os: macos, linux, windows; arch:
    /// x86_64, aarch64) or just "<os>" for any architecture.
    #[serde(default)]
    pub assets: HashMap<String, String>,
}

impl GithubSource {
    /// The asset glob for this machine, if the project ships one.
    pub fn asset_for_this_platform(&self) -> Option<&str> {
        let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
        self.assets
            .get(&format!("{os}-{arch}"))
            .or_else(|| self.assets.get(os))
            .map(String::as_str)
    }
}

impl InstallSpec {
    /// Whether this recipe can install on the current OS/CPU.
    pub fn supported_here(&self) -> bool {
        match &self.github {
            Some(g) => g.asset_for_this_platform().is_some(),
            None => true,
        }
    }

    pub fn source(&self) -> String {
        if !self.npm.is_empty() {
            format!("npm: {}", self.npm.join(" "))
        } else if !self.pip.is_empty() {
            format!("pip: {}", self.pip.join(" "))
        } else if let Some(c) = &self.cargo {
            format!("cargo: {c}")
        } else if let Some(g) = &self.go {
            format!("go: {g}")
        } else if let Some(g) = &self.github {
            format!("github: {}", g.repo)
        } else if self.script.is_some() {
            "script".into()
        } else {
            "none".into()
        }
    }
}

/// A language server definition (like an nvim-lspconfig entry).
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct ServerConfig {
    #[serde(default)]
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub description: String,
    /// Sent as `initializationOptions`.
    #[serde(default)]
    pub initialization_options: Option<toml::Value>,
    /// Answers `workspace/configuration`, keyed by section.
    #[serde(default)]
    pub settings: Option<toml::Value>,
    #[serde(default)]
    pub install: Option<InstallSpec>,
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct LanguageConfig {
    pub name: String,
    /// LSP languageId (defaults to `name`).
    #[serde(default)]
    pub language_id: Option<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Exact file names, e.g. "Makefile".
    #[serde(default)]
    pub filenames: Vec<String>,
    /// Regex matched against the first line (shebangs), e.g. "^#!.*python".
    #[serde(default)]
    pub first_line: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub tab_size: Option<usize>,
    #[serde(default)]
    pub use_tabs: Option<bool>,
    /// Extra project-root markers (servers have their own).
    #[serde(default)]
    pub roots: Vec<String>,
    /// Syntax name override (otherwise detected from the extension / packages).
    #[serde(default)]
    pub syntax: Option<String>,
    /// Server names, tried in order; the first installed one runs.
    #[serde(default)]
    pub servers: Vec<String>,
    /// Inline server (older config style); becomes a server named after the language.
    #[serde(default)]
    pub server: Option<ServerConfig>,
}

impl LanguageConfig {
    pub fn language_id(&self) -> &str {
        self.language_id.as_deref().unwrap_or(&self.name)
    }

    pub fn matches_path(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if self.filenames.iter().any(|f| f == name) {
            return true;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        !ext.is_empty() && self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }

    pub fn matches_first_line(&self, line: &str) -> bool {
        self.first_line
            .as_deref()
            .and_then(|r| regex::Regex::new(r).ok())
            .is_some_and(|re| re.is_match(line))
    }
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct KeyBinding {
    pub keys: String,
    /// Command id; "" or "none" unbinds the key in that context.
    pub command: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct KeyDoc {
    pub section: String,
    pub keys: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct MenuDef {
    pub title: String,
    pub items: Vec<String>,
}

#[derive(Deserialize, Default)]
struct FileConfig {
    #[serde(default)]
    editor: Option<EditorConfig>,
    #[serde(default)]
    languages: Vec<LanguageConfig>,
    #[serde(default)]
    servers: Vec<ServerConfig>,
}

#[derive(Deserialize, Default)]
struct KeymapFile {
    #[serde(default)]
    keys: Vec<KeyBinding>,
    #[serde(default)]
    docs: Vec<KeyDoc>,
}

#[derive(Deserialize, Default)]
struct MenusFile {
    #[serde(default)]
    menu: Vec<MenuDef>,
    /// Right-click menus by place: editor, sidebar, tab, terminal, console.
    #[serde(default)]
    context: HashMap<String, ContextDef>,
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct ContextDef {
    pub items: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub editor: EditorConfig,
    pub languages: Vec<LanguageConfig>,
    pub servers: Vec<ServerConfig>,
    pub keys: Vec<KeyBinding>,
    pub docs: Vec<KeyDoc>,
    pub menus: Vec<MenuDef>,
    /// Right-click menus (context name → items), like Sublime's Context.sublime-menu.
    pub context_menus: HashMap<String, Vec<String>>,
    /// File icons (icons.toml): later entries win.
    pub icons: Vec<IconDef>,
    pub icon_default: Option<IconDef>,
    pub folder_color: Option<String>,
    /// Folder images from an icon package: (closed, open).
    pub folder_images: (Option<PathBuf>, Option<PathBuf>),
    /// Icon packages whose icons.toml was loaded.
    pub icon_packages: Vec<String>,
    /// The main config.toml.
    pub path: Option<PathBuf>,
    /// Where installed servers go.
    pub data_dir: Option<PathBuf>,
    pub error: Option<String>,
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("FARSUB_CONFIG") {
        return Some(PathBuf::from(x));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("farsub").join("config.toml"))
}

pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("FARSUB_DATA") {
        return Some(PathBuf::from(x));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
        })?;
    Some(base.join("farsub"))
}

fn upsert<T>(list: &mut Vec<T>, item: T, same: impl Fn(&T, &T) -> bool) {
    match list.iter_mut().find(|x| same(x, &item)) {
        Some(slot) => *slot = item,
        None => list.push(item),
    }
}

impl Config {
    /// Built-in defaults only.
    pub fn builtin() -> Config {
        let mut c = Config {
            editor: EditorConfig::default(),
            ..Default::default()
        };
        let mut errs = vec![];
        for (name, text) in RUNTIME_FILES {
            if let Err(e) = c.merge_file(name, text) {
                errs.push(format!("built-in {name}: {e}"));
            }
        }
        if !errs.is_empty() {
            c.error = Some(errs.join("; "));
        }
        c
    }

    pub fn load() -> Config {
        Self::load_from(config_path())
    }

    pub fn load_from(path: Option<PathBuf>) -> Config {
        let mut c = Self::builtin();
        c.path = path.clone();
        c.data_dir = default_data_dir();
        let mut errs: Vec<String> = c.error.take().into_iter().collect();
        if let Some(p) = &path {
            if let Some(dir) = p.parent() {
                // Folder overrides first, then config.toml on top.
                for sub in ["languages", "servers"] {
                    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(sub))
                        .map(|rd| rd.flatten().map(|e| e.path()).collect())
                        .unwrap_or_default();
                    files.sort();
                    for f in files
                        .iter()
                        .filter(|f| f.extension().is_some_and(|e| e == "toml"))
                    {
                        if let Ok(text) = std::fs::read_to_string(f)
                            && let Err(e) = c.merge_file(
                                &format!("{sub}/{}", f.file_name().unwrap().to_string_lossy()),
                                &text,
                            )
                        {
                            errs.push(format!("{}: {e}", f.display()));
                        }
                    }
                }
                for name in ["keymap.toml", "menus.toml"] {
                    let f = dir.join(name);
                    if let Ok(text) = std::fs::read_to_string(&f)
                        && let Err(e) = c.merge_file(name, &text)
                    {
                        errs.push(format!("{}: {e}", f.display()));
                    }
                }
            }
            if let Ok(text) = std::fs::read_to_string(p)
                && let Err(e) = c.merge_str(&text)
            {
                errs.push(format!("{}: {e}", p.display()));
            }
            // After config.toml, so `icon_theme` is known.
            c.load_icon_packages(&mut errs);
        }
        if !errs.is_empty() {
            c.error = Some(errs.join("; "));
        }
        c
    }

    /// Merge one data file, dispatching on its path inside runtime/ or the config folder.
    pub fn merge_file(&mut self, name: &str, text: &str) -> Result<(), String> {
        let err = |e: toml::de::Error| e.message().to_string();
        if name.starts_with("languages/") {
            let l: LanguageConfig = toml::from_str(text).map_err(err)?;
            self.add_language(l);
        } else if name.starts_with("servers/") {
            let mut s: ServerConfig = toml::from_str(text).map_err(err)?;
            if s.name.is_empty() {
                s.name = Path::new(name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
            }
            upsert(&mut self.servers, s, |a, b| a.name == b.name);
        } else if name == "keymap.toml" {
            let k: KeymapFile = toml::from_str(text).map_err(err)?;
            self.keys.extend(k.keys);
            for d in k.docs {
                upsert(&mut self.docs, d, |a, b| {
                    a.section == b.section && a.keys == b.keys
                });
            }
        } else if name == "icons.toml" {
            self.merge_icons(text, None)?;
        } else if name == "menus.toml" {
            let m: MenusFile = toml::from_str(text).map_err(err)?;
            // A file with only [context.*] sections keeps the menu bar as it is.
            if !m.menu.is_empty() {
                self.menus = m.menu;
            }
            for (k, v) in m.context {
                self.context_menus.insert(k, v.items);
            }
        }
        Ok(())
    }

    /// Merge an icons.toml. `base` is the folder image paths are relative to (packages).
    pub fn merge_icons(&mut self, text: &str, base: Option<&Path>) -> Result<(), String> {
        let f: IconsFile = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let abs = |p: Option<String>| -> Option<PathBuf> {
            let p = PathBuf::from(p?);
            Some(match base {
                Some(b) if p.is_relative() => b.join(p),
                _ => p,
            })
        };
        let fix = |mut d: IconDef| {
            d.image = abs(d.image.take()).map(|p| p.to_string_lossy().into_owned());
            d
        };
        self.icons.extend(f.icons.into_iter().map(fix));
        if let Some(d) = f.default {
            self.icon_default = Some(fix(d));
        }
        if f.folder_color.is_some() {
            self.folder_color = f.folder_color;
        }
        if f.folder_image.is_some() || f.folder_open_image.is_some() {
            let closed = abs(f.folder_image);
            let open = abs(f.folder_open_image).or_else(|| closed.clone());
            self.folder_images = (closed, open);
        }
        Ok(())
    }

    /// Icon packages: every `Packages/<name>/icons.toml` (or only `icon_theme`'s), in
    /// name order, then the user's own icons.toml on top.
    fn load_icon_packages(&mut self, errs: &mut Vec<String>) {
        let Some(dir) = self.packages_dir() else {
            return;
        };
        let theme = self.editor.icon_theme.clone().filter(|t| !t.is_empty());
        for name in crate::packages::list(&dir) {
            if theme.as_ref().is_some_and(|t| *t != name) {
                continue;
            }
            let pkg = dir.join(&name);
            let f = pkg.join("icons.toml");
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            match self.merge_icons(&text, Some(&pkg)) {
                Ok(()) => self.icon_packages.push(name),
                Err(e) => errs.push(format!("{}: {e}", f.display())),
            }
        }
        if let Some(t) = theme
            && !self.icon_packages.contains(&t)
        {
            errs.push(format!("icon_theme: no package \"{t}\" with an icons.toml"));
        }
        if let Some(f) = self.config_dir().map(|d| d.join("icons.toml"))
            && let Ok(text) = std::fs::read_to_string(&f)
            && let Err(e) = self.merge_icons(&text, f.parent())
        {
            errs.push(format!("{}: {e}", f.display()));
        }
    }

    /// Merge a config.toml document.
    pub fn merge_str(&mut self, text: &str) -> Result<(), String> {
        let f: FileConfig = toml::from_str(text).map_err(|e| e.message().to_string())?;
        if let Some(e) = f.editor {
            self.editor = e;
        }
        for s in f.servers {
            upsert(&mut self.servers, s, |a, b| a.name == b.name);
        }
        for l in f.languages {
            self.add_language(l);
        }
        Ok(())
    }

    fn add_language(&mut self, mut l: LanguageConfig) {
        if let Some(mut s) = l.server.take() {
            if s.name.is_empty() {
                s.name = l.name.clone();
            }
            if s.root_markers.is_empty() {
                s.root_markers = l.roots.clone();
            }
            l.servers.retain(|n| n != &s.name);
            l.servers.insert(0, s.name.clone());
            upsert(&mut self.servers, s, |a, b| a.name == b.name);
        }
        upsert(&mut self.languages, l, |a, b| a.name == b.name);
    }

    pub fn language_for(&self, path: &Path, first_line: &str) -> Option<&LanguageConfig> {
        // Later entries (user-added) win over earlier ones on conflicting extensions.
        self.languages
            .iter()
            .rev()
            .find(|l| l.matches_path(path))
            .or_else(|| {
                self.languages
                    .iter()
                    .rev()
                    .find(|l| l.matches_first_line(first_line))
            })
    }

    pub fn language(&self, name: &str) -> Option<&LanguageConfig> {
        self.languages.iter().find(|l| l.name == name)
    }

    pub fn server(&self, name: &str) -> Option<&ServerConfig> {
        self.servers.iter().find(|s| s.name == name)
    }

    /// Languages that list a server.
    pub fn languages_using(&self, server: &str) -> Vec<String> {
        self.languages
            .iter()
            .filter(|l| l.servers.iter().any(|s| s == server))
            .map(|l| l.name.clone())
            .collect()
    }

    pub fn config_dir(&self) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
    }

    pub fn packages_dir(&self) -> Option<PathBuf> {
        self.config_dir().map(|d| d.join("Packages"))
    }
}

/// The file a new user settings file starts as (Sublime-style: nearly empty; the defaults
/// are shown beside it by Preferences: Settings).
pub const TEMPLATE: &str = r##"# farsub settings. Anything here overrides the defaults shown in the other pane
# (Preferences: Settings). Copy a line over, change it, save to apply.

[editor]
"##;

/// Every editor setting with its default and an explanation, shown read-only beside the
/// user's file. A test keeps it in sync with `EditorConfig::default()`.
pub const DEFAULT_SETTINGS: &str = r##"# farsub default settings (read-only reference).
#
# Every setting with its default value. Don't edit this file: copy the lines you want to
# change into your settings file (the other pane) and save it there. Your file only needs
# the settings you change.
#
# Everything else is data too, in your config folder (each overrides the built-ins):
#   languages/<name>.toml   add or replace a language
#   servers/<name>.toml     add or replace a language server (with optional [install])
#   keymap.toml             add/override key bindings: [[keys]] keys = "Cmd+K", command = "hover"
#   menus.toml              replace the menu layout and context menus
#   Packages/               Sublime packages (.sublime-syntax, .tmTheme); see "Packages:" commands
# Manage servers with Tools → Language Servers (Cmd+Shift+P "LSP: Manage Language Servers").

[editor]
font_size = 14.0
# Scale the whole interface: fonts, sidebar, tabs, menus, panels (1.25 = 125%).
ui_scale = 1.0
tab_size = 4
use_tabs = false
show_sidebar = true
# In-window menu bar (macOS uses the system menu bar either way).
# show_menu_bar = true
animations = true
indent_guides = true
highlight_current_line = false
# Shell for the terminal panel (default: $SHELL).
# terminal_shell = "/bin/zsh"
# File icons in the sidebar, tabs and Goto Anything; mapping in icons.toml.
file_icons = true
# Use only one icon package from Packages/ (one with an icons.toml):
# icon_theme = "My Icons"
# Caret: "smooth" (fade in/out), "blink" or "solid"; size in pixels like Sublime's
# caret_extra_width / caret_extra_top / caret_extra_bottom (width = 1 + extra).
caret_style = "smooth"
caret_extra_width = 1.5
caret_extra_top = 1.0
caret_extra_bottom = 1.0
# Keep unsaved changes when quitting and reopen the last session on start.
hot_exit = true
restore_session = true
# Any bundled theme ("Monokai Extended", "OneHalfDark", "Nord", "Dracula", "GitHub",
# "Solarized (dark)", "Visual Studio Dark+", …) or a .tmTheme from Packages/.
theme = "Monokai Extended"

# Example: a language defined inline here instead of languages/lua.toml.
#
# [[languages]]
# name = "lua"
# extensions = ["lua"]
# comment = "--"
# tab_size = 2
# servers = ["lua_ls"]
#
# [[servers]]
# name = "lua_ls"
# command = "lua-language-server"
# root_markers = [".luarc.json", ".git"]
# [servers.settings.Lua]
# diagnostics = { globals = ["vim"] }
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_reference_matches_the_code() {
        // The read-only reference shown by Preferences: Settings must parse to exactly the
        // built-in defaults and mention every setting.
        let mut c = Config::builtin();
        c.merge_str(DEFAULT_SETTINGS).unwrap();
        // Platform-dependent / unset by default: documented as comments.
        let want = EditorConfig {
            show_menu_bar: c.editor.show_menu_bar,
            ..EditorConfig::default()
        };
        assert_eq!(c.editor, want);
        let table = toml::Value::try_from(EditorConfig {
            terminal_shell: Some(String::new()),
            ..EditorConfig::default()
        })
        .unwrap();
        for key in table.as_table().unwrap().keys() {
            assert!(
                DEFAULT_SETTINGS.contains(&format!("{key} =")),
                "Default Settings doesn't document `{key}`"
            );
        }
        // A brand-new user file is the short Sublime-style one and is valid.
        let mut u = Config::builtin();
        u.merge_str(TEMPLATE).unwrap();
        assert_eq!(u.editor, EditorConfig::default());
    }

    #[test]
    fn builtins_load_and_user_overrides() {
        let c = Config::builtin();
        assert!(c.error.is_none(), "{:?}", c.error);
        assert_eq!(
            c.language_for(Path::new("a/b.rs"), "").unwrap().name,
            "rust"
        );
        assert_eq!(
            c.language_for(Path::new("x.tsx"), "")
                .unwrap()
                .language_id(),
            "typescriptreact"
        );
        assert_eq!(
            c.language_for(Path::new("run"), "#!/usr/bin/env python3")
                .unwrap()
                .name,
            "python"
        );
        assert_eq!(
            c.language_for(Path::new("deploy"), "#!/bin/bash")
                .unwrap()
                .name,
            "bash"
        );
        assert_eq!(
            c.language_for(Path::new(".bashrc"), "").unwrap().name,
            "bash"
        );
        assert!(c.language_for(Path::new("x.unknownlang"), "").is_none());
        assert_eq!(
            c.language("python").unwrap().servers,
            ["pyright", "basedpyright", "pylsp"]
        );
        let pyright = c.server("pyright").unwrap();
        assert_eq!(pyright.install.as_ref().unwrap().npm, ["pyright"]);
        assert!(pyright.settings.is_some());
        assert!(c.server("bashls").unwrap().install.is_some());
        assert!(c.keys.len() > 50 && !c.docs.is_empty() && c.menus.len() >= 8);

        // User overrides via config.toml (inline server style still works).
        let mut c = c;
        c.merge_str(
            r#"
            [editor]
            font_size = 16.0
            [[languages]]
            name = "python"
            extensions = ["py"]
            server = { command = "pylsp" }
            [[languages]]
            name = "lua"
            extensions = ["lua"]
            comment = "--"
            servers = ["lua_ls"]
            [[servers]]
            name = "lua_ls"
            command = "lua-language-server"
            [servers.settings.Lua]
            diagnostics = { globals = ["vim"] }
            "#,
        )
        .unwrap();
        assert_eq!(c.editor.font_size, 16.0);
        assert_eq!(c.editor.tab_size, 4, "missing keys keep defaults");
        assert_eq!(c.language("python").unwrap().servers, ["python"]);
        assert_eq!(c.server("python").unwrap().command, "pylsp");
        assert_eq!(c.languages_using("lua_ls"), ["lua"]);
        assert!(c.merge_str("[[languages]]\nextensions = 3").is_err());
        c.merge_str(TEMPLATE).unwrap();
    }

    #[test]
    fn config_folder_files() {
        let d = std::env::temp_dir().join(format!("farsub-cfgdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("languages")).unwrap();
        std::fs::create_dir_all(d.join("servers")).unwrap();
        std::fs::write(
            d.join("languages/zig.toml"),
            "name = \"zig\"\nextensions = [\"zig\"]\ncomment = \"//\"\nservers = [\"zls\"]\n",
        )
        .unwrap();
        std::fs::write(d.join("servers/zls.toml"), "command = \"zls\"\n").unwrap();
        std::fs::write(
            d.join("keymap.toml"),
            "[[keys]]\nkeys = \"Cmd+K\"\ncommand = \"hover\"\ncontext = \"editor\"\n",
        )
        .unwrap();
        std::fs::write(
            d.join("menus.toml"),
            "[[menu]]\ntitle = \"Mine\"\nitems = [\"save\"]\n",
        )
        .unwrap();
        let c = Config::load_from(Some(d.join("config.toml")));
        assert!(c.error.is_none(), "{:?}", c.error);
        assert_eq!(
            c.language_for(Path::new("a.zig"), "").unwrap().servers,
            ["zls"]
        );
        assert_eq!(c.server("zls").unwrap().command, "zls");
        assert_eq!(c.keys.last().unwrap().command, "hover");
        assert_eq!(c.menus.len(), 1);
        std::fs::write(d.join("servers/bad.toml"), "command = 3\n").unwrap();
        assert!(
            Config::load_from(Some(d.join("config.toml")))
                .error
                .unwrap()
                .contains("bad.toml")
        );
    }
}
