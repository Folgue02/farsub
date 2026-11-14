//! One-line text inputs (find, prompts, palettes, the console input, the keymap search).
//!
//! Each input is a one-line editor [`Buffer`], so it edits exactly like the editor: the
//! same keymap commands (word movement, select all, delete word, undo…) run through the
//! same code. Only commands that make sense on one line apply; the rest (Enter, Up/Down…)
//! stay with whatever owns the input.

use crate::commands::Command;
use crate::editor::{Buffer, Pos};
use crate::highlight::HlCache;

pub struct LineEdit {
    buf: Buffer,
}

impl Default for LineEdit {
    fn default() -> Self {
        LineEdit::new("")
    }
}

impl std::fmt::Debug for LineEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LineEdit").field(&self.text()).finish()
    }
}

/// Keymap commands an input understands. Copy / Cut / Paste need the clipboard, so the
/// app performs those itself.
pub fn accepts(cmd: Command) -> bool {
    use Command::*;
    matches!(
        cmd,
        MoveLeft
            | MoveRight
            | MoveWordLeft
            | MoveWordRight
            | MoveLineStart
            | MoveLineEnd
            | MoveDocStart
            | MoveDocEnd
            | SelectLeft
            | SelectRight
            | SelectWordLeft
            | SelectWordRight
            | SelectToLineStart
            | SelectToLineEnd
            | SelectToDocStart
            | SelectToDocEnd
            | DeleteLeft
            | DeleteRight
            | DeleteWordLeft
            | DeleteWordRight
            | DeleteToLineStart
            | SelectAll
            | Undo
            | Redo
            | Copy
            | Cut
            | Paste
    )
}

fn one_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

impl LineEdit {
    /// A new input holding `text`, caret at the end.
    pub fn new(text: &str) -> Self {
        let mut buf = Buffer::from_text(0, None, one_line(text), HlCache::new("Plain Text".into()));
        buf.move_end(false);
        LineEdit { buf }
    }

    pub fn text(&self) -> &str {
        &self.buf.lines[0]
    }

    pub fn is_empty(&self) -> bool {
        self.text().is_empty()
    }

    /// Caret position (chars).
    pub fn cursor(&self) -> usize {
        self.buf.cursor.col
    }

    /// Selected range (chars), if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.buf.selection().map(|(a, b)| (a.col, b.col))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.buf.selected_text()
    }

    /// Replace the whole text (undoable), caret at the end.
    pub fn set(&mut self, text: &str) {
        if self.text() == one_line(text) {
            return;
        }
        self.buf.select_all();
        if one_line(text).is_empty() {
            self.buf.delete_selection();
        } else {
            self.buf.insert_text(one_line(text));
        }
        self.buf.move_end(false);
    }

    /// Empty it and forget its undo history (a fresh input).
    pub fn reset(&mut self) {
        *self = LineEdit::new("");
    }

    /// Take the text out, leaving a fresh empty input.
    pub fn take(&mut self) -> String {
        let t = self.text().to_string();
        self.reset();
        t
    }

    /// Type or paste text at the caret (replacing the selection). Only the first line of
    /// pasted text is used.
    pub fn insert(&mut self, s: &str) {
        let t = one_line(s);
        let mut chars = t.chars();
        match (chars.next(), chars.next()) {
            (None, _) => {}
            (Some(c), None) => self.buf.insert_char(c),
            _ => self.buf.insert_text(t),
        }
    }

    pub fn set_cursor(&mut self, col: usize) {
        self.buf.set_cursor(Pos::new(0, col), false);
    }

    /// Cut the selection, returning its text.
    pub fn cut(&mut self) -> Option<String> {
        let t = self.selected_text()?;
        self.buf.delete_selection();
        Some(t)
    }

    /// Run an editing command. Returns whether the text changed, or None if the command
    /// isn't one an input handles (or needs the clipboard).
    pub fn command(&mut self, cmd: Command) -> Option<bool> {
        use Command::*;
        if !accepts(cmd) || matches!(cmd, Copy | Cut | Paste) {
            return None;
        }
        let before = self.buf.version;
        match cmd {
            SelectAll => self.buf.select_all(),
            Undo => {
                self.buf.undo();
            }
            Redo => {
                self.buf.redo();
            }
            c => {
                self.buf.apply_motion(c, 1, None);
            }
        }
        Some(self.buf.version != before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Command::*;

    #[test]
    fn edits_like_the_editor() {
        let mut e = LineEdit::new("hello big world");
        assert_eq!(e.cursor(), 15);
        e.command(MoveWordLeft);
        assert_eq!(e.cursor(), 10);
        e.command(DeleteWordLeft);
        assert_eq!(e.text(), "hello world");
        e.command(SelectAll);
        assert_eq!(e.selected_text().as_deref(), Some("hello world"));
        e.insert("x");
        assert_eq!(e.text(), "x");
        e.command(Undo);
        assert_eq!(e.text(), "hello world");
        e.command(MoveLineStart);
        e.command(SelectWordRight);
        assert_eq!(e.cut().as_deref(), Some("hello"));
        assert_eq!(e.text(), " world");
        e.insert("pasted\nsecond line");
        assert_eq!(e.text(), "pasted world");
        assert_eq!(e.command(MoveUp), None, "not a one-line command");
        assert_eq!(e.take(), "pasted world");
        assert!(e.is_empty());
    }
}
