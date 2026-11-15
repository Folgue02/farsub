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

/// Where a right-click menu was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextKind {
    Editor,
    Sidebar,
    Tab,
    Terminal,
    Console,
}

impl ContextKind {
    pub fn name(self) -> &'static str {
        match self {
            ContextKind::Editor => "editor",
            ContextKind::Sidebar => "sidebar",
            ContextKind::Tab => "tab",
            ContextKind::Terminal => "terminal",
            ContextKind::Console => "console",
        }
    }
}

/// An open right-click menu. `pos` is a screen position, or `None` to place it at the
/// keyboard location (cursor / selected row) when opened with Shift+F10 or the Menu key.
#[derive(Clone, Debug)]
pub struct ContextMenu {
    pub kind: ContextKind,
    pub items: Vec<Option<Item>>,
    pub pos: Option<(f32, f32)>,
    pub sel: Option<usize>,
}

/// Build the right-click menus from config (same item syntax as the menu bar).
pub fn build_context(
    defs: &std::collections::HashMap<String, Vec<String>>,
) -> (
    std::collections::HashMap<String, Vec<Option<Item>>>,
    Vec<String>,
) {
    let mut errs = vec![];
    let mut out = std::collections::HashMap::new();
    for (name, items) in defs {
        let (m, e) = build(&[MenuDef {
            title: format!("context.{name}"),
            items: items.clone(),
        }]);
        errs.extend(e);
        out.insert(
            name.clone(),
            m.into_iter().next().map(|m| m.items).unwrap_or_default(),
        );
    }
    (out, errs)
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
        ShowKeymap => Some("Keyboard Shortcuts"),
        SelectTheme => Some("Color Theme…"),
        OpenSettings => Some("Settings & Languages…"),
        ManageServers => Some("Language Servers…"),
        RevealInFileManager if cfg!(target_os = "macos") => Some("Reveal in Finder"),
        RevealInFileManager if cfg!(windows) => Some("Reveal in File Explorer"),
        RevealInFileManager => Some("Open Containing Folder"),
        CloseBuffer => Some("Close"),
        Hover => Some("Show Hover Info"),
        Format => Some("Format Document"),
        Rename => Some("Rename Symbol…"),
        RemoveFolder => Some("Remove Folder from Project"),
        CloseTerminal => Some("Kill Terminal"),
        LspInfo => Some("LSP Info"),
        ToggleConsole => Some("Show Console"),
        ToggleTerminal => Some("Terminal"),
        NewTerminal => Some("New Terminal"),
        Indentation => Some("Indentation…"),
        SetSyntax => Some("Syntax…"),
        LayoutSingle => Some("Layout: Single"),
        LayoutColumns2 => Some("Layout: 2 Columns"),
        LayoutColumns3 => Some("Layout: 3 Columns"),
        LayoutColumns4 => Some("Layout: 4 Columns"),
        FocusGroup1 => Some("Focus Group 1"),
        FocusGroup2 => Some("Focus Group 2"),
        FocusGroup3 => Some("Focus Group 3"),
        FocusGroup4 => Some("Focus Group 4"),
        MoveToGroup1 => Some("Move File to Group 1"),
        MoveToGroup2 => Some("Move File to Group 2"),
        MoveToGroup3 => Some("Move File to Group 3"),
        MoveToGroup4 => Some("Move File to Group 4"),
        FocusNextGroup => Some("Focus Next Group"),
        ToggleWordWrap => Some("Word Wrap"),
        NewWindow => Some("New Window"),
        InstallGrammar => Some("Install Grammar…"),
        RemoveGrammar => Some("Remove Grammar…"),
        InstallGrammarFromFile => Some("Install Grammar from File…"),
        InstallGrammarFromFolder => Some("Install Grammar from Folder…"),
        InstallPackageFromFile => Some("Install Package from File…"),
        InstallPackageFromFolder => Some("Install Package from Folder…"),
        _ => None,
    };
    if let Some(s) = special {
        return s.to_string();
    }
    let n = commands::name(cmd);
    n.split_once(": ").map(|(_, r)| r).unwrap_or(n).to_string()
}

/// Which shortcut each menu item shows as a *native* accelerator. Native accelerators are
/// global (the OS menu grabs the key before the window sees it), so only global/editor
/// bindings qualify, and each key combination is used once — otherwise e.g. Sidebar › Cut
/// would steal Cmd+X from Edit › Cut while you're typing.
pub fn native_accelerators(
    menus: &[Menu],
    keymap: &crate::keymap::Keymap,
) -> std::collections::HashMap<Command, crate::keymap::Spec> {
    use crate::keymap::Ctx;
    use crate::keys::KeyCode;
    let mut used: Vec<crate::keymap::Spec> = vec![];
    let mut out = std::collections::HashMap::new();
    for m in menus {
        for it in m.items.iter().flatten() {
            let Some(b) = keymap.bindings.iter().find(|b| {
                b.cmd == it.cmd
                    && b.ctx != Ctx::Sidebar
                    && (b.spec.ctrl || b.spec.alt || matches!(b.spec.code, KeyCode::F(_)))
            }) else {
                continue;
            };
            let same = |s: &crate::keymap::Spec| {
                s.code == b.spec.code
                    && s.ctrl == b.spec.ctrl
                    && s.shift == b.spec.shift
                    && s.alt == b.spec.alt
            };
            if used.iter().any(same) || out.contains_key(&it.cmd) {
                continue;
            }
            used.push(b.spec);
            out.insert(it.cmd, b.spec);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_accelerators_never_steal_editor_keys() {
        let c = crate::config::Config::builtin();
        let (km, _) = crate::keymap::Keymap::from_config(&c);
        let (menus, errs) = build(&c.menus);
        assert!(errs.is_empty(), "{errs:?}");
        let acc = native_accelerators(&menus, &km);
        let x = acc.get(&Cut).expect("Edit › Cut has Cmd+X");
        assert_eq!(x.code, crate::keys::KeyCode::Char('x'));
        for cmd in [
            SidebarCut,
            SidebarCopy,
            SidebarPaste,
            SidebarNewFile,
            SidebarRename,
            SidebarDelete,
        ] {
            assert!(
                !acc.contains_key(&cmd),
                "{cmd:?} must not get a native accelerator"
            );
        }
        let mut seen = vec![];
        for s in acc.values() {
            let k = (format!("{:?}", s.code), s.ctrl, s.shift, s.alt);
            assert!(!seen.contains(&k), "duplicate accelerator {k:?}");
            seen.push(k);
        }
    }
}
