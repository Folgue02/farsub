//! GUI: the editor pane (text, gutter, caret, scrollbars, mouse).

use super::*;
use crate::wrap::{Seg, char_width, col_in_seg, seg_of, segments};

/// One visible editor row: a slice of a buffer line (the whole line unless wrapping).
#[derive(Clone, Copy, Debug)]
pub(super) struct Row {
    pub line: usize,
    pub seg: Seg,
    /// First / last visual row of its line.
    pub first: bool,
    pub last: bool,
    pub y: f32,
    /// Screen x of display column 0 for this row (accounts for scroll and wrap indent).
    pub x0: f32,
}

/// The rows of line `ln`: wrapped at `wrap` columns, or the whole line as one row.
fn line_segs(b: &Buffer, ln: usize, wrap: Option<usize>) -> Vec<Seg> {
    match wrap {
        Some(w) => segments(&b.lines[ln], b.tab_size, w),
        None => vec![Seg {
            start: 0,
            end: usize::MAX / 2,
            start_col: 0,
            end_col: b.lines[ln].chars().count(),
            indent: 0,
        }],
    }
}

/// Move a (line, row) scroll position by `n` visual rows, staying inside the document.
fn advance(
    b: &Buffer,
    wrap: Option<usize>,
    (mut ln, mut si): (usize, usize),
    n: isize,
) -> (usize, usize) {
    if wrap.is_none() {
        let l = (ln as isize + n).clamp(0, b.lines.len().saturating_sub(1) as isize);
        return (l as usize, 0);
    }
    for _ in 0..n.unsigned_abs() {
        if n > 0 {
            if si + 1 < line_segs(b, ln, wrap).len() {
                si += 1;
            } else if ln + 1 < b.lines.len() {
                ln += 1;
                si = 0;
            } else {
                break;
            }
        } else if si > 0 {
            si -= 1;
        } else if ln > 0 {
            ln -= 1;
            si = line_segs(b, ln, wrap).len() - 1;
        } else {
            break;
        }
    }
    (ln, si)
}

/// Visual rows from `from` down to `to` (None if `to` is above), counting at most `limit`.
fn rows_between(
    b: &Buffer,
    wrap: Option<usize>,
    from: (usize, usize),
    to: (usize, usize),
    limit: usize,
) -> Option<usize> {
    if to < from {
        return None;
    }
    if wrap.is_none() {
        return Some((to.0 - from.0).min(limit));
    }
    let (mut pos, mut n) = (from, 0);
    while pos < to && n < limit {
        pos = advance(b, wrap, pos, 1);
        n += 1;
    }
    Some(n)
}

/// The rows visible from scroll position `top`, at most `n`.
#[allow(clippy::too_many_arguments)]
fn build_rows(
    b: &Buffer,
    wrap: Option<usize>,
    top: (usize, usize),
    n: usize,
    top_y: f32,
    lh: f32,
    text_left: f32,
    cw: f32,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(n);
    let (mut ln, mut skip) = top;
    let sx = if wrap.is_some() {
        0.0
    } else {
        b.scroll_x as f32
    };
    while rows.len() < n && ln < b.lines.len() {
        let segs = line_segs(b, ln, wrap);
        let count = segs.len();
        for (k, sg) in segs.into_iter().enumerate().skip(skip) {
            if rows.len() >= n {
                break;
            }
            rows.push(Row {
                line: ln,
                seg: sg,
                first: k == 0,
                last: k + 1 == count,
                y: top_y + rows.len() as f32 * lh,
                x0: text_left + (sg.indent as f32 - sg.start as f32 - sx) * cw,
            });
        }
        ln += 1;
        skip = 0;
    }
    rows
}

/// The row showing display column `d` of line `ln`, if visible.
fn row_for(rows: &[Row], ln: usize, d: usize) -> Option<&Row> {
    rows.iter()
        .find(|r| r.line == ln && d >= r.seg.start && (d < r.seg.end || r.last))
}

/// The char whose cell contains display column `target` on `row` (not a caret position).
fn cell_at(line: &str, tab: usize, row: &Row, target: f32) -> Option<(usize, char)> {
    let mut d = 0usize;
    for (col, ch) in line.chars().enumerate() {
        let w = char_width(ch, d, tab);
        if col >= row.seg.start_col
            && col < row.seg.end_col
            && target >= d as f32
            && target < (d + w) as f32
        {
            return Some((col, ch));
        }
        d += w;
    }
    None
}

impl Gui {
    // ================================================================= editor

