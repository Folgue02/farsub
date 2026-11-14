//! App: completion popup.

use super::*;

impl App {
    // ===================================================================== completion popup

    pub(super) fn comp_move(&mut self, d: isize) {
        if let Some(c) = &mut self.completion {
            if c.shown.is_empty() {
                return;
            }
            let n = c.shown.len() as isize;
            c.sel = Some(match c.sel {
                None if d > 0 => 0,
                None => (n - 1) as usize,
                Some(s) if d.abs() == 1 => (s as isize + d).rem_euclid(n) as usize,
                Some(s) => (s as isize + d).clamp(0, n - 1) as usize,
            });
        }
    }

    pub(super) fn comp_refilter(&mut self) {
        let Some(b) = self.buffers.get(self.cur) else {
            return;
        };
        let Some(c) = &mut self.completion else {
            return;
        };
        let prefix = b.text_range(c.start, b.cursor).to_lowercase();
        let matcher = fuzzy_matcher::skim::SkimMatcherV2::default();
        use fuzzy_matcher::FuzzyMatcher;
        let mut scored: Vec<(i64, usize)> = c
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                if prefix.is_empty() {
                    return Some((0, i));
                }
                matcher.fuzzy_match(&it.filter, &prefix).map(|s| {
                    let bonus = if it.filter.to_lowercase().starts_with(&prefix) {
                        1000
                    } else {
                        0
                    };
                    (s + bonus, i)
                })
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| c.items[a.1].sort.cmp(&c.items[b.1].sort))
        });
        c.shown = scored.into_iter().map(|s| s.1).collect();
        c.sel = None;
        if c.shown.is_empty() {
            self.completion = None;
        }
    }

    pub(super) fn comp_accept(&mut self) {
        let Some(c) = self.completion.take() else {
            return;
        };
        let Some(item) = c
            .sel
            .and_then(|s| c.shown.get(s))
            .and_then(|&i| c.items.get(i))
            .cloned()
        else {
            return;
        };
        let Some(b) = self.buf_mut() else { return };
        let start = item
            .edit_start
            .filter(|s| s.line == b.cursor.line && s.col <= b.cursor.col)
            .unwrap_or(c.start);
        let end = b.cursor;
        // Also swallow the rest of the identifier to the right of the cursor.
        let end = b
            .word_bounds(end)
            .map(|(_, e)| e)
            .filter(|e| *e > end)
            .unwrap_or(end);
        let mut edits = vec![(start, end, item.insert.clone())];
        edits.extend(item.additional.iter().cloned());
        let before_lines = b.lines.len();
        b.apply_edits(edits);
        // Place cursor after the inserted text (adjusting for lines added above by imports).
        let added = b.lines.len() as isize
            - before_lines as isize
            - item.insert.matches('\n').count() as isize;
        let line = (start.line as isize + added.max(0)) as usize;
        let end = crate::editor::end_of(Pos::new(line, start.col), &item.insert);
        b.goto(end);
    }
}
