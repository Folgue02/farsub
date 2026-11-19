//! Projects: a `NAME.farsub-project` TOML file listing
//! folders (with names and exclude patterns), settings that override the editor settings,
//! and optionally project-only `[[languages]]` / `[[servers]]`.
//!
//! ```toml
//! [[folders]]
//! path = "."                      # relative to this file
//! name = "App"
//! folder_exclude_patterns = ["target", "node_modules"]
//! file_exclude_patterns = ["*.min.js"]
//!
//! [[folders]]
//! path = "../shared-lib"
//!
//! [settings]
//! tab_size = 2
//! ```
//! A window without a project file is an anonymous project: just a list of folders.

use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

pub const PROJECT_EXT: &str = "farsub-project";
pub const WORKSPACE_EXT: &str = "farsub-workspace";

/// What Project › New Project starts with.
pub const NEW_PROJECT: &str = r##"# A new project: save it as NAME.farsub-project (the example in the other pane shows
# everything a project file can hold). Then "Project: Open This Project in New Window"
# (also when right-clicking the file in the sidebar) opens it.

[[folders]]
path = "."                    # relative to where you save this file

[settings]
"##;

/// The read-only sample shown beside a project file (Project › Edit Project).
pub const EXAMPLE_PROJECT: &str = r##"# farsub example project (read-only reference).
#
# A project file (NAME.farsub-project) lists the folders a window shows and settings that
# apply only in that window. Don't edit this file: copy what you need into your project
# file (the other pane) and save it there; the project's window picks it up at once.

# ---- folders: one [[folders]] per folder in the sidebar
[[folders]]
path = "."                    # relative to the project file, or absolute
name = "App"                  # optional: the name shown in the sidebar
# Hidden from the sidebar, Goto Anything and Find in Files (names or globs):
folder_exclude_patterns = ["target", "node_modules", ".cache"]
file_exclude_patterns = ["*.min.js", "*.lock"]

[[folders]]
path = "../shared-lib"        # folders can live anywhere
# index = false               # a slow (network) folder: Goto Anything only offers the
                              # files the sidebar has listed, instead of reading it all

# ---- settings: any editor setting (Preferences: Settings lists them), only here
[settings]
tab_size = 2
use_tabs = false
format_on_save = true
# theme = "Tsoding"
# highlighter = { markdown = "tree-sitter" }

# ---- languages and servers for this project only (same format as the files in
# languages/ and servers/ of your config folder); an entry with the same name as a
# built-in one replaces it.
#
# [[languages]]
# name = "python"
# servers = ["basedpyright"]
#
# [[servers]]
# name = "basedpyright"
# command = "basedpyright-langserver"
# args = ["--stdio"]
# [servers.settings.basedpyright.analysis]
# typeCheckingMode = "strict"
"##;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct FolderDef {
    /// The folder, relative to the project file ("." is the file's own folder) or absolute.
    pub path: String,
    /// Name shown in the sidebar (default: the folder's name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Folders hidden from the sidebar and Goto Anything, by name or glob: ["target"].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folder_exclude_patterns: Vec<String>,
    /// Files hidden from the sidebar and Goto Anything, by name or glob: ["*.min.js"].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_exclude_patterns: Vec<String>,
    /// Index the folder's files in the background for Goto Anything (the default).
    /// `false` for a slow (network) folder: Goto Anything then offers the files the
    /// sidebar has listed so far, instead of reading the whole folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<bool>,
}

/// A resolved project folder.
#[derive(Clone, Debug)]
pub struct Folder {
    pub path: PathBuf,
    pub name: String,
    pub def: FolderDef,
    exclude_dirs: GlobSet,
    exclude_files: GlobSet,
}

impl PartialEq for Folder {
    fn eq(&self, o: &Self) -> bool {
        self.path == o.path && self.name == o.name && self.def == o.def
    }
}

fn globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

