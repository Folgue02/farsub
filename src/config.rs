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
//!   Packages/            farsub packages: themes/*.toml, queries/<grammar>/highlights.scm, icons.toml
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
    /// The editor font: an installed font family ("" = the built-in one). Preferences ›
    /// Select Font picks it.
    pub font_face: String,
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
    /// Enter on a line holding only indentation empties it (Sublime's setting).
    pub trim_automatic_white_space: bool,
    /// Wrap long lines at the pane width (View: Toggle Word Wrap changes it per file).
    pub word_wrap: bool,
    /// Shell for the integrated terminal (default: $SHELL, or cmd.exe on Windows).
    pub terminal_shell: Option<String>,
    /// Install what a file needs (its tree-sitter grammar, its language server) in the
    /// background as soon as it's opened, instead of asking.
    pub auto_install: bool,
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
            font_face: String::new(),
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
            trim_automatic_white_space: true,
            word_wrap: true,
            terminal_shell: None,
            auto_install: true,
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
    /// A download from a fixed URL per platform (`[install.url.assets]`).
    #[serde(default)]
    pub url: Option<UrlSource>,
    /// Commands run after the download (or on their own), without a shell so they work
    /// the same everywhere: `[[install.run]] command = "gem", args = [..]`. In args, `{dir}`
    /// is the install folder (`{dir/}` with forward slashes, for R / Ruby strings).
    #[serde(default)]
    pub run: Vec<RunStep>,
    /// Programs the install (or the server) needs on PATH, e.g. `["java"]`.
    #[serde(default)]
    pub needs: Vec<String>,
    /// What to tell the user when one of `needs` is missing.
    #[serde(default)]
    pub needs_hint: Option<String>,
    /// Executable name if it differs from the server's `command`.
    #[serde(default)]
    pub bin: Option<String>,
    /// The executable's file name on Windows when it isn't `<bin>.exe` (e.g. a `.bat`).
    #[serde(default)]
    pub bin_windows: Option<String>,
    /// Generate a launcher script that runs a program with fixed arguments and
    /// environment (Java servers, `R`, gems with their own GEM_HOME); it becomes the
    /// server's executable.
    #[serde(default)]
    pub wrapper: Option<Wrapper>,
}

/// `[[install.run]]`: one command of an install.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct RunStep {
    /// A program on PATH, or a path under the install folder (`{dir}/cs`).
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// `[install.url]`: download URLs keyed by platform, like the github source's assets.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct UrlSource {
    #[serde(default)]
    pub assets: HashMap<String, String>,
    /// Name to save a single-file download as (it isn't an archive).
    #[serde(default)]
    pub file: Option<String>,
}

/// `[install.wrapper]`: the launcher farsub writes into the install folder.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct Wrapper {
    /// A program on PATH (resolved when installing), or a path under the install folder
    /// (`{dir}/bin/ruby-lsp`, may contain `*`).
    pub command: String,
    /// `command` on Windows, when it differs.
    #[serde(default)]
    pub command_windows: Option<String>,
    /// Arguments before the server's own: `{dir}`, `{os}` (mac, linux, win), `{arm}`
    /// ("_arm" on ARM CPUs), `{root_id}` (a per-project id), `*` globs (the last match).
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// The entry for this OS/CPU in an assets map: "<os>-<arch>", then "<os>", then "all".
pub fn platform_asset(assets: &HashMap<String, String>) -> Option<&str> {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    assets
        .get(&format!("{os}-{arch}"))
        .or_else(|| assets.get(os))
        .or_else(|| assets.get("all"))
        .map(String::as_str)
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
    /// Name to save a single-file download as (e.g. `cs` for coursier's `.gz`).
    #[serde(default)]
    pub file: Option<String>,
}

impl GithubSource {
    /// The asset glob for this machine, if the project ships one.
    pub fn asset_for_this_platform(&self) -> Option<&str> {
        platform_asset(&self.assets)
    }
}

