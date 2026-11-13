//! Editor + language configuration. Built-in defaults cover a handful of languages; a user
//! file at `~/.config/farsub/config.toml` can override any of them or add new ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct EditorConfig {
    pub font_size: f32,
    pub tab_size: usize,
    pub use_tabs: bool,
    pub theme: String,
    pub show_sidebar: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            font_size: 14.0,
            tab_size: 4,
            use_tabs: false,
            theme: "Monokai Extended".into(),
            show_sidebar: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Sent as `initializationOptions` in the initialize request.
    #[serde(default)]
    pub initialization_options: Option<toml::Value>,
    /// Answers `workspace/configuration` requests, keyed by section
    /// (e.g. `[languages.server.settings.rust-analyzer]`).
    #[serde(default)]
    pub settings: Option<toml::Value>,
}

#[derive(Clone, Debug, Deserialize, Default)]
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
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub tab_size: Option<usize>,
    #[serde(default)]
    pub use_tabs: Option<bool>,
    /// Files/dirs marking the project root for the language server.
    #[serde(default)]
    pub roots: Vec<String>,
    /// syntect syntax name override (otherwise detected from the extension).
    #[serde(default)]
    pub syntax: Option<String>,
    #[serde(default)]
    pub server: Option<ServerConfig>,
}

impl LanguageConfig {
    pub fn language_id(&self) -> &str {
        self.language_id.as_deref().unwrap_or(&self.name)
    }

