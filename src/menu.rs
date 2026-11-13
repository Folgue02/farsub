//! Menus, built from runtime/menus.toml (or the user's menus.toml). Every item is a
//! command, so menus, the palette and shortcuts never disagree.

use crate::commands::{self, Command, Command::*};
use crate::config::MenuDef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub menu: usize,
    pub item: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub cmd: Command,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Menu {
    pub title: String,
    /// `None` is a separator.
    pub items: Vec<Option<Item>>,
}

pub fn build(defs: &[MenuDef]) -> (Vec<Menu>, Vec<String>) {
    let mut errs = vec![];
    let menus = defs
        .iter()
        .map(|d| Menu {
            title: d.title.clone(),
            items: d
                .items
                .iter()
                .filter_map(|s| {
                    if s == "-" {
                        return Some(None);
                    }
                    let (id, label) = match s.split_once(':') {
                        Some((i, l)) => (i.trim(), Some(l.trim().to_string())),
                        None => (s.trim(), None),
                    };
                    match commands::from_id(id) {
                        Some(cmd) => Some(Some(Item {
                            cmd,
                            label: label.unwrap_or_else(|| default_label(cmd)),
                        })),
                        None => {
                            errs.push(format!("menu {}: unknown command \"{id}\"", d.title));
                            None
                        }
                    }
                })
                .collect(),
        })
        .collect();
    (menus, errs)
}

/// The command name without its "Area: " prefix, plus a few nicer wordings.
pub fn default_label(cmd: Command) -> String {
    let special = match cmd {
        GotoFile => Some("Goto Anything…"),
        ShowCommands => Some("Command Palette…"),
        GotoSymbol => Some("Goto Symbol…"),
        GotoWorkspaceSymbol => Some("Goto Symbol in Project…"),
        GotoLine => Some("Goto Line…"),
        SwitchBuffer => Some("Switch Open File…"),
        UpperCase => Some("Convert to Upper Case"),
        LowerCase => Some("Convert to Lower Case"),
        Rename => Some("Rename Symbol…"),
        ShowKeymap => Some("Keyboard Shortcuts"),
        SelectTheme => Some("Color Theme…"),
        OpenSettings => Some("Settings & Languages…"),
        ManageServers => Some("Language Servers…"),
        _ => None,
    };
    if let Some(s) = special {
        return s.to_string();
    }
    let n = commands::name(cmd);
    n.split_once(": ").map(|(_, r)| r).unwrap_or(n).to_string()
}
