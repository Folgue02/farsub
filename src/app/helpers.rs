//! App: free helpers (completion kinds, snippets, markdown, symbols, paths, locations).

use super::*;

pub(super) fn kind_name(k: i64) -> &'static str {
    const N: [&str; 27] = [
        "", "file", "mod", "ns", "pkg", "class", "method", "prop", "field", "ctor", "enum",
        "iface", "fn", "var", "const", "str", "num", "bool", "arr", "obj", "key", "null",
        "variant", "struct", "event", "op", "type",
    ];
    N.get(k as usize).copied().unwrap_or("")
}

pub(super) fn completion_kind(k: i64) -> &'static str {
    const N: [&str; 26] = [
        "", "txt", "meth", "fn", "ctor", "field", "var", "class", "iface", "mod", "prop", "unit",
        "val", "enum", "kw", "snip", "color", "file", "ref", "dir", "variant", "const", "struct",
        "event", "op", "type",
    ];
    N.get(k as usize).copied().unwrap_or("")
}

pub(super) fn kind_num<T: serde::Serialize>(k: &T) -> i64 {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// Strip LSP snippet syntax (`${1:name}`, `$0`) down to plain text.
pub(super) fn strip_snippet(s: &str) -> String {
    let mut out = String::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' && i + 1 < cs.len() {
            out.push(cs[i + 1]);
            i += 2;
        } else if c == '$' && i + 1 < cs.len() && cs[i + 1].is_ascii_digit() {
            i += 1;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
        } else if c == '$' && i + 1 < cs.len() && cs[i + 1] == '{' {
            i += 2;
            while i < cs.len() && cs[i].is_ascii_digit() {
                i += 1;
            }
            if i < cs.len() && (cs[i] == ':' || cs[i] == '|') {
                i += 1;
            }
            let mut depth = 1;
            while i < cs.len() {
                match cs[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    '|' | ',' if depth == 1 && out.ends_with(|ch: char| ch != ' ') => {}
                    ch => out.push(ch),
                }
                i += 1;
            }
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

pub(super) fn markdown_to_lines(s: &str) -> Vec<String> {
    let mut out = vec![];
    for l in s.lines() {
        if l.trim_start().starts_with("```") {
            continue;
        }
        out.push(l.replace("&nbsp;", " ").replace("\\_", "_"));
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.trim().is_empty()) {
        out.remove(0);
    }
    out
}

pub(super) fn parse_locations(v: Value) -> Vec<(PathBuf, u32, u32)> {
    let mut out = vec![];
    let arr = match v {
        Value::Array(a) => a,
        Value::Null => vec![],
        o => vec![o],
    };
    for l in arr {
        if let Ok(loc) = serde_json::from_value::<lt::Location>(l.clone()) {
            if let Some(p) = uri_to_path(loc.uri.as_str()) {
                out.push((p, loc.range.start.line, loc.range.start.character));
            }
        } else if let Ok(link) = serde_json::from_value::<lt::LocationLink>(l)
            && let Some(p) = uri_to_path(link.target_uri.as_str())
        {
            let r = link.target_selection_range.start;
            out.push((p, r.line, r.character));
        }
    }
    out
}

pub fn center(b: &mut Buffer, page: usize) {
    b.scroll_y = b.cursor.line.saturating_sub(page / 3);
}

/// Fallback "Goto Symbol" when no language server is available: keyword-based scan.
pub(super) fn heuristic_symbols(b: &Buffer) -> Vec<Item> {
    const PREFIXES: &[&str] = &[
        "pub(crate) ",
        "pub(super) ",
        "pub ",
        "export default ",
        "export ",
        "async ",
        "static ",
        "unsafe ",
        "public ",
        "private ",
        "protected ",
        "abstract ",
        "final ",
        "inline ",
        "extern \"C\" ",
    ];
    const KEYWORDS: &[&str] = &[
        "fn ",
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "impl<",
        "mod ",
        "macro_rules! ",
        "type ",
        "def ",
        "class ",
        "function ",
        "func ",
        "interface ",
        "module ",
        "union ",
        "const fn ",
    ];
    let md = b
        .path
        .as_ref()
        .and_then(|p| p.extension())
        .is_some_and(|e| e == "md" || e == "markdown");
    let mut out = vec![];
    for (i, line) in b.lines.iter().enumerate() {
        let t = line.trim_start();
        if md {
            if t.starts_with('#') {
                out.push(Item {
                    icon: None,
                    label: t.trim_start_matches('#').trim().to_string(),
                    detail: String::new(),
                    action: Action::Goto(Pos::new(i, 0)),
                });
            }
            continue;
        }
        let mut s = t;
        loop {
            match PREFIXES.iter().find(|p| s.starts_with(*p)) {
                Some(p) => s = &s[p.len()..],
                None => break,
            }
        }
        if let Some(k) = KEYWORDS.iter().find(|k| s.starts_with(*k)) {
            let indent = line.len() - t.len();
            let Some(name) = symbol_title(k, &s[k.len()..]) else {
                continue;
            };
            out.push(Item {
                icon: None,
                label: format!("{}{}", " ".repeat(indent.min(8)), name),
                detail: k.trim_end_matches([' ', '<', '!']).to_string(),
                action: Action::Goto(Pos::new(i, indent)),
            });
        }
    }
    out
}

/// The title Goto Symbol shows for a declaration: just the name (`add`, `Point`), or
/// for an impl block its subject (`impl Display for Point`), never the signature.
pub(super) fn symbol_title(keyword: &str, rest: &str) -> Option<String> {
    if keyword.starts_with("impl") {
        let mut r = rest;
        if keyword == "impl<" {
            // Skip the generic parameter list: impl<T: Clone> Foo<T>
            let mut depth = 1;
            let cut = r.char_indices().find(|&(_, c)| {
                match c {
                    '<' => depth += 1,
                    '>' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })?;
            r = r[cut.0 + 1..].trim_start();
        }
        let end = r.find(" where").or_else(|| r.find('{')).unwrap_or(r.len());
        let subject = r[..end].trim();
        return (!subject.is_empty()).then(|| format!("impl {subject}"));
    }
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Commands that act on the focused group's current file.
/// Commands that still do something while the current tab is a terminal.
pub(super) fn works_in_terminal_tab(cmd: Command) -> bool {
    use Command::*;
    if crate::commands::is_find_command(cmd) {
        return false;
    }
    !needs_buffer(cmd)
        && !matches!(
            cmd,
            ToggleWordWrap
                | SetSyntax
                | Indentation
                | GotoLine
                | GotoSymbol
                | RevertFile
                | ConvertToUtf8
                | Completion
                | Hover
                | GotoDefinition
                | FindReferences
                | Rename
                | Format
                | CodeActions
        )
        || matches!(cmd, CloseBuffer | Copy | Cut | Paste | SelectAll)
}

pub(super) fn needs_buffer(cmd: Command) -> bool {
    use Command::*;
    matches!(
        cmd,
        Save | SaveAs
            | CloseBuffer
            | Undo
            | Redo
            | Cut
            | SelectLine
            | SelectWord
            | SelectAllOccurrences
            | SplitIntoLines
            | AddCursorAbove
            | AddCursorBelow
            | DeleteLine
            | DuplicateLine
            | MoveLineUp
            | MoveLineDown
            | ToggleComment
            | Indent
            | Outdent
            | InsertLineAfter
            | InsertLineBefore
            | JoinLines
            | UpperCase
            | LowerCase
            | SortLines
            | GotoSymbol
            | GotoLine
            | NextBuffer
            | PrevBuffer
            | NextDiagnostic
            | PrevDiagnostic
            | RevealInSidebar
            | SetSyntax
            | Indentation
            | CloseOtherTabs
            | CloseTabsToRight
            | ToggleWordWrap
            | MoveLeft
            | MoveRight
            | MoveWordLeft
            | MoveWordRight
            | MoveLineStart
            | MoveLineEnd
            | MoveDocStart
            | MoveDocEnd
            | MoveUp
            | MoveDown
            | MovePageUp
            | MovePageDown
            | SelectLeft
            | SelectRight
            | SelectWordLeft
            | SelectWordRight
            | SelectToLineStart
            | SelectToLineEnd
            | SelectToDocStart
            | SelectToDocEnd
            | SelectUp
            | SelectDown
            | SelectPageUp
            | SelectPageDown
            | DeleteLeft
            | DeleteRight
            | DeleteWordLeft
            | DeleteWordRight
            | DeleteToLineStart
            | ScrollLineUp
            | ScrollLineDown
    )
}

pub(super) fn first_enabled(app: &App, menu: usize) -> usize {
    app.menus
        .get(menu)
        .and_then(|m| {
            m.items
                .iter()
                .position(|c| c.as_ref().is_some_and(|c| app.enabled(c.cmd)))
        })
        .unwrap_or(0)
}

/// Show a file or folder in the OS file manager (selecting it where supported).
pub(super) fn reveal_in_file_manager(p: &Path) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(p);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", p.display()));
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if p.is_dir() {
            p
        } else {
            p.parent().unwrap_or(p)
        });
        c
    };
    if let Err(e) = cmd.spawn() {
        crate::log::warn("app", format!("couldn't open the file manager: {e}"));
    }
}

/// Shell-style Tab completion for path prompts.
pub(super) fn complete_path(base: &Path, input: &str) -> String {
    let (dir_part, prefix) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    let dir = if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else if let Some(rest) = dir_part.strip_prefix("~/") {
        crate::paths::home_dir().unwrap_or_default().join(rest)
    } else {
        base.join(dir_part)
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return input.to_string();
    };
    let mut names: Vec<(String, bool)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            (n.starts_with(prefix) && (prefix.starts_with('.') || !n.starts_with('.')))
                .then(|| (n, e.path().is_dir()))
        })
        .collect();
    names.sort();
    match names.as_slice() {
        [] => input.to_string(),
        [(n, d)] => format!("{dir_part}{n}{}", if *d { "/" } else { "" }),
        many => {
            let first = &many[0].0;
            let mut common = first.len();
            for (n, _) in &many[1..] {
                common = common.min(
                    first
                        .bytes()
                        .zip(n.bytes())
                        .take_while(|(a, b)| a == b)
                        .count(),
                );
            }
            while !first.is_char_boundary(common) {
                common -= 1;
            }
            format!("{dir_part}{}", &first[..common])
        }
    }
}