impl InstallSpec {
    /// Whether this recipe can install on the current OS/CPU.
    pub fn supported_here(&self) -> bool {
        let gh = self
            .github
            .as_ref()
            .is_none_or(|g| g.asset_for_this_platform().is_some());
        let url = self
            .url
            .as_ref()
            .is_none_or(|u| platform_asset(&u.assets).is_some());
        gh && url
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
        } else if let Some(u) = &self.url {
            let url = platform_asset(&u.assets).unwrap_or("");
            let host = url.split('/').nth(2).unwrap_or(url);
            format!("download: {host}")
        } else if let Some(r) = self.run.first() {
            format!(
                "{}: {}",
                r.command,
                r.args.last().map(String::as_str).unwrap_or("")
            )
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
    /// Display name ("C++", "JavaScript"); defaults to `name`.
    #[serde(default)]
    pub title: Option<String>,
    /// Tree-sitter grammar for highlighting (defaults to `name`); see `grammars.rs`.
    #[serde(default)]
    pub grammar: Option<String>,
    /// Sublime-style grammar (by name, e.g. "Bourne Again Shell") for the regex fallback
    /// used while there's no tree-sitter grammar. Default: found by title, name or file
    /// extension; `""` turns the fallback off.
    #[serde(default)]
    pub fallback_syntax: Option<String>,
    /// Server names, tried in order; the first installed one runs.
    #[serde(default)]
    pub servers: Vec<String>,
    /// Inline server (older config style); becomes a server named after the language.
    #[serde(default)]
    pub server: Option<ServerConfig>,
}

impl LanguageConfig {
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }

    pub fn grammar_name(&self) -> &str {
        self.grammar.as_deref().unwrap_or(&self.name)
    }

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
    /// Installable grammar recipes (grammars/*.toml).
    pub grammars: Vec<crate::grammar_install::GrammarRecipe>,
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
    Some(crate::paths::config_dir()?.join("config.toml"))
}

/// Set `key = value` (value already in TOML syntax) under `[editor]` in the user's config
/// file, keeping everything else in it as it is.
pub fn set_editor_setting(path: &Path, key: &str, value: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| TEMPLATE.to_string());
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let new = format!("{key} = {value}");
    let header = lines.iter().position(|l| l.trim() == "[editor]");
    match header {
        Some(h) => {
            let end = lines[h + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with('['))
                .map_or(lines.len(), |i| h + 1 + i);
            let existing = (h + 1..end).find(|&i| {
                let t = lines[i].trim_start();
                t.strip_prefix(key)
                    .is_some_and(|r| r.trim_start().starts_with('='))
            });
            match existing {
                Some(i) => lines[i] = new,
                None => lines.insert(h + 1, new),
            }
        }
        None => {
            lines.push(String::new());
            lines.push("[editor]".into());
            lines.push(new);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, lines.join("\n") + "\n")
}

/// A string as a TOML value.
pub fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

/// Create `path` (and its folder) with the starter settings if it doesn't exist yet.
/// Failing is fine: the defaults apply, and Preferences: Settings says what went wrong.
pub fn ensure_config_file(path: &Path) {
    if path.exists() {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, TEMPLATE);
}

