//! App: mouse hover.

use super::*;

impl App {
    // ===================================================================== mouse hover

    pub fn diagnostics_at(&self, p: Pos) -> Vec<(u8, String)> {
        let Some(path) = self.buf().and_then(|b| b.path.as_ref()) else {
            return vec![];
        };
        let mut v: Vec<(u8, String)> = self
            .diagnostics
            .get(path)
            .map(|ds| {
                ds.iter()
                    .filter(|d| d.start <= p && (p < d.end || (d.start == d.end && p == d.start)))
                    .map(|d| (d.severity, d.message.clone()))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// Called by the GUI every frame with the buffer position under the pointer (or
    /// `None`). After the pointer rests on a word for `HOVER_DELAY`, asks the language
    /// server for hover info at that spot (diagnostics there are shown too).
    pub fn mouse_hover(&mut self, at: Option<Pos>, over_popup: bool) -> Option<Duration> {
        const HOVER_DELAY: Duration = Duration::from_millis(500);
        let blocked = self.modal.is_some()
            || self.completion.is_some()
            || self.context_menu.is_some()
            || self.menu.is_some();
        let Some(at) = at.filter(|_| !blocked) else {
            if !over_popup {
                self.hover_track = None;
                if self.hover.as_ref().is_some_and(|h| h.anchor.is_some()) {
                    self.hover = None;
                }
            }
            return None;
        };
        let Some(b) = self.buf() else { return None };
        let word = b
            .word_bounds(at)
            .unwrap_or((at, Pos::new(at.line, at.col + 1)));
        let id = b.id;
        let same = self
            .hover_track
            .as_ref()
            .is_some_and(|t| t.buf == id && t.word == word);
        if !same {
            if self.hover.as_ref().is_some_and(|h| h.anchor.is_some()) && !over_popup {
                self.hover = None;
            }
            self.hover_track = Some(HoverTrack {
                buf: id,
                word,
                pos: at,
                since: Instant::now(),
                requested: false,
            });
            return Some(HOVER_DELAY);
        }
        let t = self.hover_track.as_mut().unwrap();
        if t.requested {
            return None;
        }
        let left = HOVER_DELAY.saturating_sub(t.since.elapsed());
        if !left.is_zero() {
            return Some(left);
        }
        t.requested = true;
        let pos = t.pos;
        let sent = self.lsp_request_at(
            "textDocument/hover",
            json!({}),
            Pending::Hover {
                buf: id,
                pos,
                mouse: true,
            },
            "hoverProvider",
            pos,
            false,
        );
        if !sent {
            let diags = self.diagnostics_at(pos);
            if !diags.is_empty() {
                self.hover = Some(Hover {
                    lines: vec![],
                    diagnostics: diags,
                    anchor: Some(pos),
                });
            }
        }
        None
    }
}
