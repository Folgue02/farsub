//! Workspaces: the session for a project (open files,
//! cursors, scroll positions, sidebar state, find text) plus "hot exit" contents of unsaved
//! buffers, so quitting never loses work and the next launch picks up where you left off.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct BufferState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Word wrap toggled by hand for this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    /// Editor group (layout column) the tab is in.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub group: usize,
    /// (line, col)
    #[serde(default)]
    pub cursor: (usize, usize),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<(usize, usize)>,
    #[serde(default)]
    pub scroll: (usize, usize),
    /// Unsaved text (hot exit). `None` means the file on disk is current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsaved: Option<String>,
    /// Syntax chosen by hand (View › Syntax).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax: Option<String>,
    /// Indentation chosen by hand: (use tabs, width).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<(bool, usize)>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Workspace {
    #[serde(default)]
    pub buffers: Vec<BufferState>,
    #[serde(default)]
    pub active: usize,
    #[serde(default = "yes")]
    pub sidebar_visible: bool,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default)]
    pub expanded: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default)]
    pub closed: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    pub console_visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_height: Option<f32>,
    /// Number of editor groups (layout columns).
    #[serde(default = "one")]
    pub layout: usize,
    #[serde(default)]
    pub focused_group: usize,
    /// The find panel's toggles and its Where / Replace texts.
    #[serde(default)]
    pub find_options: crate::search::FindOptions,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub find_where: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub find_replace: String,
}

fn one() -> usize {
    1
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn yes() -> bool {
    true
}

/// Write via a temp file + rename so a crash never leaves a truncated file.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

impl Workspace {
    pub fn load(path: &Path) -> Option<Workspace> {
        toml::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        write_atomic(
            path,
            &format!(
                "# farsub session state (open files, unsaved changes). Safe to delete.\n{text}"
            ),
        )
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let d = std::env::temp_dir().join(format!("farsub-ws-{}", std::process::id()));
        let w = Workspace {
            buffers: vec![
                BufferState {
                    path: Some("/x/a.rs".into()),
                    cursor: (3, 4),
                    anchor: Some((3, 1)),
                    scroll: (1, 0),
                    unsaved: None,
                    syntax: Some("Rust".into()),
                    indent: Some((false, 2)),
                    group: 1,
                    word_wrap: Some(false),
                },
                BufferState {
                    path: None,
                    unsaved: Some("draft \"quoted\"\nline 2".into()),
                    ..Default::default()
                },
            ],
            active: 1,
            sidebar_visible: false,
            expanded: vec!["/x/src".into()],
            search: Some("needle".into()),
            layout: 2,
            focused_group: 1,
            ..Default::default()
        };
        let f = d.join("t.farsub-workspace");
        w.save(&f).unwrap();
        assert_eq!(Workspace::load(&f), Some(w));
        assert!(Workspace::load(&d.join("missing")).is_none());
    }
}
