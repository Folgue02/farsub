//! App: the command dispatcher (`run`).

use super::*;

impl App {
    // ===================================================================== commands

    pub fn run(&mut self, cmd: Command) {
        use Command::*;
        let page = self.page;
        // With an input focused (find, palette, console line…), text commands edit it, from
        // keys, menus and the palette alike.
        if crate::lineedit::accepts(cmd) && self.has_input() {
            if !self.input_command(cmd) && cmd == SelectAll && self.modal.is_none() {
                // Select All with an empty console line selects the log.
                self.console_select_all();
            }
            return;
        }
        // Editing commands need a file in the focused group (it may be an empty column).
        if needs_buffer(cmd) && self.buf().is_none() {
            return;
        }
        match cmd {
            ToggleWordWrap => {
                let default = self.config.editor.word_wrap;
                if let Some(b) = self.buf_mut() {
                    let on = !b.word_wrap.unwrap_or(default);
                    b.word_wrap = Some(on);
                    b.scroll_x = 0;
                    b.scroll_row = 0;
                    self.msg(if on { "Word wrap on" } else { "Word wrap off" });
                }
            }
            LayoutSingle => self.set_layout(1),
            LayoutColumns2 => self.set_layout(2),
            LayoutColumns3 => self.set_layout(3),
            LayoutColumns4 => self.set_layout(4),
            FocusGroup1 => self.focus_group(0),
            FocusGroup2 => self.focus_group(1),
            FocusGroup3 => self.focus_group(2),
            FocusGroup4 => self.focus_group(3),
            MoveToGroup1 => self.move_to_group(0),
            MoveToGroup2 => self.move_to_group(1),
            MoveToGroup3 => self.move_to_group(2),
            MoveToGroup4 => self.move_to_group(3),
            FocusNextGroup => self.focus_group((self.group + 1) % self.layout),
            GotoFile => self.open_palette(Mode::Files),
            ShowCommands => self.open_palette(Mode::Commands),
            GotoSymbol => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                self.focus = Focus::Editor;
                self.open_palette(Mode::Symbols)
            }
            GotoWorkspaceSymbol => self.open_palette(Mode::WorkspaceSymbols),
            GotoLine => {
                if self.buffers.is_empty() {
                    return;
                }
                self.focus = Focus::Editor;
                self.open_palette(Mode::Line)
            }
            SwitchBuffer => {
                if self.buffers.is_empty() {
                    return self.msg("No open files");
                }
                let mut p = Palette::new(Mode::Buffers, "");
                p.title = "Open files".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            SelectTheme => {
                self.theme_before = Some(self.config.editor.theme.clone());
                let mut p = Palette::new(Mode::Themes, "");
                p.title = "arrows preview, Enter applies".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
                let cur = self.config.editor.theme.clone();
                if let Some(Modal::Palette(p)) = &mut self.modal
                    && let Some(i) = p.shown.iter().position(|(i, _)| p.items[*i].label == cur)
                {
                    p.sel = i;
                }
            }
            NewFile => {
                let id = self.next_buf_id;
                self.next_buf_id += 1;
                let b = Buffer::from_text(id, None, "", HlCache::new("Plain Text".into()));
                let i = self.push_buffer(b);
                self.set_cur(i);
                self.focus = Focus::Editor;
            }
            OpenFolder => {
                let def = format!("{}/", self.base_dir().display());
                self.prompt(
                    "Open folder or file  (Tab completes)",
                    &def,
                    PromptKind::OpenPath,
                );
            }
            OpenSettings => self.open_settings(),
            ReopenClosed => {
                while let Some(p) = self.closed.pop() {
                    if p.is_file() {
                        self.open_file(&p, true);
                        return;
                    }
                }
                self.msg("No recently closed files");
            }
            Quit => {
                let n = self.buffers.iter().filter(|b| b.modified).count();
                if self.config.editor.hot_exit {
                    // Unsaved changes are kept in the workspace and restored next time.
                    self.save_workspace();
                    self.quit = true;
                } else if n > 0 {
                    self.modal = Some(Modal::Confirm(Confirm {
                        message: format!("{n} file(s) have unsaved changes. Quit anyway?"),
                        kind: ConfirmKind::Quit,
                    }));
                } else {
                    self.quit = true;
                }
            }
            NextBuffer | PrevBuffer => {
                if self.buf().is_some() {
                    self.cycle_tab(if cmd == NextBuffer { 1 } else { -1 });
                    self.focus = Focus::Editor;
                    self.completion = None;
                    self.hover = None;
                }
            }
            ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                if !self.sidebar_visible {
                    self.focus = Focus::Editor;
                }
            }
            FocusSidebar => {
                if self.focus == Focus::Sidebar && self.sidebar_visible {
                    self.focus = Focus::Editor;
                } else {
                    self.focus_sidebar();
                }
            }
            FocusEditor => self.focus = Focus::Editor,
            RevealInSidebar => {
                self.tree_revealed = None;
                self.focus_sidebar();
            }
            ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.tree
                    .set_roots(self.project.folders.clone(), self.show_hidden);
                let s = if self.show_hidden { "shown" } else { "hidden" };
                self.msg(format!("Hidden files {s}"));
            }
            RefreshSidebar => {
                self.refresh_tree();
                self.msg("Sidebar refreshed");
            }
            SidebarUp | SidebarDown | SidebarPageUp | SidebarPageDown | SidebarTop
            | SidebarBottom | SidebarExpand | SidebarCollapse | SidebarOpen
            | SidebarOpenKeepFocus | SidebarMark | SidebarMarkUp | SidebarMarkDown
            | SidebarEscape => {
                if !self.sidebar_visible || self.focus != Focus::Sidebar {
                    self.focus_sidebar();
                }
                self.sidebar_nav(cmd);
            }
            OpenProject => {
                let def = format!("{}/", self.base_dir().display());
                self.prompt(
                    "Open project (.farsub-project) or folder  (Tab completes)",
                    &def,
                    PromptKind::OpenProject,
                );
            }
            SwitchProject => {
                let mut p = Palette::new(Mode::Projects, "");
                p.title = "Recent projects and folders".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            SaveProjectAs => {
                let base = self.base_dir();
                let name = self
                    .project
                    .file
                    .as_ref()
                    .map(|f| f.display().to_string())
                    .unwrap_or_else(|| {
                        let stem = base
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        base.join(format!("{stem}.{}", project::PROJECT_EXT))
                            .display()
                            .to_string()
                    });
                self.prompt("Save project as", &name, PromptKind::SaveProjectAs);
            }
            CloseProject => self.switch_project(Project::default()),
            AddFolder => {
                let def = format!(
                    "{}/",
                    self.base_dir().parent().unwrap_or(Path::new("/")).display()
                );
                self.prompt(
                    "Add folder to project  (Tab completes)",
                    &def,
                    PromptKind::AddFolder,
                );
            }
            RemoveFolder => {
                let target = self
                    .tree
                    .selected_path()
                    .and_then(|p| self.tree.root_of(p))
                    .map(|f| f.path.clone())
                    .or_else(|| {
                        (self.project.folders.len() == 1)
                            .then(|| self.project.folders[0].path.clone())
                    });
                match target {
                    Some(p) if self.project.remove_folder(&p) => {
                        self.project_folders_changed();
                        self.msg(format!("Removed {} from the project", p.display()));
                    }
                    _ => self.msg("Select a project folder in the sidebar first"),
                }
            }
            EditProject => match self.project.file.clone() {
                Some(f) => {
                    self.open_file(&f, true);
                }
                None => {
                    self.msg("This window has no project file yet — Project › Save Project As…")
                }
            },
            Indentation => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                let mut p = Palette::new(Mode::Indentation, "");
                p.title = "Indentation for this file".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            SetSyntax => {
                if self.buffers.is_empty() {
                    return self.msg("No open file");
                }
                let mut p = Palette::new(Mode::Syntaxes, "");
                p.title = "Set syntax for this file".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
                let cur = self.buf().map(|b| b.hl.syntax.clone()).unwrap_or_default();
                if let Some(Modal::Palette(p)) = &mut self.modal
                    && let Some(i) = p.shown.iter().position(|(i, _)| p.items[*i].label == cur)
                {
                    p.sel = i;
                }
            }
            OpenContextMenu => {
                let kind = match self.focus {
                    Focus::Sidebar if self.sidebar_visible => menu::ContextKind::Sidebar,
                    Focus::Terminal => menu::ContextKind::Terminal,
                    Focus::Console => menu::ContextKind::Console,
                    _ => menu::ContextKind::Editor,
                };
                self.open_context_menu_at(kind, None);
            }
            Copy | Cut | Paste | SelectAll
                if matches!(self.focus, Focus::Terminal | Focus::Console) =>
            {
                match cmd {
                    Copy => self.on_copy(false),
                    Cut => self.on_copy(true),
                    Paste => {
                        let t = self.clipboard_text();
                        if self.focus == Focus::Terminal {
                            if let Some(term) = self.current_terminal_mut() {
                                term.paste(&t);
                            }
                        } else {
                            self.on_paste(&t);
                        }
                    }
                    _ => {
                        if self.focus == Focus::Console {
                            self.console_select_all();
                        }
                    }
                }
            }
            CopyFilePath => {
                let p = self.context_path();
                if let Some(p) = p {
                    self.msg(format!("Copied {}", p.display()));
                    self.pending_copy = Some(p.display().to_string());
                }
            }
            RevealInFileManager => {
                if let Some(p) = self.context_path() {
                    reveal_in_file_manager(&p);
                }
            }
            OpenTerminalHere => {
                let dir = if self.focus == Focus::Sidebar {
                    self.tree.target_dir()
                } else {
                    self.buf()
                        .and_then(|b| b.path.as_ref())
                        .and_then(|p| p.parent())
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| self.base_dir())
                };
                self.new_terminal_in(&dir);
            }
            CloseOtherTabs | CloseTabsToRight => {
                let Some(keep_id) = self.buf().map(|b| b.id) else {
                    return;
                };
                let keep = self.cur;
                let ids: Vec<usize> = self
                    .group_tabs(self.group)
                    .into_iter()
                    .filter(|&i| {
                        if cmd == CloseOtherTabs {
                            i != keep
                        } else {
                            i > keep
                        }
                    })
                    .map(|i| self.buffers[i].id)
                    .collect();
                let mut skipped = 0;
                for id in ids {
                    let Some(i) = self.buf_index(id) else {
                        continue;
                    };
                    if self.buffers[i].modified && !self.config.editor.hot_exit {
                        skipped += 1;
                        continue;
                    }
                    if self.buffers[i].modified {
                        // Hot exit would lose these on close; keep them open instead.
                        skipped += 1;
                        continue;
                    }
                    self.close_buffer_at(i);
                }
                if let Some(i) = self.buf_index(keep_id) {
                    self.set_cur(i);
                }
                if skipped > 0 {
                    self.msg(format!("Kept {skipped} tab(s) with unsaved changes"));
                }
            }
            ClearTerminal => {
                if let Some(t) = self.current_terminal_mut() {
                    t.write(b"\x0c");
                }
            }
            ConsoleClear => crate::log::clear(),
            ConsoleToggleWrap => {
                self.console.wrap = !self.console.wrap;
                self.console.hscroll = 0.0;
            }
            ToggleConsole => {
                if self.console_shown() && self.focus == Focus::Console {
                    self.panel.visible = false;
                    self.focus = Focus::Editor;
                } else {
                    self.panel.visible = true;
                    self.panel.tab = PanelTab::Console;
                    self.console.scroll = 0;
                    self.focus = Focus::Console;
                }
            }
            ToggleTerminal => {
                let showing = self.panel.visible && matches!(self.panel.tab, PanelTab::Terminal(_));
                if showing && self.focus == Focus::Terminal {
                    self.panel.visible = false;
                    self.focus = Focus::Editor;
                } else {
                    let id = match self.panel.tab {
                        PanelTab::Terminal(id) if self.terminals.iter().any(|t| t.id == id) => {
                            Some(id)
                        }
                        _ => self.terminals.last().map(|t| t.id),
                    };
                    match id {
                        Some(id) => self.show_terminal(id),
                        None => self.new_terminal(),
                    }
                }
            }
            NewTerminal => self.new_terminal(),
            CloseTerminal => {
                if let PanelTab::Terminal(id) = self.panel.tab {
                    self.close_terminal(id);
                }
            }
            LspInfo => {
                self.completion = None;
                self.hover = None;
                self.modal = Some(Modal::LspInfo(LspInfoView { scroll: 0 }));
            }
            ManageServers => {
                self.completion = None;
                self.hover = None;
                self.modal = Some(Modal::Servers(ServersView {
                    sel: 0,
                    details: false,
                }));
            }
            InstallPackage => self.prompt(
                "Install package from git URL (Sublime packages with .sublime-syntax / .tmTheme)",
                "https://github.com/",
                PromptKind::PackageUrl,
            ),
            RemovePackage => {
                if self.packages.is_empty() {
                    return self.msg("No packages installed");
                }
                let mut p = Palette::new(Mode::Packages, "");
                p.title = "Remove package".into();
                self.modal = Some(Modal::Palette(p));
                self.populate_palette();
            }
            BrowsePackages => {
                if let Some(d) = self.config.packages_dir() {
                    packages::reveal(&d);
                }
            }
            ReloadPackages => {
                // Icon packages live in the config (icons.toml files).
                self.reload_config();
                let dir = self.config.packages_dir();
                self.packages = dir.as_deref().map(packages::list).unwrap_or_default();
                packages::load_async(dir, self.tx.clone());
            }
            ShowKeymap => {
                self.completion = None;
                self.hover = None;
                self.modal = Some(Modal::Keymap(KeymapView {
                    query: LineEdit::default(),
                    sel: 0,
                    scroll: 0,
                }));
            }
            OpenMenu => {
                self.completion = None;
                self.hover = None;
                self.menu = Some(MenuState {
                    menu: 0,
                    item: first_enabled(self, 0),
                });
            }
            ToggleMenuBar => {
                self.menu_bar_visible = !self.menu_bar_visible;
                if !self.menu_bar_visible {
                    self.msg("Menu bar hidden — F10 still opens it");
                }
            }
            SidebarNewFile | SidebarNewFolder | SidebarRename | SidebarDelete | SidebarCopy
            | SidebarCut | SidebarPaste | SidebarDuplicate | SidebarCopyPath => {
                if !self.sidebar_visible {
                    self.focus_sidebar();
                }
                self.run_sidebar(cmd);
            }
            ZoomIn => self.font_size = (self.font_size + 1.0).min(40.0),
            ZoomOut => self.font_size = (self.font_size - 1.0).max(8.0),
            ZoomReset => self.font_size = self.config.editor.font_size,
            JumpBack => {
                if let Some((path, pos)) = self.jumps.pop()
                    && let Some(i) = self.open_file(&path, true)
                {
                    let b = &mut self.buffers[i];
                    b.goto(pos);
                    center(b, page);
                }
            }
            RestartLsp => {
                self.restart_lsp();
                self.msg("Language servers restarted");
            }
            SaveAll => {
                let (cur, group) = (self.cur, self.group);
                for i in 0..self.buffers.len() {
                    if self.buffers[i].modified && self.buffers[i].path.is_some() {
                        self.set_cur(i);
                        self.save_current();
                    }
                }
                self.cur = cur;
                self.group = group;
            }
            _ => self.run_editor(cmd),
        }
    }
}