impl Folder {
    pub fn new(path: PathBuf, def: FolderDef) -> Folder {
        let name = def.name.clone().unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        });
        Folder {
            exclude_dirs: globset(&def.folder_exclude_patterns),
            exclude_files: globset(&def.file_exclude_patterns),
            path,
            name,
            def,
        }
    }

    pub fn plain(path: PathBuf) -> Folder {
        let def = FolderDef {
            path: path.display().to_string(),
            ..Default::default()
        };
        Folder::new(path, def)
    }

    /// `index` isn't `false`.
    pub fn indexed(&self) -> bool {
        self.def.index != Some(false)
    }

    /// Whether an entry (by name) inside this folder is excluded by the project.
    pub fn excluded(&self, name: &str, is_dir: bool) -> bool {
        if is_dir {
            self.exclude_dirs.is_match(name)
        } else {
            self.exclude_files.is_match(name)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Project {
    /// The `.farsub-project` file; `None` for an anonymous (folders only) project.
    pub file: Option<PathBuf>,
    pub folders: Vec<Folder>,
    /// `[settings]`: overrides for `[editor]` settings.
    pub settings: toml::Table,
    /// The whole file, so saving keeps keys we don't interpret (languages, servers, …).
    raw: toml::Table,
}

/// Lexically normalize `a/b/../c` → `a/c` (project paths may point at siblings).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `target` relative to `base`, using `..` where needed (both absolute).
pub fn relative_to(target: &Path, base: &Path) -> PathBuf {
    let t: Vec<_> = target.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = t.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

impl Project {
    pub fn from_folders(paths: Vec<PathBuf>) -> Project {
        let mut p = Project::default();
        for path in paths {
            p.add_folder(path);
        }
        p
    }

    pub fn load(file: &Path) -> Result<Project, String> {
        let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        Self::parse(file, &text)
    }

    pub fn parse(file: &Path, text: &str) -> Result<Project, String> {
        let raw: toml::Table =
            toml::from_str(text).map_err(|e| format!("{}: {}", file.display(), e.message()))?;
        let defs: Vec<FolderDef> = match raw.get("folders") {
            Some(v) => v.clone().try_into().map_err(|e: toml::de::Error| {
                format!("{}: folders: {}", file.display(), e.message())
            })?,
            None => vec![],
        };
        let base = file.parent().unwrap_or(Path::new("."));
        let folders = defs
            .into_iter()
            .map(|d| {
                let p = Path::new(&d.path);
                let abs = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    base.join(p)
                };
                let abs = abs.canonicalize().unwrap_or_else(|_| normalize(&abs));
                Folder::new(abs, d)
            })
            .collect();
        let settings = raw
            .get("settings")
            .and_then(|v| v.as_table())
            .cloned()
            .unwrap_or_default();
        Ok(Project {
            file: Some(file.to_path_buf()),
            folders,
            settings,
            raw,
        })
    }

    /// Display name: the project file's stem, or the folder names.
    pub fn name(&self) -> String {
        if let Some(f) = &self.file {
            return f
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .replace(&format!(".{PROJECT_EXT}"), "");
        }
        match self.folders.as_slice() {
            [] => "untitled".into(),
            fs => fs
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    pub fn primary(&self) -> Option<&Path> {
        self.folders.first().map(|f| f.path.as_path())
    }

    /// The folder containing a path (deepest match, for nested folders).
    pub fn folder_of(&self, p: &Path) -> Option<&Folder> {
        self.folders
            .iter()
            .filter(|f| p.starts_with(&f.path))
            .max_by_key(|f| f.path.components().count())
    }

    pub fn add_folder(&mut self, path: PathBuf) -> bool {
        if self.folders.iter().any(|f| f.path == path) {
            return false;
        }
        self.folders.push(Folder::plain(path));
        true
    }

    pub fn remove_folder(&mut self, path: &Path) -> bool {
        let n = self.folders.len();
        self.folders.retain(|f| f.path != path);
        n != self.folders.len()
    }

    /// Write the project file (folders relative to it), keeping other keys as they were.
    pub fn save_as(&mut self, file: &Path) -> Result<(), String> {
        let base = file.parent().unwrap_or(Path::new("."));
        let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
        let defs: Vec<FolderDef> = self
            .folders
            .iter()
            .map(|f| {
                let rel = relative_to(&f.path, &base);
                // Keep absolute paths for folders on another drive / far away.
                let path = if rel
                    .components()
                    .filter(|c| matches!(c, Component::ParentDir))
                    .count()
                    > 3
                {
                    f.path.display().to_string()
                } else {
                    rel.display().to_string()
                };
                FolderDef {
                    path,
                    ..f.def.clone()
                }
            })
            .collect();
        let mut raw = self.raw.clone();
        raw.insert(
            "folders".into(),
            toml::Value::try_from(&defs).map_err(|e| e.to_string())?,
        );
        if !self.settings.is_empty() {
            raw.insert("settings".into(), toml::Value::Table(self.settings.clone()));
        }
        let text = toml::to_string_pretty(&raw).map_err(|e| e.to_string())?;
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(file, text).map_err(|e| e.to_string())?;
        self.file = Some(file.to_path_buf());
        self.raw = raw;
        Ok(())
    }

    /// Project-level `[[languages]]` / `[[servers]]` as a config.toml snippet.
    pub fn config_snippet(&self) -> Option<String> {
        let mut t = toml::Table::new();
        for k in ["languages", "servers"] {
            if let Some(v) = self.raw.get(k) {
                t.insert(k.into(), v.clone());
            }
        }
        (!t.is_empty()).then(|| toml::to_string(&t).unwrap_or_default())
    }

    /// Where this project's session state is kept.
    pub fn workspace_file(&self, data: Option<&Path>) -> Option<PathBuf> {
        if let Some(f) = &self.file {
            return Some(f.with_extension(WORKSPACE_EXT));
        }
        if self.folders.is_empty() {
            return None;
        }
        // Anonymous projects: one workspace per folder set, in the data folder.
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for f in &self.folders {
            f.path.hash(&mut h);
        }
        let slug: String = self
            .name()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .take(40)
            .collect();
        Some(
            data?
                .join("workspaces")
                .join(format!("{slug}-{:016x}.{WORKSPACE_EXT}", h.finish())),
        )
    }
}

/// Apply `[settings]` over editor settings (unknown keys are reported).
pub fn apply_settings(
    editor: &crate::config::EditorConfig,
    settings: &toml::Table,
) -> Result<crate::config::EditorConfig, String> {
    let mut t = match toml::Value::try_from(editor) {
        Ok(toml::Value::Table(t)) => t,
        _ => return Err("can't apply settings".into()),
    };
    for (k, v) in settings {
        if !t.contains_key(k) {
            return Err(format!("unknown setting `{k}`"));
        }
        t.insert(k.clone(), v.clone());
    }
    toml::Value::Table(t)
        .try_into()
        .map_err(|e: toml::de::Error| e.message().to_string())
}

// ===================================================================== recent projects

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Recent {
    #[serde(default)]
    pub projects: Vec<PathBuf>,
    /// Anonymous projects (folder sets).
    #[serde(default)]
    pub folders: Vec<Vec<PathBuf>>,
    /// The workspace to restore when started without arguments.
    #[serde(default)]
    pub last: Option<Last>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum Last {
    Project(PathBuf),
    Folders(Vec<PathBuf>),
}

impl Recent {
    fn path(data: &Path) -> PathBuf {
        data.join("recent.toml")
    }

    pub fn load(data: Option<&Path>) -> Recent {
        data.and_then(|d| std::fs::read_to_string(Self::path(d)).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data: Option<&Path>) {
        let Some(d) = data else { return };
        let _ = std::fs::create_dir_all(d);
        if let Ok(t) = toml::to_string_pretty(self) {
            let _ = crate::workspace::write_atomic(&Self::path(d), &t);
        }
    }

    pub fn touch(&mut self, p: &Project) {
        let last = match &p.file {
            Some(f) => {
                self.projects.retain(|x| x != f);
                self.projects.insert(0, f.clone());
                self.projects.truncate(30);
                Last::Project(f.clone())
            }
            None if p.folders.is_empty() => return,
            None => {
                let fs: Vec<PathBuf> = p.folders.iter().map(|f| f.path.clone()).collect();
                self.folders.retain(|x| x != &fs);
                self.folders.insert(0, fs.clone());
                self.folders.truncate(30);
                Last::Folders(fs)
            }
        };
        self.last = Some(last);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_file_round_trip() {
        let d = std::env::temp_dir().join(format!("farsub-proj-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("app/target")).unwrap();
        std::fs::create_dir_all(d.join("lib")).unwrap();
        let d = d.canonicalize().unwrap();
        let file = d.join("app/My.farsub-project");
        std::fs::write(
            &file,
            r#"
            [[folders]]
            path = "."
            name = "App"
            folder_exclude_patterns = ["target"]
            file_exclude_patterns = ["*.min.js"]
            [[folders]]
            path = "../lib"
            [settings]
            tab_size = 2
            [[languages]]
            name = "lua"
            extensions = ["lua"]
            "#,
        )
        .unwrap();
        let mut p = Project::load(&file).unwrap();
        assert_eq!(p.name(), "My");
        assert_eq!(p.folders.len(), 2);
        assert_eq!(p.folders[0].name, "App");
        assert_eq!(p.folders[1].path, d.join("lib"));
        assert!(p.folders[0].excluded("target", true));
        assert!(!p.folders[0].excluded("target", false));
        assert!(p.folders[0].excluded("x.min.js", false));
        assert_eq!(p.folder_of(&d.join("lib/a.rs")).unwrap().name, "lib");
        assert!(p.config_snippet().unwrap().contains("lua"));
        let ed = apply_settings(&crate::config::EditorConfig::default(), &p.settings).unwrap();
        assert_eq!(ed.tab_size, 2);
        assert!(apply_settings(&ed, &toml::from_str("bogus = 1").unwrap()).is_err());

        // Save elsewhere: paths become relative to the new file; other keys survive.
        p.add_folder(d.join("app/target"));
        let other = d.join("My2.farsub-project");
        p.save_as(&other).unwrap();
        let q = Project::load(&other).unwrap();
        assert_eq!(
            q.folders.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
            p.folders.iter().map(|f| f.path.clone()).collect::<Vec<_>>()
        );
        let text = std::fs::read_to_string(&other).unwrap();
        assert!(
            text.contains("path = \"app\"") && text.contains("path = \"lib\""),
            "{text}"
        );
        assert!(text.contains("lua") && text.contains("tab_size"));
        assert_eq!(
            q.workspace_file(None).unwrap(),
            d.join("My2.farsub-workspace")
        );
        assert_eq!(
            relative_to(&d.join("lib"), &d.join("app")),
            PathBuf::from("../lib")
        );
    }
}