    /// Returns the cursor's screen position (for popups).
    pub(super) fn draw_editor(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        c: &Palette,
        g: usize,
    ) -> Option<Pos2> {
        let active_group = g == self.app.group;
        let Some(cur) = self.shown_in(g) else {
            if self.app.buffers.is_empty() && g == 0 {
                self.draw_welcome(p, r, c);
            }
            // An empty group still takes focus when clicked.
            let resp = ui.interact(r, Id::unique(("editor-pane", g)), Sense::click());
            if resp.clicked() || resp.secondary_clicked() {
                self.app.focus_group(g);
            }
            return None;
        };
        let (cw, lh, rh) = self.metrics(ui);
        let font = self.mono();
        let visible = ((r.height() / lh).ceil() as usize).max(1);
        if active_group {
            self.app.page = ((r.height() / lh).floor() as usize).max(1);
        }
        let focused = active_group && self.app.focus == Focus::Editor && self.app.modal.is_none();
        // The find panel's matches are outlined while it's open on this file.
        let find_m = if active_group {
            self.app.highlight_matcher()
        } else {
            None
        };
        let find_col = self.app.syn.find_highlight().unwrap_or(c.accent);
        let path = self.app.buffers[cur].path.clone();
        let empty = Vec::new();
        let diags = path
            .as_ref()
            .and_then(|p| self.app.diagnostics.get(p))
            .unwrap_or(&empty);
        let wrap_on = self.app.buffers[cur]
            .word_wrap
            .unwrap_or(self.app.config.editor.word_wrap);
        let b: &mut Buffer = &mut self.app.buffers[cur];
        TAB.with(|t| t.set(b.tab_size));
        let guides = self.app.config.editor.indent_guides;
        let line_highlight = self.app.config.editor.highlight_current_line;
        let guide_col = self.app.syn.guide().unwrap_or(mix(c.bg, c.fg, 0.12));
        let active_guide_col = self.app.syn.active_guide().unwrap_or(mix(c.bg, c.fg, 0.32));

        let digits = b.lines.len().to_string().len().max(3) as f32;
        let gutter_w = (digits + 3.0) * cw;
        let text_left = r.left() + gutter_w;
        if active_group {
            self.text_geom = (cw, text_left, r.top(), lh);
        }
        // Leave room for the vertical scrollbar on the right.
        let text_cols = (((r.width() - gutter_w - SCROLLBAR_W) / cw).floor() as usize).max(1);
        // Word wrap: rows one column narrower than the text area, so the caret at a row's
        // end stays visible.
        let wrap = wrap_on.then(|| text_cols.saturating_sub(1).max(8));
        if wrap.is_some() {
            b.scroll_x = 0;
        }
        b.scroll_y = b.scroll_y.min(b.lines.len().saturating_sub(1));
        b.scroll_row = b
            .scroll_row
            .min(line_segs(b, b.scroll_y, wrap).len().saturating_sub(1));
        let tab = b.tab_size;
        // Something else moved the view (Goto, Page Down…): drop the sub-row offset.
        if self.scroll_mark.get(&b.id) != Some(&(b.scroll_y, b.scroll_row)) {
            b.scroll_px = 0.0;
        }
        b.scroll_px = b.scroll_px.clamp(0.0, lh - 0.01);
        // Rows as they are before this frame's scrolling, for hit-testing the mouse.
        let hit_rows = build_rows(
            b,
            wrap,
            (b.scroll_y, b.scroll_row),
            visible + 1,
            r.top() - b.scroll_px,
            lh,
            text_left,
            cw,
        );
        let row_at = |pt: Pos2| -> Option<&Row> {
            hit_rows
                .iter()
                .find(|row| pt.y >= row.y && pt.y < row.y + lh)
        };

        // Mouse: click, drag, wheel.
        let resp = ui.interact(r, Id::unique(("editor-pane", g)), Sense::click_and_drag());
        let to_pos = |b: &Buffer, pt: Pos2| -> Pos {
            // Above the text: the first row; below it: the last.
            let edge = match hit_rows.first() {
                Some(f) if pt.y < f.y => hit_rows.first(),
                _ => hit_rows.last(),
            };
            let Some(row) = row_at(pt).or(edge) else {
                return Pos::new(0, 0);
            };
            let target = ((pt.x - row.x0) / cw).max(row.seg.start as f32);
            let col = col_in_seg(&b.lines[row.line], tab, &row.seg, target, row.last);
            Pos::new(row.line, col)
        };
        // Pointer: I-beam over text, but the normal arrow over selected text.
        // Uses the character cell under the pointer, not the nearest caret position.
        let over_selection = |b: &Buffer, pt: Pos2| -> bool {
            let Some((a, z)) = b.selection() else {
                return false;
            };
            let Some(row) = row_at(pt) else {
                return false;
            };
            cell_at(&b.lines[row.line], tab, row, (pt.x - row.x0) / cw).is_some_and(|(col, _)| {
                let p = Pos::new(row.line, col);
                a <= p && p < z
            })
        };
        // The character cell under the pointer, for mouse hover (LSP info / diagnostics).
        let char_under = |b: &Buffer, pt: Pos2| -> Option<Pos> {
            if pt.x < text_left {
                return None;
            }
            let row = row_at(pt)?;
            cell_at(&b.lines[row.line], tab, row, (pt.x - row.x0) / cw)
                .filter(|(_, ch)| !ch.is_whitespace())
                .map(|(col, _)| Pos::new(row.line, col))
        };
        let hover_at = if !active_group || resp.dragged() || resp.is_pointer_button_down_on() {
            None
        } else {
            resp.hover_pos().and_then(|hp| char_under(b, hp))
        };
        let mut right_click: Option<(Pos, Pos2, bool)> = None;
        if resp.secondary_clicked()
            && let Some(pt) = resp.interact_pointer_pos()
        {
            right_click = Some((to_pos(b, pt), pt, over_selection(b, pt)));
        }
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        } else if let Some(hp) = resp.hover_pos().filter(|hp| hp.x >= text_left - 2.0) {
            let icon = if over_selection(b, hp) {
                egui::CursorIcon::Default
            } else {
                egui::CursorIcon::Text
            };
            ui.ctx().set_cursor_icon(icon);
        }
        // Line and display column under a point, also past the end of the line (for
        // column selection).
        let to_vcol = |pt: Pos2| -> (usize, usize) {
            match row_at(pt).or(hit_rows.last()) {
                Some(row) => (row.line, ((pt.x - row.x0) / cw).round().max(0.0) as usize),
                None => (0, 0),
            }
        };
        let mut click: Option<(Pos, u32, bool)> = None;
        // Cmd+Click (Ctrl elsewhere) adds a cursor; Alt+drag selects a column.
        let mut add_caret: Option<(Pos, u32)> = None;
        let mut column: Option<((usize, usize), (usize, usize))> = None;
        if !resp.dragged() && !resp.drag_started() {
            self.col_drag = None;
        }
        // A plain press places the caret where the button went down (not where egui
        // notices a drag, a few pixels on) and counts double / triple clicks itself, so
        // a double-click-and-drag selects by word and a triple one by line.
        let (m, pressed, primary_down, latest) = ui.ctx().input(|i| {
            (
                i.modifiers,
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.latest_pos(),
            )
        });
        if !primary_down {
            self.sel_drag = None;
            self.drag_pt = None;
        }
        let mut press: Option<(Pos, u32, bool)> = None;
        // (A press and release in one frame only shows up as a click.)
        if pressed
            && (resp.is_pointer_button_down_on() || resp.clicked())
            && !m.command
            && !m.alt
            && let Some(at) = ui.ctx().input(|i| i.pointer.press_origin())
        {
            let now = Instant::now();
            let n = match self.last_press {
                Some((t, p0, n))
                    if now.duration_since(t) < Duration::from_millis(500)
                        && p0.distance(at) < 6.0 =>
                {
                    n % 3 + 1
                }
                _ => 1,
            };
            self.last_press = Some((now, at, n));
            press = Some((to_pos(b, at), n, m.shift && n == 1));
            self.drag_pt = Some(at);
        }
        // Dragging a plain selection, also outside the pane or the window: above or
        // below the text the view scrolls toward the pointer, faster the further away.
        let mut drag: Option<(Pos, u32, Pos, Pos)> = None;
        if primary_down
            && press.is_none()
            && let Some((unit, s0, e0)) = self.sel_drag
        {
            if latest.is_some() {
                self.drag_pt = latest;
            }
            if let Some(pt) = self.drag_pt {
                let text_bottom = r.top() + self.app.page as f32 * lh;
                let (above, below) = (pt.y < r.top(), pt.y >= text_bottom.min(r.bottom()));
                let pos = if above || below {
                    let now = Instant::now();
                    let tick = Duration::from_millis(40);
                    if now >= self.autoscroll_at {
                        let far = if above { r.top() - pt.y } else { pt.y - text_bottom };
                        let step = ((far / lh).ceil() as isize).clamp(1, 30);
                        let top = (b.scroll_y, b.scroll_row);
                        (b.scroll_y, b.scroll_row) =
                            advance(b, wrap, top, if above { -step } else { step });
                        b.scroll_px = 0.0;
                        self.autoscroll_at = now + tick;
                    }
                    ui.ctx().request_repaint_after(tick);
                    let rows = build_rows(
                        b,
                        wrap,
                        (b.scroll_y, b.scroll_row),
                        visible,
                        r.top(),
                        lh,
                        text_left,
                        cw,
                    );
                    let edge = if above {
                        rows.first()
                    } else {
                        rows.get(self.app.page.saturating_sub(1)).or(rows.last())
                    };
                    match edge {
                        Some(row) => {
                            let target = ((pt.x - row.x0) / cw).max(row.seg.start as f32);
                            let col = col_in_seg(&b.lines[row.line], tab, &row.seg, target, row.last);
                            Pos::new(row.line, col)
                        }
                        None => to_pos(b, pt),
                    }
                } else {
                    to_pos(b, pt)
                };
                drag = Some((pos, unit, s0, e0));
            }
        }
        if let Some(pt) = resp.interact_pointer_pos() {
            let origin = ui.ctx().input(|i| i.pointer.press_origin()).unwrap_or(pt);
            if resp.drag_started() && m.alt {
                self.col_drag = Some(to_vcol(origin));
            }
            if let Some(start) = self.col_drag {
                column = Some((start, to_vcol(pt)));
            } else if m.command && (resp.double_clicked() || resp.triple_clicked()) {
                add_caret = Some((to_pos(b, pt), 2));
            } else if m.command && (resp.drag_started() || resp.clicked()) {
                add_caret = Some((to_pos(b, if resp.drag_started() { origin } else { pt }), 1));
            } else if m.alt && resp.clicked() {
                click = Some((to_pos(b, pt), 1, m.shift));
            }
        }
        let mut hscrolled = false;
        if resp.contains_pointer() {
            let d = ui.ctx().input(|i| i.smooth_scroll_delta);
            if d.y != 0.0 {
                // Smooth: scroll by pixels; whole rows carry into the line position.
                let mut px = b.scroll_px - d.y;
                let mut top = (b.scroll_y, b.scroll_row);
                while px >= lh {
                    let next = advance(b, wrap, top, 1);
                    if next == top {
                        px = 0.0;
                        break;
                    }
                    top = next;
                    px -= lh;
                }
                while px < 0.0 {
                    let prev = advance(b, wrap, top, -1);
                    if prev == top {
                        px = 0.0;
                        break;
                    }
                    top = prev;
                    px += lh;
                }
                // The last row at the top is as far as it goes: no partial row past it
                // (that snapped back once it reached a whole row, a rubber band).
                if px > 0.0 && advance(b, wrap, top, 1) == top {
                    px = 0.0;
                }
                (b.scroll_y, b.scroll_row) = top;
                b.scroll_px = px;
            }
            if d.x != 0.0 && wrap.is_none() {
                // Whole columns; the remainder carries over so slow swipes still move.
                let cols = self.hscroll_rem - d.x / cw;
                let whole = cols.trunc();
                self.hscroll_rem = cols - whole;
                b.scroll_x = (b.scroll_x as isize + whole as isize).max(0) as usize;
                if b.scroll_x == 0 && self.hscroll_rem < 0.0 {
                    self.hscroll_rem = 0.0;
                }
                hscrolled = true;
            }
        }

