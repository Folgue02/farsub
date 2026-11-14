//! App: menu & keymap.

use super::*;

impl App {
    // ===================================================================== menu & keymap

    /// Whether a command can do anything right now (menus grey out the rest).
    pub fn enabled(&self, cmd: Command) -> bool {
        use Command::*;
        let has_buf = self.buf().is_some();
        let lsp = self.buf().is_some_and(|b| b.lsp_client.is_some());
        let in_panel = matches!(self.focus, Focus::Terminal | Focus::Console);
        match cmd {
            Copy | Paste | SelectAll if in_panel => true,
            CloseOtherTabs => has_buf && self.group_tabs(self.group).len() > 1,
            CloseTabsToRight => {
                has_buf
                    && self
                        .group_tabs(self.group)
                        .last()
                        .is_some_and(|&l| l > self.cur)
            }
            FocusGroup2 if self.layout < 2 => false,
            FocusGroup3 if self.layout < 3 => false,
            FocusGroup4 if self.layout < 4 => false,
            MoveToGroup1 | MoveToGroup2 | MoveToGroup3 | MoveToGroup4 => has_buf,
            FocusNextGroup => self.layout > 1,
            CopyFilePath | RevealInFileManager => {
                (self.focus == Focus::Sidebar && self.tree.selected_path().is_some())
                    || self.buf().is_some_and(|b| b.path.is_some())
            }
            ClearTerminal => self.terminal_shown().is_some(),
            Copy | Paste | SelectAll => has_buf,
            SwitchBuffer => !self.buffers.is_empty(),
            c if needs_buffer(c) => has_buf,
            Completion | Hover | GotoDefinition | FindReferences | Rename | Format => lsp,
            SaveAll => self.any_modified(),
            ReopenClosed => !self.closed.is_empty(),
            JumpBack => !self.jumps.is_empty(),
            SidebarPaste => self.file_clip.is_some(),
            RemovePackage => !self.packages.is_empty(),
            _ => true,
        }
    }

    /// Open a menu by index (mouse), or close it with `None`.
    pub fn set_menu(&mut self, menu: Option<usize>) {
        self.menu = menu.map(|m| MenuState {
            menu: m,
            item: first_enabled(self, m),
        });
    }

    pub fn menu_activate(&mut self, menu: usize, item: usize) {
        let cmd = self
            .menus
            .get(menu)
            .and_then(|m| m.items.get(item))
            .and_then(|i| i.as_ref())
            .map(|i| i.cmd);
        if let Some(cmd) = cmd {
            if self.enabled(cmd) {
                self.menu = None;
                self.run(cmd);
            }
        }
    }

    pub(super) fn menu_key(&mut self, k: Key) {
        let Some(mut m) = self.menu else { return };
        let n_menus = self.menus.len().max(1);
        let items: Vec<Option<Command>> = self
            .menus
            .get(m.menu)
            .map(|mm| mm.items.iter().map(|i| i.as_ref().map(|i| i.cmd)).collect())
            .unwrap_or_default();
        let step = |m: &MenuState, d: isize| -> usize {
            let n = items.len() as isize;
            let mut i = m.item as isize;
            for _ in 0..n {
                i = (i + d).rem_euclid(n);
                if let Some(Some(c)) = items.get(i as usize)
                    && self.enabled(*c)
                {
                    return i as usize;
                }
            }
            m.item
        };
        match k.code {
            KeyCode::Esc | KeyCode::F(10) => self.menu = None,
            KeyCode::Left => {
                m.menu = (m.menu + n_menus - 1) % n_menus;
                m.item = first_enabled(self, m.menu);
                self.menu = Some(m);
            }
            KeyCode::Right => {
                m.menu = (m.menu + 1) % n_menus;
                m.item = first_enabled(self, m.menu);
                self.menu = Some(m);
            }
            KeyCode::Up => {
                m.item = step(&m, -1);
                self.menu = Some(m);
            }
            KeyCode::Down => {
                m.item = step(&m, 1);
                self.menu = Some(m);
            }
            KeyCode::Home => {
                m.item = first_enabled(self, m.menu);
                self.menu = Some(m);
            }
            KeyCode::Enter => self.menu_activate(m.menu, m.item),
            _ => {}
        }
    }

    /// Typing a letter in an open menu jumps to (and runs) the item starting with it.
    pub(super) fn menu_char(&mut self, c: char) {
        let Some(m) = self.menu else { return };
        let c = c.to_ascii_lowercase();
        let Some(menu) = self.menus.get(m.menu) else {
            return;
        };
        let hits: Vec<usize> = menu
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.as_ref()
                    .is_some_and(|i| self.enabled(i.cmd) && i.label.to_lowercase().starts_with(c))
            })
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [] => {}
            [one] => self.menu_activate(m.menu, *one),
            many => {
                let next = many.iter().find(|&&i| i > m.item).unwrap_or(&many[0]);
                self.menu = Some(MenuState {
                    menu: m.menu,
                    item: *next,
                });
            }
        }
    }

    pub(super) fn keymap_key(&mut self, k: Key, mut v: KeymapView) {
        let n = keymap_rows(&self.keymap, &v.query).len();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                if let Some(cmd) = keymap_rows(&self.keymap, &v.query)
                    .get(v.sel)
                    .and_then(|r| r.cmd)
                {
                    self.run(cmd);
                    return;
                }
            }
            KeyCode::Up => v.sel = v.sel.saturating_sub(1),
            KeyCode::Down => v.sel = (v.sel + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp => v.sel = v.sel.saturating_sub(15),
            KeyCode::PageDown => v.sel = (v.sel + 15).min(n.saturating_sub(1)),
            KeyCode::Home => v.sel = 0,
            KeyCode::End => v.sel = n.saturating_sub(1),
            KeyCode::Backspace => {
                if k.ctrl {
                    v.query.clear();
                } else {
                    v.query.pop();
                }
                v.sel = 0;
            }
            _ => {}
        }
        self.modal = Some(Modal::Keymap(v));
    }
}