pub fn default_data_dir() -> Option<PathBuf> {
    crate::paths::data_dir()
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

    /// The user's config (creating the config folder with a starter config.toml on first
    /// run, so there's always a file to edit).
    pub fn load() -> Config {
        let path = config_path();
        if let Some(p) = &path {
            ensure_config_file(p);
        }
        Self::load_from(path)
    }

    pub fn load_from(path: Option<PathBuf>) -> Config {
        let mut c = Self::builtin();
        c.path = path.clone();
        c.data_dir = default_data_dir();
        let mut errs: Vec<String> = c.error.take().into_iter().collect();
        if let Some(p) = &path {
            if let Some(dir) = p.parent() {
                // Folder overrides first, then config.toml on top.
                for sub in ["languages", "servers", "grammars"] {
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
        } else if let Some(file) = name.strip_prefix("grammars/") {
            let mut r: crate::grammar_install::GrammarRecipe = toml::from_str(text).map_err(err)?;
            if r.name.is_empty() {
                r.name = file.trim_end_matches(".toml").to_string();
            }
            upsert(&mut self.grammars, r, |a, b| a.name == b.name);
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

    /// Packages: grammar recipes from every package; icons from every
    /// `Packages/<name>/icons.toml` (or only `icon_theme`'s), in
    /// name order, then the user's own icons.toml on top.
    fn load_icon_packages(&mut self, errs: &mut Vec<String>) {
        let Some(dir) = self.packages_dir() else {
            return;
        };
        let theme = self.editor.icon_theme.clone().filter(|t| !t.is_empty());
        for name in crate::packages::list(&dir) {
            let pkg = dir.join(&name);
            // Grammar recipes a package provides (from every package).
            if let Ok(rd) = std::fs::read_dir(pkg.join("grammars")) {
                let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                files.sort();
                for f in files
                    .iter()
                    .filter(|f| f.extension().is_some_and(|e| e == "toml"))
                {
                    if let Ok(text) = std::fs::read_to_string(f)
                        && let Err(e) = self.merge_file(
                            &format!("grammars/{}", f.file_name().unwrap().to_string_lossy()),
                            &text,
                        )
                    {
                        errs.push(format!("{}: {e}", f.display()));
                    }
                }
            }
            // Icons only from the chosen icon theme, if one is set.
            if theme.as_ref().is_some_and(|t| *t != name) {
                continue;
            }
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

    pub fn grammar_recipe(&self, name: &str) -> Option<&crate::grammar_install::GrammarRecipe> {
        self.grammars.iter().find(|g| g.name == name)
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
#   themes/*.toml           color themes; queries/<grammar>/highlights.scm  highlight overrides
#   Packages/               packages adding themes, queries, icons; see "Packages:" commands
# Manage servers with Tools → Language Servers (Cmd+Shift+P "LSP: Manage Language Servers").

[editor]
font_size = 14.0
# An installed font family for the editor text, e.g. "JetBrains Mono" ("" = built-in).
# Preferences › Select Font… lists them and sets this for you.
font_face = ""
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
# Pressing Enter on a line that holds only indentation leaves it empty (no trailing spaces).
trim_automatic_white_space = true
# Wrap long lines at the pane width; View › Word Wrap (Alt+Z) toggles it per file.
word_wrap = true
# Shell for the terminal panel (default: $SHELL).
# terminal_shell = "/bin/zsh"
# Install a file's grammar and language server automatically (in the background) when it
# opens. Off: farsub tells you what's missing and you install from the Tools menu.
auto_install = true
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
# "Solarized (dark)", "Catppuccin Mocha", "gruvbox-dark", …) or one from themes/ or a package.
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
    fn pickers_edit_one_setting_and_keep_the_rest() {
        let d = std::env::temp_dir().join(format!("farsub-setset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let f = d.join("config.toml");
        // A missing file starts from the template.
        set_editor_setting(&f, "font_face", &toml_string("Menlo")).unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(t.contains("[editor]\nfont_face = \"Menlo\""), "{t}");
        // An existing value is replaced in place; comments, other keys and tables stay.
        std::fs::write(
            &f,
            "# mine\n[editor]\nfont_size = 15.0\nfont_face = \"X\" # old\n\n[[languages]]\nname = \"a\"\n",
        )
        .unwrap();
        set_editor_setting(&f, "font_face", &toml_string("Fira \"Code\"")).unwrap();
        set_editor_setting(&f, "theme", &toml_string("Nord")).unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert_eq!(
            t,
            "# mine\n[editor]\ntheme = \"Nord\"\nfont_size = 15.0\nfont_face = 'Fira \"Code\"'\n\n[[languages]]\nname = \"a\"\n"
        );
        let c: toml::Value = toml::from_str(&t).unwrap();
        assert_eq!(c["editor"]["font_face"].as_str(), Some("Fira \"Code\""));
        // No [editor] table yet: one is added.
        std::fs::write(&f, "[[languages]]\nname = \"a\"\n").unwrap();
        set_editor_setting(&f, "theme", &toml_string("Nord")).unwrap();
        let c: toml::Value = toml::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(c["editor"]["theme"].as_str(), Some("Nord"));
        let _ = std::fs::remove_dir_all(&d);
    }

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
