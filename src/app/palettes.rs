//! App: command palette, Goto Anything and the other palettes.

use super::*;

impl App {
    pub(super) fn open_palette(&mut self, mode: Mode) {
        let mut p = Palette::new(mode, Palette::prefix(mode));
        if matches!(mode, Mode::Symbols | Mode::Line)
            && let Some(b) = self.buf()
        {
            p.origin = Some((self.cur, b.cursor, b.scroll_y));
        }
        self.modal = Some(Modal::Palette(p));
        self.populate_palette();
    }

    pub(super) fn palette(&mut self) -> Option<&mut Palette> {
        match &mut self.modal {
            Some(Modal::Palette(p)) => Some(p),
            _ => None,
        }
    }

    /// Goto Anything items for every project folder that has been indexed. Returns true if
    /// some folders are still being indexed.
    pub(super) fn project_file_items(&mut self) -> (Vec<Item>, bool) {
        let multi = self.project.folders.len() > 1;
        let mut items = vec![];
        let mut loading = false;
        for f in self.project.folders.clone() {
            if !f.indexed() {
                // `index = false`: only what the sidebar has listed.
                let known = self.tree.known_files(&f.path);
                items.extend(known.into_iter().map(|rel| Item {
                    icon: None,
                    label: if multi {
                        format!("{}/{rel}", f.name)
                    } else {
                        rel.clone()
                    },
                    detail: String::new(),
                    action: Action::Open(f.path.join(&rel), None),
                }));
                continue;
            }
            match self.files.get(&f.path) {
                Some(list) => items.extend(list.iter().map(|rel| Item {
                    icon: None,
                    label: if multi {
                        format!("{}/{rel}", f.name)
                    } else {
                        rel.clone()
                    },
                    detail: String::new(),
                    action: Action::Open(f.path.join(rel), None),
                })),
                None => {
                    loading = true;
                    self.start_indexing(f);
                }
            }
        }
        (items, loading)
    }