        // Keep the cursor visible when it moved (not when the user scrolled).
        let key = (b.id, b.cursor, b.version);
        let cur_dcol = display_col(&b.lines[b.cursor.line], b.cursor.col);
        let cur_row = (
            b.cursor.line,
            seg_of(&line_segs(b, b.cursor.line, wrap), cur_dcol),
        );
        if active_group && self.last_cursor != Some(key) {
            let pg = self.app.page;
            let margin = 2.min(pg / 3);
            let top = (b.scroll_y, b.scroll_row);
            let new_top = match rows_between(b, wrap, top, cur_row, pg + 1) {
                None => Some(advance(b, wrap, cur_row, -(margin as isize))),
                Some(d) if d < margin => Some(advance(b, wrap, cur_row, -(margin as isize))),
                Some(d) if d + margin >= pg => Some(advance(
                    b,
                    wrap,
                    cur_row,
                    -(pg.saturating_sub(margin + 1) as isize),
                )),
                _ => None,
            };
            if let Some(t) = new_top {
                (b.scroll_y, b.scroll_row) = t;
                b.scroll_px = 0.0;
            }
            if wrap.is_some() {
                // Wrapped text never scrolls sideways.
            } else if cur_dcol < b.scroll_x {
                b.scroll_x = cur_dcol.saturating_sub(4);
            } else if cur_dcol + 2 >= b.scroll_x + text_cols {
                b.scroll_x = cur_dcol + 4 - text_cols.min(cur_dcol + 4);
            }
            self.last_cursor = Some(key);
        }
        b.scroll_y = b.scroll_y.min(b.lines.len().saturating_sub(1));
        let rows = build_rows(
            b,
            wrap,
            (b.scroll_y, b.scroll_row),
            visible + 1,
            r.top() - b.scroll_px,
            lh,
            text_left,
            cw,
        );

