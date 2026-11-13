//! Menu bar layout. Every item is a command, so menus, the palette and shortcuts never
//! disagree about what an action does or which key runs it.

use crate::commands::{self, Command, Command::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub menu: usize,
    pub item: usize,
}

/// `None` is a separator.
pub const MENUS: &[(&str, &[Option<Command>])] = &[
    (
        "File",
        &[
            Some(NewFile),
            Some(OpenFolder),
            Some(ReopenClosed),
            None,
            Some(Save),
            Some(SaveAs),
            Some(SaveAll),
            None,
            Some(CloseBuffer),
            None,
            Some(OpenSettings),
            Some(SelectTheme),
            None,
            Some(Quit),
        ],
    ),
    (
        "Edit",
        &[
            Some(Undo),
            Some(Redo),
            None,
            Some(Cut),
            Some(Copy),
            Some(Paste),
            None,
            Some(ToggleComment),
            Some(DuplicateLine),
            Some(DeleteLine),
            Some(MoveLineUp),
            Some(MoveLineDown),
            Some(JoinLines),
            Some(InsertLineAfter),
            Some(InsertLineBefore),
            None,
            Some(Indent),
            Some(Outdent),
            Some(SortLines),
            Some(UpperCase),
            Some(LowerCase),
        ],
    ),
    (
        "Selection",
        &[Some(SelectAll), Some(SelectLine), Some(SelectWord)],
    ),
    (
        "Find",
        &[
            Some(Find),
            Some(FindNext),
            Some(FindPrev),
            None,
            Some(Replace),
        ],
    ),
    (
        "Goto",
        &[
            Some(GotoFile),
            Some(GotoSymbol),
            Some(GotoWorkspaceSymbol),
            Some(GotoLine),
            Some(SwitchBuffer),
            None,
            Some(GotoDefinition),
            Some(FindReferences),
            Some(JumpBack),
            None,
            Some(NextDiagnostic),
            Some(PrevDiagnostic),
        ],
    ),
    (
        "Sidebar",
        &[
            Some(FocusSidebar),
            Some(RevealInSidebar),
            None,
            Some(SidebarNewFile),
            Some(SidebarNewFolder),
            Some(SidebarRename),
            Some(SidebarDuplicate),
            None,
            Some(SidebarCopy),
            Some(SidebarCut),
            Some(SidebarPaste),
            Some(SidebarCopyPath),
            None,
            Some(SidebarDelete),
            None,
            Some(ToggleHidden),
            Some(RefreshSidebar),
        ],
    ),
    (
        "View",
        &[
            Some(ToggleSidebar),
            Some(ToggleMenuBar),
            None,
            Some(NextBuffer),
            Some(PrevBuffer),
            None,
            Some(ZoomIn),
            Some(ZoomOut),
            Some(ZoomReset),
        ],
    ),
    (
        "Tools",
        &[
            Some(ShowCommands),
            None,
            Some(Completion),
            Some(Hover),
            Some(Rename),
            Some(Format),
            None,
            Some(RestartLsp),
        ],
    ),
    ("Help", &[Some(ShowKeymap), Some(ShowCommands)]),
];

/// Item label: the command name without its "Area: " prefix, plus a few nicer wordings.
pub fn label(cmd: Command) -> String {
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
        _ => None,
    };
    if let Some(s) = special {
        return s.to_string();
    }
    let n = commands::name(cmd);
    n.split_once(": ").map(|(_, r)| r).unwrap_or(n).to_string()
}

pub fn items(menu: usize) -> &'static [Option<Command>] {
    MENUS.get(menu).map(|m| m.1).unwrap_or(&[])
}
