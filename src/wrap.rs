//! Soft word wrap: splitting a line into visual rows by display columns.
//!
//! Rows break after whitespace when possible (the space stays at the end of the row, like
//! Sublime), otherwise mid-word. Continuation rows are indented like the line itself
//! (Sublime's `indent_subsequent_lines`), capped at half the width.

use unicode_width::UnicodeWidthChar;

/// Display width of `c` at display column `at` (tabs advance to the next tab stop).
pub fn char_width(c: char, at: usize, tab: usize) -> usize {
    if c == '\t' {
        let t = tab.max(1);
        t - at % t
    } else {
        c.width().unwrap_or(1).max(1)
    }
}

/// One visual row of a line. Columns are display columns from the line start; `*_col` are
/// char indices. `indent` is how many columns the row is shifted right on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seg {
    pub start: usize,
    pub end: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub indent: usize,
}

/// Split `line` into rows no wider than `width` columns (including the indent).
pub fn segments(line: &str, tab: usize, width: usize) -> Vec<Seg> {
    let width = width.max(8);
    let lead: usize = {
        let mut w = 0;
        for c in line.chars().take_while(|c| *c == ' ' || *c == '\t') {
            w += char_width(c, w, tab);
        }
        w
    };
    let cont = if lead <= width / 2 { lead } else { 0 };
    let mut out = vec![];
    let (mut s, mut sc) = (0usize, 0usize); // current row start (dcol, char)
    let mut brk: Option<(usize, usize)> = None; // last break opportunity after whitespace
    let mut d = 0usize;
    for (ci, c) in line.chars().enumerate() {
        let w = char_width(c, d, tab);
        let indent = if out.is_empty() { 0 } else { cont };
        if d > s && d + w - s + indent > width {
            let (e, ec) = match brk {
                Some(b) if b.0 > s => b,
                _ => (d, ci),
            };
            out.push(Seg {
                start: s,
                end: e,
                start_col: sc,
                end_col: ec,
                indent,
            });
            s = e;
            sc = ec;
            brk = None;
        }
        d += w;
        if c == ' ' || c == '\t' {
            brk = Some((d, ci + 1));
        }
    }
    out.push(Seg {
        start: s,
        end: d,
        start_col: sc,
        end_col: line.chars().count(),
        indent: if out.is_empty() { 0 } else { cont },
    });
    out
}

/// Display column of char index `col`.
pub fn dcol(line: &str, col: usize, tab: usize) -> usize {
    let mut w = 0;
    for c in line.chars().take(col) {
        w += char_width(c, w, tab);
    }
    w
}

/// The row holding display column `d`. A column on a row boundary belongs to the next
/// row (the caret after a wrapped space shows at the start of the following row).
pub fn seg_of(segs: &[Seg], d: usize) -> usize {
    segs.iter()
        .position(|s| d >= s.start && d < s.end)
        .unwrap_or(segs.len() - 1)
}

/// Char index in row `seg` nearest to display column `target` (rounding to the closer
/// caret position). On rows other than the last, the result stays before the row end so
/// the caret remains on that row.
pub fn col_in_seg(line: &str, tab: usize, seg: &Seg, target: f32, last: bool) -> usize {
    let mut d = 0usize;
    let mut col = 0usize;
    for c in line.chars() {
        if col >= seg.start_col {
            break;
        }
        d += char_width(c, d, tab);
        col += 1;
    }
    for c in line.chars().skip(seg.start_col) {
        if col >= seg.end_col {
            break;
        }
        let w = char_width(c, d, tab);
        if d as f32 + w as f32 / 2.0 > target {
            break;
        }
        d += w;
        col += 1;
    }
    if !last && col >= seg.end_col && seg.end_col > seg.start_col {
        col = seg.end_col - 1;
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_after_spaces_and_indents_continuations() {
        let s = segments("    alpha beta gamma delta", 4, 16);
        // "    alpha beta " | "gamma delta" (continuation indented by 4)
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!((s[0].start, s[0].end, s[0].indent), (0, 15, 0));
        assert_eq!((s[1].start, s[1].end, s[1].indent), (15, 26, 4));
        assert_eq!((s[1].start_col, s[1].end_col), (15, 26));
        // A word longer than the row breaks mid-word.
        let s = segments(&"x".repeat(20), 4, 8);
        assert_eq!(
            s.iter().map(|g| g.end - g.start).collect::<Vec<_>>(),
            [8, 8, 4]
        );
        // Short and empty lines are one row.
        assert_eq!(segments("", 4, 10).len(), 1);
        assert_eq!(segments("short", 4, 10).len(), 1);
    }

    #[test]
    fn columns_map_to_rows() {
        let line = "alpha beta gamma";
        let s = segments(line, 4, 8);
        // "alpha " | "beta " | "gamma"
        assert_eq!(s.len(), 3, "{s:?}");
        assert_eq!(seg_of(&s, 0), 0);
        assert_eq!(seg_of(&s, 6), 1, "boundary belongs to the next row");
        assert_eq!(seg_of(&s, 16), 2, "line end is on the last row");
        // Clicking past the end of a middle row stays on it.
        assert_eq!(col_in_seg(line, 4, &s[1], 99.0, false), 10);
        assert_eq!(col_in_seg(line, 4, &s[2], 99.0, true), 16);
        assert_eq!(col_in_seg(line, 4, &s[2], 12.6, true), 13);
    }
}
