//! App: groups.

use super::*;

impl App {
    // ===================================================================== groups

    /// Make buffer `i` current; focus follows it to its group.
    pub fn set_cur(&mut self, i: usize) {
        self.cur = i;
        if let Some(b) = self.buffers.get(i) {
            self.group = b.group;
            self.group_active[b.group] = Some(b.id);
        }
    }

    /// Add a buffer to the focused group; returns its index.
    pub(super) fn push_buffer(&mut self, mut b: Buffer) -> usize {
        b.group = self.group;
        self.buffers.push(b);
        self.buffers.len() - 1
    }

    /// Buffer indices in a group, in tab order.
    pub fn group_tabs(&self, g: usize) -> Vec<usize> {
        (0..self.buffers.len())
            .filter(|&i| self.buffers[i].group == g)
            .collect()
    }

    /// The buffer a group shows: its last active one, else its first tab.
    pub fn group_shown(&self, g: usize) -> Option<usize> {
        self.group_active[g]
            .and_then(|id| self.buf_index(id))
            .filter(|&i| self.buffers[i].group == g)
            .or_else(|| self.group_tabs(g).first().copied())
    }

    pub fn focus_group(&mut self, g: usize) {
        if g >= self.layout {
            return;
        }
        self.group = g;
        if let Some(i) = self.group_shown(g) {
            self.set_cur(i);
        }
        self.focus = Focus::Editor;
        self.completion = None;
        self.hover = None;
    }

    /// Switch to 1–4 columns. Tabs of removed groups move into the last remaining one.
    pub fn set_layout(&mut self, n: usize) {
        let n = n.clamp(1, MAX_GROUPS);
        if n < self.layout {
            let last = n - 1;
            for b in &mut self.buffers {
                if b.group > last {
                    b.group = last;
                }
            }
            for g in n..MAX_GROUPS {
                self.group_active[g] = None;
            }
        }
        self.layout = n;
        self.group_split = vec![1.0 / n as f32; n];
        let g = self.group.min(n - 1);
        let keep = self.buf().map(|b| b.id);
        self.group = g;
        match keep.and_then(|id| self.buf_index(id)) {
            Some(i) => self.set_cur(i),
            None => {
                if let Some(i) = self.group_shown(g) {
                    self.set_cur(i);
                }
            }
        }
    }

    /// Move the current file to group `g` (adding columns if needed); focus follows it.
    pub fn move_to_group(&mut self, g: usize) {
        if g >= MAX_GROUPS {
            return;
        }
        let Some(id) = self.buf().map(|b| b.id) else {
            return;
        };
        if g >= self.layout {
            self.set_layout(g + 1);
        }
        let i = self.buf_index(id).unwrap();
        let from = self.buffers[i].group;
        if from == g {
            return;
        }
        // The group it leaves shows its neighbour (the tab to the left, else the first).
        let rest: Vec<usize> = self
            .group_tabs(from)
            .into_iter()
            .filter(|&t| t != i)
            .collect();
        let neighbour = rest
            .iter()
            .rev()
            .find(|&&t| t < i)
            .or(rest.first())
            .copied();
        self.group_active[from] = neighbour.map(|t| self.buffers[t].id);
        self.buffers[i].group = g;
        self.set_cur(i);
        self.focus = Focus::Editor;
    }

    /// After a buffer in group `g` closed at index `i`, show a neighbouring tab of the
    /// same group (the one to its left), keeping focus in that group.
    pub(super) fn reselect_after_close(&mut self, g: usize, i: usize) {
        let tabs = self.group_tabs(g);
        let pick = tabs
            .iter()
            .rev()
            .find(|&&t| t < i)
            .or_else(|| tabs.first())
            .copied();
        match pick {
            Some(t) => self.set_cur(t),
            None => {
                self.group_active[g] = None;
                self.group = g;
                // Keep `cur` valid; buf() is None for an empty focused group.
                self.cur = self.cur.min(self.buffers.len().saturating_sub(1));
            }
        }
    }

    /// Cycle tabs within the focused group.
    pub(super) fn cycle_tab(&mut self, d: isize) {
        let tabs = self.group_tabs(self.group);
        if tabs.is_empty() {
            return;
        }
        let at = tabs.iter().position(|&t| t == self.cur).unwrap_or(0) as isize;
        let n = tabs.len() as isize;
        self.set_cur(tabs[(at + d).rem_euclid(n) as usize]);
    }

    pub(super) fn buf_index(&self, id: usize) -> Option<usize> {
        self.buffers.iter().position(|b| b.id == id)
    }

    pub fn title(&self) -> String {
        let root = self.project.name();
        match self.buf() {
            Some(b) => format!(
                "{}{}{} — {root}",
                b.name(),
                if b.deleted { " (deleted)" } else { "" },
                if b.modified { " •" } else { "" }
            ),
            None => format!("{root} — farsub"),
        }
    }

    pub fn any_modified(&self) -> bool {
        self.buffers.iter().any(|b| b.modified)
    }

    #[cfg(test)]
    pub fn refresh_tree_for_test(&mut self) {
        self.refresh_tree();
    }

    pub(super) fn refresh_tree(&mut self) {
        self.tree.refresh();
        self.files.clear();
    }

    /// A path for display: relative to its project folder (prefixed with the folder's name
    /// when the project has several).
    pub fn rel(&self, p: &Path) -> String {
        match self.project.folder_of(p) {
            Some(f) => {
                let rest = p.strip_prefix(&f.path).unwrap_or(p).display().to_string();
                if self.project.folders.len() > 1 {
                    if rest.is_empty() {
                        f.name.clone()
                    } else {
                        format!("{}/{rest}", f.name)
                    }
                } else {
                    rest
                }
            }
            None => p.display().to_string(),
        }
    }

    /// Where relative paths typed in prompts start from.
    pub fn base_dir(&self) -> PathBuf {
        self.project
            .primary()
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default()
    }

    pub(super) fn resolve(&self, base: &Path, s: &str) -> PathBuf {
        let s = s.trim();
        if let Some(p) = crate::paths::expand_tilde(s) {
            return p;
        }
        let p = PathBuf::from(s);
        if p.is_absolute() { p } else { base.join(p) }
    }
}
