//! App: context menus.

use super::*;

impl App {
    // ===================================================================== context menus

    /// Open the right-click menu for a place, at a screen position (or the keyboard spot).
    pub fn open_context_menu(&mut self, kind: menu::ContextKind, pos: (f32, f32)) {
        self.open_context_menu_at(kind, Some(pos));
    }

    pub fn open_context_menu_at(&mut self, kind: menu::ContextKind, pos: Option<(f32, f32)>) {
        let mut items = self.contexts.get(kind.name()).cloned().unwrap_or_default();
        // Items only for some targets are left out elsewhere (not shown greyed out).
        items.retain(|i| i.as_ref().is_none_or(|i| !only_when_enabled(i.cmd) || self.enabled(i.cmd)));
        tidy_separators(&mut items);
        if items.is_empty() {
            return;
        }
        self.completion = None;
        self.hover = None;
        self.menu = None;
        let sel = pos
            .is_none()
            .then(|| {
                items
                    .iter()
                    .position(|i| i.as_ref().is_some_and(|i| self.enabled(i.cmd)))
            })
            .flatten();
        self.context_menu = Some(menu::ContextMenu {
            kind,
            items,
            pos,
            sel,
        });
    }

    pub fn context_activate(&mut self, i: usize) {
        let Some(cm) = self.context_menu.take() else {
            return;
        };
        if let Some(Some(it)) = cm.items.get(i)
            && self.enabled(it.cmd)
        {
            self.run(it.cmd);
        }
    }

    pub(super) fn context_key(&mut self, k: Key) {
        let Some(cm) = &self.context_menu else { return };
        let n = cm.items.len();
        let enabled: Vec<bool> = cm
            .items
            .iter()
            .map(|i| i.as_ref().is_some_and(|i| self.enabled(i.cmd)))
            .collect();
        let step = |from: Option<usize>, d: isize| -> Option<usize> {
            let mut i = from
                .map(|x| x as isize)
                .unwrap_or(if d > 0 { -1 } else { n as isize });
            for _ in 0..n {
                i = (i + d).rem_euclid(n as isize);
                if enabled[i as usize] {
                    return Some(i as usize);
                }
            }
            from
        };
        let sel = cm.sel;
        match k.code {
            KeyCode::Esc => self.context_menu = None,
            KeyCode::Up => self.context_menu.as_mut().unwrap().sel = step(sel, -1),
            KeyCode::Down => self.context_menu.as_mut().unwrap().sel = step(sel, 1),
            KeyCode::Enter => {
                if let Some(i) = sel {
                    self.context_activate(i);
                }
            }
            _ => {}
        }
    }

    pub(super) fn context_char(&mut self, c: char) {
        let Some(cm) = &self.context_menu else { return };
        let c = c.to_ascii_lowercase();
        let hit = cm.items.iter().position(|i| {
            i.as_ref()
                .is_some_and(|i| self.enabled(i.cmd) && i.label.to_lowercase().starts_with(c))
        });
        if let Some(i) = hit {
            self.context_activate(i);
        }
    }

    /// The file a context command acts on: the sidebar selection, else the current file.
    pub(super) fn context_path(&self) -> Option<PathBuf> {
        if self.focus == Focus::Sidebar
            && let Some(p) = self.tree.selected_path()
        {
            return Some(p.to_path_buf());
        }
        self.buf().and_then(|b| b.path.clone())
    }
}

/// Context menu items shown only where they apply.
fn only_when_enabled(cmd: Command) -> bool {
    matches!(cmd, Command::OpenProjectInNewWindow)
}

/// No separator first, last, or next to another.
fn tidy_separators<T>(items: &mut Vec<Option<T>>) {
    let mut out: Vec<Option<T>> = Vec::with_capacity(items.len());
    for it in items.drain(..) {
        if it.is_none() && out.last().is_none_or(|l| l.is_none()) {
            continue;
        }
        out.push(it);
    }
    if out.last().is_some_and(|l| l.is_none()) {
        out.pop();
    }
    *items = out;
}