    pub fn matches(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if self.filenames.iter().any(|f| f == name) {
            return true;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        !ext.is_empty() && self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
struct FileConfig {
    #[serde(default)]
    editor: Option<EditorConfig>,
    #[serde(default)]
    languages: Vec<LanguageConfig>,
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub editor: EditorConfig,
    pub languages: Vec<LanguageConfig>,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

fn lang(
    name: &str,
    id: Option<&str>,
    exts: &[&str],
    comment: &str,
    roots: &[&str],
    cmd: &str,
    args: &[&str],
) -> LanguageConfig {
    LanguageConfig {
        name: name.into(),
        language_id: id.map(String::from),
        extensions: exts.iter().map(|s| s.to_string()).collect(),
        comment: Some(comment.into()),
        roots: roots.iter().map(|s| s.to_string()).collect(),
        server: Some(ServerConfig {
            command: cmd.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn default_languages() -> Vec<LanguageConfig> {
    let js_roots = &["package.json", "tsconfig.json", "jsconfig.json"];
    let ts = &["typescript-language-server", "--stdio"];
    vec![
        lang(
            "rust",
            None,
            &["rs"],
            "//",
            &["Cargo.toml"],
            "rust-analyzer",
            &[],
        ),
        lang(
            "python",
            None,
            &["py", "pyi"],
            "#",
            &[
                "pyproject.toml",
                "setup.py",
                "setup.cfg",
                "requirements.txt",
            ],
            "pyright-langserver",
            &["--stdio"],
        ),
        lang(
            "typescript",
            None,
            &["ts", "mts", "cts"],
            "//",
            js_roots,
            ts[0],
            &ts[1..],
        ),
        lang(
            "tsx",
            Some("typescriptreact"),
            &["tsx"],
            "//",
            js_roots,
            ts[0],
            &ts[1..],
        ),
        lang(
            "javascript",
            None,
            &["js", "mjs", "cjs"],
            "//",
            js_roots,
            ts[0],
            &ts[1..],
        ),
        lang(
            "jsx",
            Some("javascriptreact"),
            &["jsx"],
            "//",
            js_roots,
            ts[0],
            &ts[1..],
        ),
        lang("go", None, &["go"], "//", &["go.mod"], "gopls", &[]),
        lang(
            "c",
            None,
            &["c", "h"],
            "//",
            &["compile_commands.json", "CMakeLists.txt", "Makefile"],
            "clangd",
            &[],
        ),
        lang(
            "cpp",
            None,
            &["cc", "cpp", "cxx", "hpp", "hh", "hxx"],
            "//",
            &["compile_commands.json", "CMakeLists.txt", "Makefile"],
            "clangd",
            &[],
        ),
    ]
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

impl Config {
    pub fn load() -> Config {
        Self::load_from(config_path())
    }

    pub fn load_from(path: Option<PathBuf>) -> Config {
        let mut c = Config {
            editor: EditorConfig::default(),
            languages: default_languages(),
            path: path.clone(),
            error: None,
        };
        if let Some(p) = &path {
            if let Ok(text) = std::fs::read_to_string(p) {
                if let Err(e) = c.merge_str(&text) {
                    c.error = Some(format!("{}: {e}", p.display()));
                }
            }
        }
        c
    }

    /// Merge a TOML document: `[editor]` replaces editor settings; each `[[languages]]`
    /// entry replaces the built-in language of the same name or adds a new one.
    pub fn merge_str(&mut self, text: &str) -> Result<(), String> {
        let f: FileConfig = toml::from_str(text).map_err(|e| e.message().to_string())?;
        if let Some(e) = f.editor {
            self.editor = e;
        }
        for l in f.languages {
            match self.languages.iter_mut().find(|x| x.name == l.name) {
                Some(slot) => *slot = l,
                None => self.languages.push(l),
            }
        }
        Ok(())
    }

    pub fn language_for(&self, path: &Path) -> Option<&LanguageConfig> {
        // Later entries (user-added) win over earlier ones on conflicting extensions.
        self.languages.iter().rev().find(|l| l.matches(path))
    }

    pub fn language(&self, name: &str) -> Option<&LanguageConfig> {
        self.languages.iter().find(|l| l.name == name)
    }
}

pub const TEMPLATE: &str = r##"# farsub configuration. Save this file to apply changes.

[editor]
font_size = 14.0
tab_size = 4
use_tabs = false
show_sidebar = true
# Any theme bundled with bat/two-face, e.g. "Monokai Extended", "OneHalfDark", "Nord",
# "Dracula", "Solarized (dark)", "GitHub", "Visual Studio Dark+".
theme = "Monokai Extended"

# Built-in languages: rust, python, typescript, tsx, javascript, jsx, go, c, cpp.
# A [[languages]] entry with the same name replaces the built-in one; a new name adds a
# language.
#
# [[languages]]
# name = "python"
# extensions = ["py", "pyi"]
# comment = "#"
# roots = ["pyproject.toml", "setup.py"]
# server = { command = "pylsp" }
#
# [[languages]]
# name = "lua"
# extensions = ["lua"]
# comment = "--"
# tab_size = 2
# roots = [".luarc.json"]
# [languages.server]
# command = "lua-language-server"
# [languages.server.settings.Lua]
# diagnostics = { globals = ["vim"] }
#
# [[languages]]
# name = "rust"
# extensions = ["rs"]
# comment = "//"
# roots = ["Cargo.toml"]
# [languages.server]
# command = "rust-analyzer"
# [languages.server.initialization_options]
# check = { command = "clippy" }
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_overrides() {
        let mut c = Config {
            languages: default_languages(),
            ..Default::default()
        };
        assert_eq!(c.language_for(Path::new("a/b.rs")).unwrap().name, "rust");
        assert_eq!(
            c.language_for(Path::new("x.tsx")).unwrap().language_id(),
            "typescriptreact"
        );
        assert!(c.language_for(Path::new("x.lua")).is_none());
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
            [languages.server]
            command = "lua-language-server"
            [languages.server.settings.Lua]
            diagnostics = { globals = ["vim"] }
            "#,
        )
        .unwrap();
        assert_eq!(c.editor.font_size, 16.0);
        assert_eq!(c.editor.tab_size, 4, "missing keys keep defaults");
        let py = c.language_for(Path::new("m.py")).unwrap();
        assert_eq!(py.server.as_ref().unwrap().command, "pylsp");
        let lua = c.language_for(Path::new("init.lua")).unwrap();
        assert_eq!(lua.comment.as_deref(), Some("--"));
        assert!(lua.server.as_ref().unwrap().settings.is_some());
        assert!(c.merge_str("[[languages]]\nextensions = 3").is_err());
        // The shipped template must parse.
        c.merge_str(TEMPLATE).unwrap();
    }
}