        let deadline = Instant::now() + Duration::from_millis(12);
        let hl_done = b.hl.ensure(
            &self.app.syn,
            &b.lines,
            b.scroll_y,
            b.scroll_y + visible + 1,
            deadline,
        );

        // Every selection (multiple cursors) on the lines in view.
        let (vis0, vis1) = (
            rows.first().map_or(0, |r| r.line),
            rows.last().map_or(0, |r| r.line),
        );
        let all_sels: Vec<crate::editor::Sel> = b
            .sels()
            .into_iter()
            .filter(|s| {
                let (a, z) = s.span();
                z.line >= vis0 && a.line <= vis1
            })
            .collect();
        let sel_spans: Vec<(Pos, Pos)> = all_sels
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.span())
            .collect();
        let caret_lines: Vec<usize> = all_sels
            .iter()
            .filter(|s| s.is_empty())
            .map(|s| s.cursor.line)
            .collect();
        let clip = p.with_clip_rect(r);
        let text_clip = p.with_clip_rect(Rect::from_min_max(
            Pos2::new(text_left - 2.0, r.top()),
            r.max,
        ));
        // Indent guides sit at absolute columns (continuation rows are indented to match).
        let x0 = text_left - b.scroll_x as f32 * cw;
        // Indent guides: one line per indentation level; the guide of the block around the
        // cursor is highlighted (the "active" guide).
        let unit = if b.indent_unit == "\t" {
            b.tab_size
        } else {
            b.indent_unit.len()
        }
        .max(1);
        let active = if guides {
            let w = b.indent_width(b.cursor.line);
            // On a block's opening line, highlight the block below it.
            let below = b
                .lines
                .get(b.cursor.line + 1)
                .map(|_| b.indent_width(b.cursor.line + 1))
                .unwrap_or(0);
            let col = if below > w {
                w
            } else if w >= unit {
                w - unit
            } else {
                usize::MAX
            };
            if col == usize::MAX {
                None
            } else {
                let (mut top, mut bot) = (b.cursor.line, b.cursor.line);
                while top > 0 && b.cursor.line - top < 3000 && b.indent_width(top - 1) > col {
                    top -= 1;
                }
                while bot + 1 < b.lines.len()
                    && bot - b.cursor.line < 3000
                    && b.indent_width(bot + 1) > col
                {
                    bot += 1;
                }
                if b.indent_width(b.cursor.line) <= col {
                    top = b.cursor.line + 1;
                }
                Some((col, top, bot))
            }
        } else {
            None
        };
        for row in &rows {
            let ln = row.line;
            let y = row.y;
            let line = &b.lines[ln];
            // Display columns this row shows; the last row also covers the line break.
            let (rs, re) = (
                row.seg.start,
                if row.last { usize::MAX } else { row.seg.end },
            );
            let xr = |d: usize| row.x0 + d as f32 * cw;
            let lr = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), lh));
            if line_highlight && caret_lines.contains(&ln) {
                clip.rect_filled(lr, 0.0, c.line);
            }
            // Gutter: only on a line's first row.
            let worst = diags
                .iter()
                .filter(|d| d.start.line <= ln && d.end.line >= ln)
                .map(|d| d.severity)
                .min();
            if let Some(s) = worst.filter(|_| row.first) {
                let col = match s {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                clip.circle_filled(Pos2::new(r.left() + cw * 0.8, y + lh / 2.0), 3.0, col);
            }
            if row.first {
                clip.text(
                    Pos2::new(text_left - cw * 1.5, y + lh / 2.0),
                    Align2::RIGHT_CENTER,
                    (ln + 1).to_string(),
                    font.clone(),
                    if ln == b.cursor.line { c.fg } else { c.gutter },
                );
            }
            // Selections.
            for &(a, z) in sel_spans.iter().filter(|(a, z)| a.line <= ln && z.line >= ln) {
                let s = if a.line == ln {
                    display_col(line, a.col)
                } else {
                    0
                };
                let mut e = if z.line == ln {
                    display_col(line, z.col)
                } else {
                    display_col(line, usize::MAX) + 1
                };
                if e == s && z.line != ln {
                    e = s + 1;
                }
                let (s, e) = (s.max(rs), e.min(re));
                if e > s {
                    text_clip.rect_filled(
                        Rect::from_min_max(Pos2::new(xr(s), y), Pos2::new(xr(e), y + lh)),
                        2.0,
                        c.sel,
                    );
                }
            }
            // Search matches.
            if let Some(m) = &find_m {
                for (s, e) in m.find_in(line) {
                    let sc = display_col(line, line[..s].chars().count()).max(rs);
                    let ec = display_col(line, line[..e].chars().count()).min(re);
                    if ec <= sc {
                        continue;
                    }
                    text_clip.rect_stroke(
                        Rect::from_min_max(
                            Pos2::new(xr(sc), y + 1.0),
                            Pos2::new(xr(ec), y + lh - 1.0),
                        ),
                        2.0,
                        Stroke::new(1.0, find_col),
                        StrokeKind::Inside,
                    );
                }
            }
            // Find Results: the matches, filled.
            if let Some(rl) = b.results.as_ref().and_then(|r| r.lines.get(ln)) {
                for &(s, e) in &rl.spans {
                    let sc = display_col(line, s).max(rs);
                    let ec = display_col(line, e).min(re);
                    if ec <= sc {
                        continue;
                    }
                    let r = Rect::from_min_max(
                        Pos2::new(xr(sc), y + 1.0),
                        Pos2::new(xr(ec), y + lh - 1.0),
                    );
                    text_clip.rect_filled(r, 2.0, find_col.gamma_multiply(0.25));
                    text_clip.rect_stroke(r, 2.0, Stroke::new(1.0, find_col), StrokeKind::Inside);
                }
            }
            if guides {
                let w = b.indent_width(ln);
                let mut col = 0;
                while col + unit <= w {
                    let x = x0 + col as f32 * cw + 0.5;
                    let is_active =
                        active.is_some_and(|(ac, t, bt)| ac == col && ln >= t && ln <= bt);
                    text_clip.line_segment(
                        [Pos2::new(x, y), Pos2::new(x, y + lh)],
                        Stroke::new(
                            1.0,
                            if is_active {
                                active_guide_col
                            } else {
                                guide_col
                            },
                        ),
                    );
                    col += unit;
                }
            }
            // Text.
            let result_spans = b
                .results
                .as_ref()
                .and_then(|r| r.lines.get(ln))
                .map(|rl| result_line_spans(line, rl.kind, &self.app.syn, c));
            let spans = match &result_spans {
                Some(sp) => Some(sp),
                None => b.hl.line(ln),
            };
            if wrap.is_some() {
                let n = row.seg.end - row.seg.start;
                if n > 0 {
                    let job = line_job(line, spans, &font, c.fg, row.seg.start, n - 1);
                    text_clip.galley(
                        Pos2::new(xr(row.seg.start), y + (lh - rh) / 2.0),
                        p.layout_job(job),
                        c.fg,
                    );
                }
            } else {
                let job = line_job(line, spans, &font, c.fg, b.scroll_x, text_cols + 2);
                let skipped = skipped_cols(line, b.scroll_x);
                text_clip.galley(
                    Pos2::new(text_left + skipped as f32 * cw, y + (lh - rh) / 2.0),
                    p.layout_job(job),
                    c.fg,
                );
            }
            // Diagnostics underline (squiggle).
            for d in diags
                .iter()
                .filter(|d| d.start.line <= ln && d.end.line >= ln)
            {
                let s = if d.start.line == ln {
                    display_col(line, d.start.col)
                } else {
                    0
                };
                let mut e = if d.end.line == ln {
                    display_col(line, d.end.col)
                } else {
                    display_col(line, usize::MAX)
                };
                if e <= s {
                    e = s + 1;
                }
                let (s, e) = (s.max(rs), e.min(re));
                if e <= s {
                    continue;
                }
                let col = match d.severity {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                squiggle(&text_clip, xr(s), xr(e), y + lh - 2.5, col);
            }
        }
        // Cursor.
        let cursor_px = row_for(&rows, b.cursor.line, cur_dcol)
            .map(|row| Pos2::new(row.x0 + cur_dcol as f32 * cw, row.y))
            .filter(|p| p.y < r.bottom());
        if focused && (cursor_px.is_some() || !all_sels.is_empty()) {
            let ed = &self.app.config.editor;
            let mark = Some((b.id, b.cursor, b.version));
            if self.caret_mark.0 != mark {
                self.caret_mark = (mark, Instant::now());
            }
            // Only animate while the window has focus (no idle repaints in the background).
            let animate = ui.ctx().input(|i| i.focused);
            let style = if animate {
                ed.caret_style
            } else {
                CaretStyle::Solid
            };
            let (alpha, next) = caret_alpha(style, self.caret_mark.1.elapsed());
            if let Some(next) = next {
                ui.ctx().request_repaint_after(next);
            }
            let w = 1.0 + ed.caret_extra_width.clamp(0.0, 20.0);
            let up = ed.caret_extra_top.clamp(-lh / 2.0, lh);
            let down = lh + ed.caret_extra_bottom.clamp(-lh / 2.0, lh);
            if alpha > 0.0 && down > -up {
                let bar = |x: f32, y: f32| {
                    text_clip.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(x - w / 2.0, y - up),
                            Pos2::new(x + w / 2.0, y + down),
                        ),
                        0.0,
                        c.accent.gamma_multiply(alpha),
                    );
                };
                if let Some(cp) = cursor_px {
                    bar(cp.x, cp.y);
                }
                // The other cursors blink along with the primary one.
                for s in all_sels.iter().filter(|s| s.cursor != b.cursor) {
                    let d = display_col(&b.lines[s.cursor.line], s.cursor.col);
                    if let Some(row) = row_for(&rows, s.cursor.line, d) {
                        bar(row.x0 + d as f32 * cw, row.y);
                    }
                }
            }
        }
        if b.scroll_y > 0 || b.scroll_row > 0 || b.scroll_px > 0.0 {
            // Subtle shadow under the tab bar when scrolled.
            for k in 0..6 {
                let a = 40 - k * 6;
                clip.line_segment(
                    [
                        Pos2::new(r.left(), r.top() + k as f32),
                        Pos2::new(r.right(), r.top() + k as f32),
                    ],
                    Stroke::new(1.0, Color32::from_black_alpha(a as u8)),
                );
            }
        }
        // Scrollbars (thin, wider on hover). Vertical: drag the thumb or click
        // the track to jump there; diagnostics are marked along it.
        let max_scroll = b.lines.len().saturating_sub(1);
        let page = visible.saturating_sub(1).max(1);
        if max_scroll > 0 {
            let track = Rect::from_min_max(Pos2::new(r.right() - SCROLLBAR_W, r.top()), r.max);
            let sresp = ui.interact(track, Id::unique(("vscroll", g)), Sense::click_and_drag());
            let wide = sresp.hovered() || sresp.dragged();
            let total = (max_scroll + page) as f32;
            let thumb_h = (track.height() * page as f32 / total).clamp(24.0, track.height());
            let span = (track.height() - thumb_h).max(1.0);
            // Continuous: the sub-row offset moves the thumb too.
            let pos = (b.scroll_y as f32 + b.scroll_px / lh).min(max_scroll as f32);
            let thumb_y = |sy: f32| track.top() + span * sy / max_scroll as f32;
            for d in diags.iter() {
                let y = track.top() + track.height() * d.start.line as f32 / b.lines.len() as f32;
                let col = match d.severity {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                clip.rect_filled(
                    Rect::from_min_size(Pos2::new(track.right() - 4.0, y), Vec2::new(3.0, 2.0)),
                    0.0,
                    col,
                );
            }
            if let Some(pt) = sresp.interact_pointer_pos() {
                let at = thumb_y(pos);
                if sresp.drag_started()
                    || (sresp.is_pointer_button_down_on() && self.vscroll_grab.is_none())
                {
                    // Grabbing the thumb keeps the grip point; the track jumps there first.
                    let on_thumb = pt.y >= at && pt.y <= at + thumb_h;
                    self.vscroll_grab = Some(if on_thumb { pt.y - at } else { thumb_h / 2.0 });
                }
                let grab = self.vscroll_grab.unwrap_or(thumb_h / 2.0);
                let f = ((pt.y - grab - track.top()) / span).clamp(0.0, 1.0);
                let target = f * max_scroll as f32;
                b.scroll_y = target.floor() as usize;
                b.scroll_row = 0;
                b.scroll_px = if b.scroll_y < max_scroll {
                    target.fract() * lh
                } else {
                    0.0
                };
            }
            if !sresp.is_pointer_button_down_on() {
                self.vscroll_grab = None;
            }
            if wide {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            let tw = if wide { 8.0 } else { 5.0 };
            let ty = thumb_y((b.scroll_y as f32 + b.scroll_px / lh).min(max_scroll as f32));
            clip.rect_filled(
                Rect::from_min_size(
                    Pos2::new(track.right() - tw - 2.0, ty),
                    Vec2::new(tw, thumb_h),
                ),
                tw / 2.0,
                mix(c.bg, c.fg, if wide { 0.35 } else { 0.2 }),
            );
        }
        // Horizontal: only when lines are wider than the pane.
        // Widest line, cached: scanning a big file every frame (or keystroke) is slow. After
        // an edit only the visible lines can have grown, so they update the cached width;
        // a full rescan (which also notices shrinking) runs at most once a second.
        let widest = if wrap.is_some() {
            0
        } else {
            let cols = |ls: &[String]| {
                ls.iter()
                    .map(|l| display_col(l, usize::MAX))
                    .max()
                    .unwrap_or(0)
            };
            match self.widest_cache.get(&b.id).copied() {
                Some((v, w, _)) if v == b.version => w,
                Some((_, w, full)) if full.elapsed() < Duration::from_secs(1) => {
                    let end = (b.scroll_y + visible).min(b.lines.len());
                    let w = w.max(cols(&b.lines[b.scroll_y.min(end)..end]));
                    self.widest_cache.insert(b.id, (b.version, w, full));
                    w
                }
                _ => {
                    let w = cols(&b.lines);
                    self.widest_cache
                        .insert(b.id, (b.version, w, Instant::now()));
                    w
                }
            }
        };
        if hscrolled {
            // The wheel stops where the widest line's end is in view.
            let max_x = (widest + 4).saturating_sub(text_cols);
            if b.scroll_x >= max_x {
                b.scroll_x = max_x;
                self.hscroll_rem = self.hscroll_rem.min(0.0);
            }
        }
        if wrap.is_none() && (widest + 2 > text_cols || b.scroll_x > 0) {
            let track = Rect::from_min_max(
                Pos2::new(text_left, r.bottom() - SCROLLBAR_W),
                Pos2::new(r.right() - SCROLLBAR_W, r.bottom()),
            );
            let hresp = ui.interact(track, Id::unique(("hscroll", g)), Sense::click_and_drag());
            let wide = hresp.hovered() || hresp.dragged();
            let max_x = (widest + 4)
                .saturating_sub(text_cols)
                .max(b.scroll_x)
                .max(1);
            let total = (max_x + text_cols) as f32;
            let thumb_w = (track.width() * text_cols as f32 / total).clamp(24.0, track.width());
            let span = (track.width() - thumb_w).max(1.0);
            let thumb_x = |sx: usize| track.left() + span * sx as f32 / max_x as f32;
            if let Some(pt) = hresp.interact_pointer_pos() {
                let at = thumb_x(b.scroll_x);
                if hresp.drag_started()
                    || (hresp.is_pointer_button_down_on() && self.hscroll_grab.is_none())
                {
                    let on_thumb = pt.x >= at && pt.x <= at + thumb_w;
                    self.hscroll_grab = Some(if on_thumb { pt.x - at } else { thumb_w / 2.0 });
                }
                let grab = self.hscroll_grab.unwrap_or(thumb_w / 2.0);
                let f = ((pt.x - grab - track.left()) / span).clamp(0.0, 1.0);
                b.scroll_x = (f * max_x as f32).round() as usize;
            }
            if !hresp.is_pointer_button_down_on() {
                self.hscroll_grab = None;
            }
            if wide {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            let th = if wide { 8.0 } else { 5.0 };
            clip.rect_filled(
                Rect::from_min_size(
                    Pos2::new(thumb_x(b.scroll_x), track.bottom() - th - 2.0),
                    Vec2::new(thumb_w, th),
                ),
                th / 2.0,
                mix(c.bg, c.fg, if wide { 0.35 } else { 0.2 }),
            );
        }
        self.app.hl_pending |= !hl_done;
        self.scroll_mark.insert(b.id, (b.scroll_y, b.scroll_row));
        if active_group {
            self.app.wrap_cols = wrap;
            self.last_rows = rows
                .iter()
                .map(|r| (r.line, r.seg.start, r.seg.end, r.y))
                .collect();
        }
        if !active_group {
            // A click in another group focuses it first, then acts there.
            if press.is_some() || click.is_some() || right_click.is_some() {
                self.app.set_cur(cur);
                self.app.focus = Focus::Editor;
                self.app.completion = None;
                self.app.hover = None;
            } else {
                return None;
            }
        }
        // Where a mouse-hover popup points.
        self.hover_anchor_px = self
            .app
            .hover
            .as_ref()
            .and_then(|h| h.anchor)
            .and_then(|a| {
                let b = self.app.buf()?;
                let d = display_col(b.lines.get(a.line)?, a.col);
                let row = row_for(&rows, a.line, d)?;
                Some(Pos2::new(row.x0 + d as f32 * cw, row.y))
            });
        self.keyboard_spot = cursor_px.map(|p| p + Vec2::new(0.0, lh));
        if let Some((pos, n, ext)) = press {
            self.app.editor_click(pos, n, ext);
            // What the press selected (the word, the line), which a drag extends from.
            self.sel_drag = self.app.buf().map(|b| {
                let (s, e) = b.selection().unwrap_or((b.cursor, b.cursor));
                if n == 1 { (1, pos, pos) } else { (n, s, e) }
            });
            // Double-click on a Find Results line opens its match.
            if n == 2 && self.app.buf().is_some_and(|b| b.results.is_some()) {
                self.app.open_result_line(pos.line);
            }
        } else if let Some((pos, unit, s, e)) = drag {
            self.app.editor_drag(pos, unit, (s, e));
        } else if let Some((pos, n, ext)) = click {
            self.app.editor_click(pos, n, ext);
        } else if let Some((pos, n)) = add_caret {
            self.app.editor_add_caret(pos, n);
        } else if let Some((from, to)) = column {
            self.app.editor_column(from, to);
        }
        if let Some((pos, pt, in_sel)) = right_click {
            // Like VS Code: right-click outside the selection moves the cursor there first.
            if !in_sel {
                self.app.editor_click(pos, 1, false);
            }
            self.app.focus = Focus::Editor;
            self.app
                .open_context_menu(crate::menu::ContextKind::Editor, (pt.x, pt.y));
        }
        let over_popup = self
            .hover_rect
            .zip(ui.ctx().pointer_hover_pos())
            .is_some_and(|(hr, p)| hr.expand(8.0).contains(p));
        if let Some(wait) = self.app.mouse_hover(hover_at, over_popup) {
            ui.ctx().request_repaint_after(wait);
        }
        cursor_px
    }

    /// The empty window's hints: (what, its key). Keys come from the keymap, so they're
    /// right on each platform and after rebinding; unbound ones are left out.
    pub fn welcome_lines(&self) -> Vec<(&'static str, String)> {
        use crate::commands::Command::*;
        [
            ("Goto Anything", GotoFile),
            ("Command Palette", ShowCommands),
            ("Focus Sidebar", FocusSidebar),
            ("Toggle Sidebar", ToggleSidebar),
            ("New File", NewFile),
            ("Open File", OpenFile),
            ("Settings & Languages", OpenSettings),
        ]
        .into_iter()
        .map(|(a, cmd)| (a, self.app.keymap.shortcut(cmd)))
        .filter(|(_, k)| !k.is_empty())
        .collect()
    }

    pub(super) fn draw_welcome(&self, p: &Painter, r: Rect, c: &Palette) {
        let lines = self.welcome_lines();
        let font = FontId::proportional(15.0);
        let top = r.center().y - lines.len() as f32 * 15.0;
        for (i, (a, k)) in lines.iter().enumerate() {
            let y = top + i as f32 * 30.0;
            p.text(
                Pos2::new(r.center().x - 12.0, y),
                Align2::RIGHT_CENTER,
                *a,
                font.clone(),
                c.dim,
            );
            p.text(
                Pos2::new(r.center().x + 12.0, y),
                Align2::LEFT_CENTER,
                k,
                font.clone(),
                c.fg,
            );
        }
    }
}

/// Colors for a Find Results line: file names like headings, line numbers dimmed.
fn result_line_spans(
    line: &str,
    kind: crate::search::ResultKind,
    syn: &crate::highlight::Syntax,
    c: &Palette,
) -> Vec<crate::highlight::Span> {
    use crate::highlight::Style;
    use crate::search::ResultKind;
    let style = |cap: &str, fallback: Color32| {
        syn.capture_style(cap).unwrap_or(Style {
            fg: fallback,
            bold: false,
            italic: false,
            underline: false,
        })
    };
    match kind {
        ResultKind::Header => vec![(0, line.len(), style("markup.heading", c.accent))],
        ResultKind::Match | ResultKind::Context => {
            let num = line
                .find([':', ' '])
                .map(|_| 7.min(line.len()))
                .unwrap_or(0);
            let mut v = vec![(0, num, style("comment", c.dim))];
            if kind == ResultKind::Context {
                v.push((num, line.len(), style("comment", c.dim)));
            }
            v
        }
        ResultKind::Text => vec![(0, line.len(), style("comment", c.dim))],
    }
}