    pub(super) fn start_indexing(&mut self, folder: crate::project::Folder) {
        if !self.indexing.insert(folder.path.clone()) {
            return;
        }
        let root = folder.path.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let excl = folder.clone();
            let walker = ignore::WalkBuilder::new(&root)
                .hidden(true)
                .git_ignore(true)
                // .gitignore applies even when the folder isn't a git repository.
                .require_git(false)
                .filter_entry(move |e| {
                    let name = e.file_name().to_string_lossy();
                    let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                    e.depth() == 0
                        || (!excl.excluded(&name, is_dir) && !name.ends_with(".farsub-workspace"))
                })
                .build();
            for e in walker.flatten() {
                if e.file_type().is_some_and(|t| t.is_file())
                    && let Ok(rel) = e.path().strip_prefix(&root)
                {
                    out.push(rel.to_string_lossy().into_owned());
                }
                if out.len() >= 200_000 {
                    break;
                }
            }
            let _ = tx.send(AppEvent::Files(root, out));
        });
    }

    pub(super) fn populate_palette(&mut self) {
        let Some(mode) = self.palette().map(|p| p.mode) else {
            return;
        };
        let mut loading = false;
        let mut title = String::new();
        let items: Vec<Item> = match mode {
            Mode::Files => {
                title = format!("Files in {}", self.project.name());
                if self.project.folders.iter().any(|f| !f.indexed()) {
                    title.push_str(" · unindexed folders: listed files only");
                }
                let (items, still) = self.project_file_items();
                loading = still;
                items
            }
            Mode::Commands => COMMANDS
                .iter()
                .filter(|c| c.2)
                .filter(|c| self.config.editor.wakatime || !App::wakatime_command(c.0))
                .map(|(c, name, _)| Item {
                    icon: None,
                    label: name.to_string(),
                    detail: self.keymap.shortcut(*c),
                    action: Action::Command(*c),
                })
                .collect(),
            Mode::Symbols => {
                title = "Symbols".into();
                let items = self.buf().map(heuristic_symbols).unwrap_or_default();
                if self.request_doc_symbols() {
                    loading = true;
                }
                items
            }
            Mode::Line => {
                title = "Goto line".into();
                vec![]
            }
            Mode::WorkspaceSymbols => {
                title = "Project symbols (type to search)".into();
                vec![]
            }
            Mode::Buffers => self
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| Item {
                    icon: None,
                    label: b.name(),
                    detail: b
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                        + if b.modified { "  (modified)" } else { "" },
                    action: Action::Buffer(i),
                })
                .collect(),
            Mode::Fonts => {
                let cur = self.config.editor.font_face.clone();
                let mut items = vec![Item {
                    icon: None,
                    label: "Built-in (Hack)".into(),
                    detail: if cur.is_empty() {
                        "current".into()
                    } else {
                        "monospace".into()
                    },
                    action: Action::Font(String::new()),
                }];
                items.extend(crate::fonts::families().into_iter().map(|f| Item {
                    icon: None,
                    detail: match (f.name == cur, f.mono) {
                        (true, _) => "current".into(),
                        (false, true) => "monospace".into(),
                        (false, false) => String::new(),
                    },
                    action: Action::Font(f.name.clone()),
                    label: f.name,
                }));
                items
            }
            Mode::Themes => {
                let cur = self.config.editor.theme.clone();
                self.syn
                    .theme_names()
                    .into_iter()
                    .map(|n| Item {
                        icon: None,
                        detail: if n == cur {
                            "current".into()
                        } else {
                            String::new()
                        },
                        action: Action::Theme(n.clone()),
                        label: n,
                    })
                    .collect()
            }
            Mode::Projects => {
                let mut v: Vec<Item> = self
                    .recent
                    .projects
                    .iter()
                    .filter(|p| p.is_file())
                    .map(|p| Item {
                        icon: None,
                        label: p
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        detail: p.display().to_string(),
                        action: Action::Project(Last::Project(p.clone())),
                    })
                    .collect();
                v.extend(
                    self.recent
                        .folders
                        .iter()
                        .filter(|fs| fs.iter().all(|f| f.is_dir()))
                        .map(|fs| Item {
                            icon: None,
                            label: fs
                                .iter()
                                .map(|f| {
                                    f.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned()
                                })
                                .collect::<Vec<_>>()
                                .join(", "),
                            detail: fs
                                .iter()
                                .map(|f| f.display().to_string())
                                .collect::<Vec<_>>()
                                .join("  ·  "),
                            action: Action::Project(Last::Folders(fs.clone())),
                        }),
                );
                v
            }
            Mode::Syntaxes => {
                let cur = self.buf().map(|b| b.hl.syntax.clone()).unwrap_or_default();
                let mut langs: Vec<&crate::config::LanguageConfig> =
                    self.config.languages.iter().collect();
                langs.sort_by_key(|l| l.title().to_lowercase());
                langs.dedup_by(|a, b| a.name == b.name);
                let mut v: Vec<Item> = langs
                    .into_iter()
                    .map(|l| Item {
                        icon: None,
                        label: l.title().to_string(),
                        detail: if l.name == cur {
                            "current".into()
                        } else {
                            l.extensions
                                .iter()
                                .take(4)
                                .map(|e| format!(".{e}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        },
                        action: Action::Syntax(l.name.clone()),
                    })
                    .collect();
                v.insert(
                    0,
                    Item {
                        icon: None,
                        label: "Plain Text".into(),
                        detail: if cur == "Plain Text" {
                            "current".into()
                        } else {
                            String::new()
                        },
                        action: Action::Syntax("Plain Text".into()),
                    },
                );
                v.sort_by_key(|i| i.label.to_lowercase());
                v.dedup_by(|a, b| a.label == b.label);
                v
            }
            Mode::Indentation => {
                let Some(b) = self.buf() else { return };
                let kind = b.indent_kind();
                let cur = |on: bool| {
                    if on {
                        "current".to_string()
                    } else {
                        String::new()
                    }
                };
                let guess = match crate::editor::guess_indent(&b.lines) {
                    Some(crate::editor::Indent::Tabs) => "detected: tabs".to_string(),
                    Some(crate::editor::Indent::Spaces(n)) => format!("detected: {n} spaces"),
                    None => "nothing to detect".into(),
                };
                let mut v = vec![
                    Item {
                        icon: None,
                        label: "Indent Using Spaces".into(),
                        detail: cur(kind != crate::editor::Indent::Tabs),
                        action: Action::Indent(IndentAction::Spaces),
                    },
                    Item {
                        icon: None,
                        label: "Indent Using Tabs".into(),
                        detail: cur(kind == crate::editor::Indent::Tabs),
                        action: Action::Indent(IndentAction::Tabs),
                    },
                ];
                for n in 1..=8 {
                    v.push(Item {
                        icon: None,
                        label: format!("Tab Width: {n}"),
                        detail: cur(b.tab_size == n),
                        action: Action::Indent(IndentAction::Width(n)),
                    });
                }
                v.push(Item {
                    icon: None,
                    label: "Guess Settings From Buffer".into(),
                    detail: guess,
                    action: Action::Indent(IndentAction::Guess),
                });
                v.push(Item {
                    icon: None,
                    label: "Convert Indentation to Spaces".into(),
                    detail: String::new(),
                    action: Action::Indent(IndentAction::ToSpaces),
                });
                v.push(Item {
                    icon: None,
                    label: "Convert Indentation to Tabs".into(),
                    detail: String::new(),
                    action: Action::Indent(IndentAction::ToTabs),
                });
                v
            }
            Mode::Grammars | Mode::GrammarsRemove => {
                let data = self.config.data_dir.clone();
                let mut v: Vec<Item> = vec![];
                for r in &self.config.grammars {
                    let installed = data.as_deref().is_some_and(|d| {
                        crate::grammar_install::installed_lib(d, &r.name).is_some()
                    });
                    if mode == Mode::GrammarsRemove && !installed {
                        continue;
                    }
                    // Label: the languages that use it ("F#", "OCaml"…).
                    let mut titles: Vec<&str> = self
                        .config
                        .languages
                        .iter()
                        .filter(|l| l.grammar_name() == r.name)
                        .map(|l| l.title())
                        .collect();
                    titles.dedup();
                    let label = if titles.is_empty() {
                        r.name.clone()
                    } else {
                        format!("{}  ({})", titles.join(", "), r.name)
                    };
                    let state = if self.installing_grammars.contains(&r.name) {
                        "installing…".to_string()
                    } else if installed {
                        format!("installed · {}", r.source_desc())
                    } else {
                        r.source_desc()
                    };
                    v.push(Item {
                        icon: None,
                        label,
                        detail: state,
                        action: if mode == Mode::Grammars {
                            Action::InstallGrammar(r.name.clone())
                        } else {
                            Action::RemoveGrammar(r.name.clone())
                        },
                    });
                }
                v.sort_by_key(|i| i.label.to_lowercase());
                v
            }
            Mode::Packages => self
                .packages
                .iter()
                .map(|n| Item {
                    icon: None,
                    label: n.clone(),
                    detail: String::new(),
                    action: Action::Package(n.clone()),
                })
                .collect(),
            Mode::Locations | Mode::CodeActions => return,
        };
        if let Some(p) = self.palette() {
            p.loading = loading;
            if !title.is_empty() {
                p.title = title;
            }
            p.set_items(items);
        }
        if mode == Mode::WorkspaceSymbols {
            self.request_workspace_symbols();
        }
    }

    pub(super) fn palette_query_changed(&mut self) {
        let Some(p) = self.palette() else { return };
        let new_mode = p.mode_for_query();
        if new_mode != p.mode {
            p.mode = new_mode;
            self.populate_palette();
        } else {
            p.refilter();
            if new_mode == Mode::WorkspaceSymbols {
                self.request_workspace_symbols();
            }
        }
        self.palette_preview();
    }

    pub(super) fn palette_preview(&mut self) {
        let Some(Modal::Palette(p)) = &self.modal else {
            return;
        };
        if p.mode == Mode::Themes {
            if let Some(Action::Theme(n)) = p.selected().map(|i| i.action.clone()) {
                self.apply_theme(&n);
            }
            return;
        }
        if p.mode == Mode::Fonts {
            // The GUI loads the font next frame.
            if let Some(Action::Font(f)) = p.selected().map(|i| i.action.clone()) {
                self.config.editor.font_face = f;
            }
            return;
        }
        let target = match p.mode {
            Mode::Symbols => match p.selected().map(|i| &i.action) {
                Some(Action::Goto(pos)) => Some(*pos),
                _ => None,
            },
            Mode::Line => p
                .needle()
                .trim()
                .parse::<usize>()
                .ok()
                .map(|n| Pos::new(n.saturating_sub(1), 0)),
            _ => None,
        };
        if let (Some(pos), Some((bi, _, _))) = (target, p.origin)
            && let Some(b) = self.buffers.get_mut(bi)
        {
            b.goto(pos);
            center(b, self.page);
        }
    }

    pub(super) fn palette_key(&mut self, k: Key) {
        let Some(p) = self.palette() else { return };
        match k.code {
            KeyCode::Esc => {
                let origin = p.origin;
                let mode = p.mode;
                if mode == Mode::Themes
                    && let Some(t) = self.theme_before.take()
                {
                    self.apply_theme(&t);
                    self.modal = None;
                    return;
                }
                if mode == Mode::Fonts
                    && let Some(f) = self.font_before.take()
                {
                    self.config.editor.font_face = f;
                    self.modal = None;
                    return;
                }
                if let Some((bi, pos, sy)) = origin
                    && let Some(b) = self.buffers.get_mut(bi)
                {
                    b.goto(pos);
                    b.scroll_y = sy;
                }
                self.modal = None;
            }
            KeyCode::Enter => {
                let Some(Modal::Palette(p)) = self.modal.take() else {
                    return;
                };
                self.palette_accept(p);
            }
            KeyCode::Up => {
                p.move_sel(-1);
                self.palette_preview();
            }
            KeyCode::Down | KeyCode::Tab => {
                p.move_sel(1);
                self.palette_preview();
            }
            KeyCode::Char('p') if k.ctrl && p.mode == Mode::Files => {
                p.move_sel(1);
            }
            KeyCode::PageUp => {
                p.move_sel(-10);
                self.palette_preview();
            }
            KeyCode::PageDown => {
                p.move_sel(10);
                self.palette_preview();
            }
            _ => {}
        }
    }

    pub(super) fn palette_accept(&mut self, p: Palette) {
        if p.mode == Mode::Line {
            if let Ok(n) = p.needle().trim().parse::<usize>()
                && let Some(b) = self.buf_mut()
            {
                b.goto(Pos::new(n.saturating_sub(1), 0));
                let pg = self.page;
                if let Some(b) = self.buf_mut() {
                    center(b, pg);
                }
            }
            return;
        }
        let line = p.file_line();
        let Some(item) = p.selected().cloned() else {
            return;
        };
        match item.action {
            Action::Command(c) => self.run(c),
            Action::Open(path, pos) => {
                let pos = pos.or(line.map(|l| Pos::new(l - 1, 0)));
                self.goto_location(&path, pos);
            }
            Action::Goto(pos) => {
                self.focus = Focus::Editor;
                self.push_jump();
                if let Some((bi, _, _)) = p.origin {
                    self.set_cur(bi);
                }
                let pg = self.page;
                if let Some(b) = self.buf_mut() {
                    b.goto(pos);
                    center(b, pg);
                }
            }
            Action::Buffer(i) => {
                if i < self.buffers.len() {
                    self.set_cur(i);
                    self.focus = Focus::Editor;
                }
            }
            Action::Project(last) => match last {
                Last::Project(f) => self.open_project_file(&f),
                Last::Folders(fs) => self.switch_project(Project::from_folders(fs)),
            },
            Action::Indent(a) => {
                let Some(b) = self.buf_mut() else { return };
                let note = match a {
                    IndentAction::Spaces => {
                        let w = b.tab_size;
                        b.set_indent(crate::editor::Indent::Spaces(w));
                        format!("Indent using {w} spaces")
                    }
                    IndentAction::Tabs => {
                        b.set_indent(crate::editor::Indent::Tabs);
                        "Indent using tabs".into()
                    }
                    IndentAction::Width(n) => {
                        b.set_tab_width(n);
                        format!("Tab width: {n}")
                    }
                    IndentAction::Guess => match b.guess_indentation() {
                        Some(crate::editor::Indent::Tabs) => "Detected: tabs".into(),
                        Some(crate::editor::Indent::Spaces(n)) => format!("Detected: {n} spaces"),
                        None => "No indentation to detect".into(),
                    },
                    IndentAction::ToSpaces => format!(
                        "Converted {} line(s) to spaces",
                        b.convert_indentation(false)
                    ),
                    IndentAction::ToTabs => {
                        format!("Converted {} line(s) to tabs", b.convert_indentation(true))
                    }
                };
                self.msg(note);
            }
            Action::Syntax(n) => {
                let i = self.cur;
                if i < self.buffers.len() {
                    self.set_syntax_at(i, &n);
                    let lang = self.buffers[i]
                        .lang
                        .clone()
                        .unwrap_or_else(|| "no language".into());
                    self.msg(format!("Syntax: {n} ({lang})"));
                }
            }
            Action::InstallGrammar(n) => self.install_grammar(&n),
            Action::RemoveGrammar(n) => {
                if let Some(data) = self.config.data_dir.clone() {
                    match crate::grammar_install::uninstall(&data, &n) {
                        Ok(()) => {
                            self.reload_syntax();
                            self.msg(format!("Removed the {n} grammar"));
                        }
                        Err(e) => self.msg(format!("Couldn't remove the {n} grammar: {e}")),
                    }
                }
            }
            Action::Package(n) => {
                self.modal = Some(Modal::Confirm(Confirm {
                    message: format!("Move package {n} to the Trash?"),
                    kind: ConfirmKind::RemovePackage(n),
                }));
            }
            Action::Theme(n) => {
                self.apply_theme(&n);
                self.theme_before = None;
                self.config.editor.theme = n.clone();
                self.keep_setting("theme", &n, &format!("Theme: {n}"));
            }
            Action::Font(f) => {
                self.font_before = None;
                self.config.editor.font_face = f.clone();
                let label = if f.is_empty() {
                    "the built-in font".to_string()
                } else {
                    f.clone()
                };
                self.keep_setting("font_face", &f, &format!("Font: {label}"));
            }
            Action::CodeAction(i) => self.run_code_action(i),
            Action::None => {}
        }
    }
}
