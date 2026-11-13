//! The one table of key bindings. Key handling, the command palette's shortcut hints, the
//! menu bar's shortcut column and the keymap viewer are all derived from it.
//!
//! Specs are written like "Cmd+Shift+P". `Cmd` is the primary modifier (Cmd on macOS, Ctrl
//! elsewhere; both always work), `Ctrl` is written where Sublime uses Ctrl on every platform
//! (it is matched the same way), plus `Shift` and `Alt`.

use crate::commands::Command::{self, *};
use crate::keys::{Key, KeyCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    /// Works wherever focus is (checked after the focused pane's own bindings).
    Global,
    Editor,
    Sidebar,
}

pub const BINDINGS: &[(&str, Command, Ctx)] = &[
    // ---- global
    ("Cmd+P", GotoFile, Ctx::Global),
    ("Cmd+Shift+P", ShowCommands, Ctx::Global),
    ("F1", ShowCommands, Ctx::Global),
    ("Cmd+R", GotoSymbol, Ctx::Global),
    ("Cmd+Shift+R", GotoWorkspaceSymbol, Ctx::Global),
    ("Ctrl+G", GotoLine, Ctx::Global),
    ("Cmd+E", SwitchBuffer, Ctx::Global),
    ("Cmd+N", NewFile, Ctx::Global),
    ("Cmd+O", OpenFolder, Ctx::Global),
    ("Cmd+Shift+T", ReopenClosed, Ctx::Global),
    ("Cmd+S", Save, Ctx::Global),
    ("Cmd+Shift+S", SaveAs, Ctx::Global),
    ("Cmd+Alt+S", SaveAll, Ctx::Global),
    ("Cmd+W", CloseBuffer, Ctx::Global),
    ("Cmd+,", OpenSettings, Ctx::Global),
    ("Cmd+Shift+/", ShowKeymap, Ctx::Global),
    ("F10", OpenMenu, Ctx::Global),
    ("Cmd+B", ToggleSidebar, Ctx::Global),
    ("Cmd+0", FocusSidebar, Ctx::Global),
    ("Cmd+Shift+E", RevealInSidebar, Ctx::Global),
    ("Ctrl+Tab", NextBuffer, Ctx::Global),
    ("Ctrl+PageDown", NextBuffer, Ctx::Global),
    ("Cmd+Alt+Right", NextBuffer, Ctx::Global),
    ("Ctrl+Shift+Tab", PrevBuffer, Ctx::Global),
    ("Ctrl+PageUp", PrevBuffer, Ctx::Global),
    ("Cmd+Alt+Left", PrevBuffer, Ctx::Global),
    ("Cmd+=", ZoomIn, Ctx::Global),
    ("Cmd+Shift+=", ZoomIn, Ctx::Global),
    ("Cmd+-", ZoomOut, Ctx::Global),
    ("Alt+-", JumpBack, Ctx::Global),
    ("Cmd+Q", Quit, Ctx::Global),
    // ---- editor
    ("Cmd+Z", Undo, Ctx::Editor),
    ("Cmd+Shift+Z", Redo, Ctx::Editor),
    ("Cmd+Y", Redo, Ctx::Editor),
    ("Cmd+X", Cut, Ctx::Editor),
    ("Cmd+C", Copy, Ctx::Editor),
    ("Cmd+V", Paste, Ctx::Editor),
    ("Cmd+A", SelectAll, Ctx::Editor),
    ("Cmd+L", SelectLine, Ctx::Editor),
    ("Cmd+D", SelectWord, Ctx::Editor),
    ("Cmd+Shift+D", DuplicateLine, Ctx::Editor),
    ("Cmd+Shift+K", DeleteLine, Ctx::Editor),
    ("Alt+Up", MoveLineUp, Ctx::Editor),
    ("Ctrl+Shift+Up", MoveLineUp, Ctx::Editor),
    ("Alt+Down", MoveLineDown, Ctx::Editor),
    ("Ctrl+Shift+Down", MoveLineDown, Ctx::Editor),
    ("Cmd+/", ToggleComment, Ctx::Editor),
    ("Cmd+]", Indent, Ctx::Editor),
    ("Cmd+[", Outdent, Ctx::Editor),
    ("Cmd+Enter", InsertLineAfter, Ctx::Editor),
    ("Cmd+Shift+Enter", InsertLineBefore, Ctx::Editor),
    ("Cmd+J", JoinLines, Ctx::Editor),
    ("F9", SortLines, Ctx::Editor),
    ("Cmd+F", Find, Ctx::Editor),
    ("F3", FindNext, Ctx::Editor),
    ("Shift+F3", FindPrev, Ctx::Editor),
    ("Cmd+Alt+F", Replace, Ctx::Editor),
    ("Cmd+H", Replace, Ctx::Editor),
    ("Ctrl+Space", Completion, Ctx::Editor),
    ("Cmd+I", Hover, Ctx::Editor),
    ("F12", GotoDefinition, Ctx::Editor),
    ("Shift+F12", FindReferences, Ctx::Editor),
    ("F2", Rename, Ctx::Editor),
    ("Alt+Shift+F", Format, Ctx::Editor),
    ("F8", NextDiagnostic, Ctx::Editor),
    ("Shift+F8", PrevDiagnostic, Ctx::Editor),
    // ---- sidebar (single letters work because they arrive as text)
    ("Up", SidebarUp, Ctx::Sidebar),
    ("K", SidebarUp, Ctx::Sidebar),
    ("Down", SidebarDown, Ctx::Sidebar),
    ("J", SidebarDown, Ctx::Sidebar),
    ("Shift+Up", SidebarMarkUp, Ctx::Sidebar),
    ("Shift+Down", SidebarMarkDown, Ctx::Sidebar),
    ("PageUp", SidebarPageUp, Ctx::Sidebar),
    ("PageDown", SidebarPageDown, Ctx::Sidebar),
    ("Home", SidebarTop, Ctx::Sidebar),
    ("G", SidebarTop, Ctx::Sidebar),
    ("End", SidebarBottom, Ctx::Sidebar),
    ("Shift+G", SidebarBottom, Ctx::Sidebar),
    ("Right", SidebarExpand, Ctx::Sidebar),
    ("L", SidebarExpand, Ctx::Sidebar),
    ("Left", SidebarCollapse, Ctx::Sidebar),
    ("H", SidebarCollapse, Ctx::Sidebar),
    ("Enter", SidebarOpen, Ctx::Sidebar),
    ("O", SidebarOpen, Ctx::Sidebar),
    ("Alt+Enter", SidebarOpenKeepFocus, Ctx::Sidebar),
    ("Space", SidebarMark, Ctx::Sidebar),
    ("A", SidebarNewFile, Ctx::Sidebar),
    ("Cmd+N", SidebarNewFile, Ctx::Sidebar),
    ("Shift+A", SidebarNewFolder, Ctx::Sidebar),
    ("Cmd+Shift+N", SidebarNewFolder, Ctx::Sidebar),
    ("R", SidebarRename, Ctx::Sidebar),
    ("F2", SidebarRename, Ctx::Sidebar),
    ("C", SidebarCopy, Ctx::Sidebar),
    ("Cmd+C", SidebarCopy, Ctx::Sidebar),
    ("X", SidebarCut, Ctx::Sidebar),
    ("Cmd+X", SidebarCut, Ctx::Sidebar),
    ("P", SidebarPaste, Ctx::Sidebar),
    ("V", SidebarPaste, Ctx::Sidebar),
    ("Cmd+V", SidebarPaste, Ctx::Sidebar),
    ("Shift+D", SidebarDuplicate, Ctx::Sidebar),
    ("Y", SidebarCopyPath, Ctx::Sidebar),
    ("D", SidebarDelete, Ctx::Sidebar),
    ("Delete", SidebarDelete, Ctx::Sidebar),
    ("Cmd+Backspace", SidebarDelete, Ctx::Sidebar),
    (".", ToggleHidden, Ctx::Sidebar),
    ("F5", RefreshSidebar, Ctx::Sidebar),
    ("Shift+R", RefreshSidebar, Ctx::Sidebar),
    ("Esc", SidebarEscape, Ctx::Sidebar),
    ("Tab", FocusEditor, Ctx::Sidebar),
];

