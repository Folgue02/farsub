//! GUI: the editor pane (text, gutter, caret, scrollbars, mouse).

use super::*;

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
        let search = self.app.search.clone().filter(|s| !s.is_empty() && active_group && matches!(&self.app.modal, Some(Modal::Prompt(p)) if matches!(p.kind, PromptKind::Find { .. })));
        let path = self.app.buffers[cur].path.clone();
        let empty = Vec::new();
        let diags = path
            .as_ref()
            .and_then(|p| self.app.diagnostics.get(p))
            .unwrap_or(&empty);
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

        // Mouse: click, drag, wheel.
        let resp = ui.interact(r, Id::unique(("editor-pane", g)), Sense::click_and_drag());
        let to_pos = |b: &Buffer, pt: Pos2| -> Pos {
            let line = (b.scroll_y as f32 + ((pt.y - r.top()) / lh).floor().max(0.0)) as usize;
            let line = line.min(b.lines.len() - 1);
            let target = ((pt.x - text_left) / cw + b.scroll_x as f32).max(0.0);
            let mut dcol = 0.0;
            let mut col = 0;
            for ch in b.lines[line].chars() {
                let w = char_w(ch, dcol as usize) as f32;
                if dcol + w / 2.0 > target {
                    break;
                }
                dcol += w;
                col += 1;
            }
            Pos::new(line, col)
        };
        // Pointer: I-beam over text, but the normal arrow over selected text (like Sublime).
        // Uses the character cell under the pointer, not the nearest caret position.
        let over_selection = |b: &Buffer, pt: Pos2| -> bool {
            let Some((a, z)) = b.selection() else {
                return false;
            };
            let line = b.scroll_y + ((pt.y - r.top()) / lh).floor().max(0.0) as usize;
            if line >= b.lines.len() {
                return false;
            }
            let target = (pt.x - text_left) / cw + b.scroll_x as f32;
            let mut dcol = 0.0;
            for (col, ch) in b.lines[line].chars().enumerate() {
                let w = char_w(ch, dcol as usize) as f32;
                if target >= dcol && target < dcol + w {
                    let p = Pos::new(line, col);
                    return a <= p && p < z;
                }
                dcol += w;
            }
            false
        };
        // The character cell under the pointer, for mouse hover (LSP info / diagnostics).
        let char_under = |b: &Buffer, pt: Pos2| -> Option<Pos> {
            let line = b.scroll_y + ((pt.y - r.top()) / lh).floor().max(0.0) as usize;
            if line >= b.lines.len() || pt.x < text_left {
                return None;
            }
            let target = (pt.x - text_left) / cw + b.scroll_x as f32;
            let mut dcol = 0.0;
            for (col, ch) in b.lines[line].chars().enumerate() {
                let w = char_w(ch, dcol as usize) as f32;
                if target >= dcol && target < dcol + w {
                    return (!ch.is_whitespace()).then_some(Pos::new(line, col));
                }
                dcol += w;
            }
            None
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
        let mut click: Option<(Pos, u32, bool)> = None;
        let mut drag: Option<Pos> = None;
        if let Some(pt) = resp.interact_pointer_pos() {
            let shift = ui.ctx().input(|i| i.modifiers.shift);
            if resp.triple_clicked() {
                click = Some((to_pos(b, pt), 3, false));
            } else if resp.double_clicked() {
                click = Some((to_pos(b, pt), 2, false));
            } else if resp.drag_started() || resp.clicked() {
                click = Some((to_pos(b, pt), 1, shift));
            } else if resp.dragged() {
                drag = Some(to_pos(b, pt));
            }
        }
        if resp.contains_pointer() {
            let d = ui.ctx().input(|i| i.smooth_scroll_delta);
            if d.y != 0.0 {
                self.scroll_acc -= d.y / lh;
                let n = self.scroll_acc.trunc();
                self.scroll_acc -= n;
                let max = b.lines.len().saturating_sub(1) as f32;
                b.scroll_y = (b.scroll_y as f32 + n).clamp(0.0, max) as usize;
            }
            if d.x != 0.0 {
                b.scroll_x = (b.scroll_x as f32 - d.x / cw).max(0.0) as usize;
            }
        }

        // Keep the cursor visible when it moved (not when the user scrolled).
        let key = (b.id, b.cursor, b.version);
        let cur_dcol = display_col(&b.lines[b.cursor.line], b.cursor.col);
        if active_group && self.last_cursor != Some(key) {
            let margin = 2.min(self.app.page / 3);
            if b.cursor.line < b.scroll_y + margin {
                b.scroll_y = b.cursor.line.saturating_sub(margin);
            } else if b.cursor.line + margin >= b.scroll_y + self.app.page {
                b.scroll_y = (b.cursor.line + margin + 1).saturating_sub(self.app.page);
            }
            if cur_dcol < b.scroll_x {
                b.scroll_x = cur_dcol.saturating_sub(4);
            } else if cur_dcol + 2 >= b.scroll_x + text_cols {
                b.scroll_x = cur_dcol + 4 - text_cols.min(cur_dcol + 4);
            }
            self.last_cursor = Some(key);
        }
        b.scroll_y = b.scroll_y.min(b.lines.len().saturating_sub(1));

        let deadline = Instant::now() + Duration::from_millis(12);
        let hl_done =
            b.hl.ensure(&self.app.syn, &b.lines, b.scroll_y + visible, deadline);

        let sel = b.selection();
        let clip = p.with_clip_rect(r);
        let text_clip = p.with_clip_rect(Rect::from_min_max(
            Pos2::new(text_left - 2.0, r.top()),
            r.max,
        ));
        let x0 = text_left - b.scroll_x as f32 * cw;
        // Indent guides: one line per indentation level; the guide of the block around the
        // cursor is highlighted (Sublime's "active" guide).
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
        for row in 0..visible {
            let ln = b.scroll_y + row;
            if ln >= b.lines.len() {
                break;
            }
            let y = r.top() + row as f32 * lh;
            let line = &b.lines[ln];
            let lr = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), lh));
            if line_highlight && ln == b.cursor.line && sel.is_none() {
                clip.rect_filled(lr, 0.0, c.line);
            }
            // Gutter.
            let worst = diags
                .iter()
                .filter(|d| d.start.line <= ln && d.end.line >= ln)
                .map(|d| d.severity)
                .min();
            if let Some(s) = worst {
                let col = match s {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                clip.circle_filled(Pos2::new(r.left() + cw * 0.8, y + lh / 2.0), 3.0, col);
            }
            clip.text(
                Pos2::new(text_left - cw * 1.5, y + lh / 2.0),
                Align2::RIGHT_CENTER,
                (ln + 1).to_string(),
                font.clone(),
                if ln == b.cursor.line { c.fg } else { c.gutter },
            );
            // Selection.
            if let Some((a, z)) = sel
                && a.line <= ln
                && z.line >= ln
            {
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
                text_clip.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(x0 + s as f32 * cw, y),
                        Pos2::new(x0 + e as f32 * cw, y + lh),
                    ),
                    2.0,
                    c.sel,
                );
            }
            // Search matches.
            if let Some(q) = &search {
                for (s, e) in Buffer::matches_in_line(line, q) {
                    let sc = display_col(line, line[..s].chars().count());
                    let ec = display_col(line, line[..e].chars().count());
                    text_clip.rect_stroke(
                        Rect::from_min_max(
                            Pos2::new(x0 + sc as f32 * cw, y + 1.0),
                            Pos2::new(x0 + ec as f32 * cw, y + lh - 1.0),
                        ),
                        2.0,
                        Stroke::new(1.0, c.accent),
                        StrokeKind::Inside,
                    );
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
            let spans = b.hl.lines.get(ln);
            let job = line_job(line, spans, &font, c.fg, b.scroll_x, text_cols + 2);
            let galley = p.layout_job(job);
            let skipped = skipped_cols(line, b.scroll_x);
            text_clip.galley(
                Pos2::new(text_left + skipped as f32 * cw, y + (lh - rh) / 2.0),
                galley,
                c.fg,
            );
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
                let col = match d.severity {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                squiggle(
                    &text_clip,
                    x0 + s as f32 * cw,
                    x0 + e as f32 * cw,
                    y + lh - 2.5,
                    col,
                );
            }
        }
        // Cursor.
        let cx = x0 + cur_dcol as f32 * cw;
        let cy = r.top() + (b.cursor.line as f32 - b.scroll_y as f32) * lh;
        let cursor_px = (cy >= r.top() && cy < r.bottom()).then_some(Pos2::new(cx, cy));
        if focused && cursor_px.is_some() {
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
            let top = cy - ed.caret_extra_top.clamp(-lh / 2.0, lh);
            let bottom = cy + lh + ed.caret_extra_bottom.clamp(-lh / 2.0, lh);
            if alpha > 0.0 && bottom > top {
                text_clip.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(cx - w / 2.0, top),
                        Pos2::new(cx + w / 2.0, bottom),
                    ),
                    0.0,
                    c.accent.gamma_multiply(alpha),
                );
            }
        }
        if b.scroll_y > 0 {
            // Subtle shadow under the tab bar when scrolled, like Sublime.
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
        // Scrollbars (Sublime-style: thin, wider on hover). Vertical: drag the thumb or click
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
            let thumb_y = |sy: usize| track.top() + span * sy as f32 / max_scroll as f32;
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
                let at = thumb_y(b.scroll_y);
                if sresp.drag_started()
                    || (sresp.is_pointer_button_down_on() && self.vscroll_grab.is_none())
                {
                    // Grabbing the thumb keeps the grip point; the track jumps there first.
                    let on_thumb = pt.y >= at && pt.y <= at + thumb_h;
                    self.vscroll_grab = Some(if on_thumb { pt.y - at } else { thumb_h / 2.0 });
                }
                let grab = self.vscroll_grab.unwrap_or(thumb_h / 2.0);
                let f = ((pt.y - grab - track.top()) / span).clamp(0.0, 1.0);
                b.scroll_y = (f * max_scroll as f32).round() as usize;
            }
            if !sresp.is_pointer_button_down_on() {
                self.vscroll_grab = None;
            }
            if wide {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            let tw = if wide { 8.0 } else { 5.0 };
            let ty = thumb_y(b.scroll_y);
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
        let widest = {
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
        if widest + 2 > text_cols || b.scroll_x > 0 {
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
        if !active_group {
            // A click in another group focuses it first, then acts there.
            if click.is_some() || right_click.is_some() {
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
                let line = b.lines.get(a.line)?;
                let row = a.line.checked_sub(b.scroll_y)?;
                Some(Pos2::new(
                    x0 + display_col(line, a.col) as f32 * cw,
                    r.top() + row as f32 * lh,
                ))
            });
        self.keyboard_spot = cursor_px.map(|p| p + Vec2::new(0.0, lh));
        if let Some((pos, n, ext)) = click {
            self.app.editor_click(pos, n, ext);
        } else if let Some(pos) = drag {
            self.app.editor_drag(pos);
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

    pub(super) fn draw_welcome(&self, p: &Painter, r: Rect, c: &Palette) {
        let lines = [
            ("Goto Anything", "Cmd+P"),
            ("Command Palette", "Cmd+Shift+P"),
            ("Focus Sidebar", "Cmd+0"),
            ("Toggle Sidebar", "Cmd+B"),
            ("New File", "Cmd+N"),
            ("Open Folder", "Cmd+O"),
            ("Settings & Languages", "Cmd+,"),
        ];
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
                *k,
                font.clone(),
                c.fg,
            );
        }
    }
}