/// Built-in keys that aren't commands, listed in the keymap viewer for completeness.
pub const DOC_KEYS: &[(&str, &str, &str)] = &[
    (
        "Editor",
        "Alt+Left / Alt+Right",
        "Move by word (Ctrl+Left/Right too)",
    ),
    ("Editor", "Cmd+Left / Cmd+Right", "Line start / end (macOS)"),
    ("Editor", "Home / End", "Smart line start / end"),
    (
        "Editor",
        "Cmd+Up / Cmd+Down",
        "Document start / end (Ctrl+Home/End too)",
    ),
    ("Editor", "Shift + any motion", "Extend the selection"),
    (
        "Editor",
        "Alt+Backspace / Alt+Delete",
        "Delete word left / right",
    ),
    ("Editor", "Cmd+Backspace", "Delete to line start"),
    (
        "Editor",
        "Tab / Shift+Tab",
        "Indent / unindent (selection or line)",
    ),
    (
        "Editor",
        "Ctrl+Up / Ctrl+Down",
        "Scroll without moving the cursor",
    ),
    ("Editor", "Cmd+1 … Cmd+9", "Go to tab 1–9"),
    ("Editor", "Esc", "Clear selection, close popups"),
    ("Palette", ">", "Commands (type in Goto Anything)"),
    ("Palette", "@", "Symbols in file"),
    ("Palette", ":", "Goto line"),
    ("Palette", "#", "Symbols in project"),
    ("Palette", "name:42", "Open file at line 42"),
    ("Palette", "Up / Down, Enter, Esc", "Choose, open, cancel"),
    ("Find panel", "Enter / Shift+Enter", "Next / previous match"),
    ("Find panel", "Esc", "Close and return to where you started"),
    ("Prompts", "Tab", "Complete a path"),
    ("Prompts", "Cmd+U", "Clear the input"),
    ("Menu bar", "F10", "Open the menu bar"),
    (
        "Menu bar",
        "Left / Right, Up / Down",
        "Move between menus and items",
    ),
    ("Menu bar", "Enter / Esc", "Run the item / close"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

pub fn parse(spec: &str) -> Option<Spec> {
    let mut s = Spec {
        code: KeyCode::Esc,
        ctrl: false,
        shift: false,
        alt: false,
    };
    // Split on '+' but keep a trailing "+" key ("Cmd++" isn't used; "=" is).
    let parts: Vec<&str> = spec.split('+').collect();
    let (key, mods) = parts.split_last()?;
    for m in mods {
        match *m {
            "Cmd" | "Ctrl" => s.ctrl = true,
            "Shift" => s.shift = true,
            "Alt" => s.alt = true,
            _ => return None,
        }
    }
    s.code = match *key {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Space" => KeyCode::Char(' '),
        k if k.len() > 1 && k.starts_with('F') => KeyCode::F(k[1..].parse().ok()?),
        k => {
            let mut cs = k.chars();
            let c = cs.next()?;
            if cs.next().is_some() {
                return None;
            }
            KeyCode::Char(c.to_ascii_lowercase())
        }
    };
    Some(s)
}

/// How a spec is shown to the user on this platform.
pub fn display(spec: &str) -> String {
    let primary = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    spec.split('+')
        .map(|p| if p == "Cmd" { primary } else { p })
        .collect::<Vec<_>>()
        .join("+")
}

fn matches(s: &Spec, k: &Key) -> bool {
    s.code == k.code && s.ctrl == k.ctrl && s.shift == k.shift && s.alt == k.alt
}

/// Binding for a key in a context (no Global fallback).
pub fn lookup(k: Key, ctx: Ctx) -> Option<Command> {
    BINDINGS
        .iter()
        .filter(|b| b.2 == ctx)
        .find(|b| parse(b.0).is_some_and(|s| matches(&s, &k)))
        .map(|b| b.1)
}

/// The key a typed character corresponds to (for single-letter sidebar commands).
pub fn key_for_char(c: char) -> Key {
    Key {
        code: KeyCode::Char(c.to_ascii_lowercase()),
        ctrl: false,
        alt: false,
        shift: c.is_ascii_uppercase(),
        sup: false,
    }
}

/// Display strings of every key bound to a command (context-specific first).
pub fn keys_for(cmd: Command) -> Vec<String> {
    BINDINGS
        .iter()
        .filter(|b| b.1 == cmd)
        .map(|b| display(b.0))
        .collect()
}

/// The primary shortcut for a command, for menus and the palette.
pub fn shortcut(cmd: Command) -> String {
    keys_for(cmd).into_iter().next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_binding_parses_and_none_conflict() {
        let mut seen = HashSet::new();
        for (spec, cmd, ctx) in BINDINGS {
            let s = parse(spec).unwrap_or_else(|| panic!("bad spec {spec}"));
            let key = (format!("{:?}", s), format!("{ctx:?}"));
            assert!(seen.insert(key), "{spec} bound twice in {ctx:?} ({cmd:?})");
        }
    }

    #[test]
    fn lookup_and_display() {
        let k = Key {
            code: KeyCode::Char('p'),
            ctrl: true,
            alt: false,
            shift: true,
            sup: true,
        };
        assert_eq!(lookup(k, Ctx::Global), Some(ShowCommands));
        assert_eq!(
            lookup(key_for_char('A'), Ctx::Sidebar),
            Some(SidebarNewFolder)
        );
        assert_eq!(
            lookup(key_for_char('a'), Ctx::Sidebar),
            Some(SidebarNewFile)
        );
        assert_eq!(lookup(key_for_char('.'), Ctx::Sidebar), Some(ToggleHidden));
        assert!(shortcut(Save).ends_with("+S"));
        assert_eq!(parse("Cmd+Shift+/").unwrap().code, KeyCode::Char('/'));
        assert_eq!(parse("F12").unwrap().code, KeyCode::F(12));
        assert!(parse("Hyper+X").is_none());
    }
}
