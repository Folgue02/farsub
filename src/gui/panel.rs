//! GUI: the bottom panel: terminals and console.

use super::*;

impl Gui {
    /// The bottom panel: a tab strip (terminals, CONSOLE) and the active view.
    pub(super) fn draw_bottom_panel(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        c: &Palette,
    ) {
        use crate::app::PanelTab;
        let bg = terminal_bg(c);
        p.rect_filled(r, 0.0, bg);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, c.border));
        // Resize by dragging the top edge.
        let edge = Rect::from_min_max(
            r.left_top() - Vec2::new(0.0, 3.0),
            r.right_top() + Vec2::new(0.0, 3.0),
        );
        let er = ui.interact(edge, Id::unique("panel-resize"), Sense::drag());
        if er.hovered() || er.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            p.line_segment([r.left_top(), r.right_top()], Stroke::new(2.0, c.accent));
        }
        if er.dragged() {
            self.app.panel.height = (self.app.panel.height - er.drag_delta().y).clamp(80.0, 1200.0);
        }
        // Tab strip.
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), 26.0));
        let font = FontId::proportional(11.0);
        let mut x = r.left() + 8.0;
        let in_panel = self.app.panel_terminals();
        let mut tabs: Vec<(PanelTab, String)> = self
            .app
            .terminals
            .iter()
            .filter(|t| in_panel.contains(&t.id))
            .enumerate()
            .map(|(i, t)| {
                (
                    PanelTab::Terminal(t.id),
                    format!("TERMINAL {}: {}", i + 1, t.title()),
                )
            })
            .collect();
        tabs.push((PanelTab::Console, "CONSOLE".into()));
        let mut clicked = None;
        let mut close: Option<usize> = None;
        let mut context: Option<(PanelTab, Pos2)> = None;
        for (tab, label) in &tabs {
            let label: String = label.chars().take(36).collect();
            let is_term = matches!(tab, PanelTab::Terminal(_));
            let w = ui
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), c.fg).size().x)
                + if is_term { 38.0 } else { 20.0 };
            let tr = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(w, head.height()));
            x += w;
            let active = *tab == self.app.panel.tab;
            let resp = ui.interact(
                tr,
                Id::unique(("panel-tab", format!("{tab:?}"))),
                Sense::click(),
            );
            // Terminal tabs: × close button (shown on hover / when active), middle-click
            // closes, right-click opens the terminal's context menu.
            let mut on_x = false;
            if let PanelTab::Terminal(id) = tab {
                let xr = Rect::from_center_size(
                    Pos2::new(tr.right() - 12.0, tr.center().y),
                    Vec2::splat(14.0),
                );
                let over_x = resp.hover_pos().is_some_and(|hp| xr.contains(hp));
                on_x = over_x;
                if active || resp.hovered() {
                    if over_x {
                        p.rect_filled(xr, 3.0, mix(bg, c.fg, 0.12));
                    }
                    let k = 3.5;
                    let o = xr.center();
                    let st = Stroke::new(1.2, if over_x { c.fg } else { c.dim });
                    p.line_segment([o + Vec2::new(-k, -k), o + Vec2::new(k, k)], st);
                    p.line_segment([o + Vec2::new(-k, k), o + Vec2::new(k, -k)], st);
                }
                if (resp.clicked() && over_x) || resp.middle_clicked() {
                    close = Some(*id);
                }
            }
            if resp.secondary_clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                context = Some((*tab, pos));
            }
            let col = if active {
                c.fg
            } else if resp.hovered() {
                mix(c.dim, c.fg, 0.5)
            } else {
                c.dim
            };
            p.text(
                Pos2::new(tr.left() + 10.0, tr.center().y),
                Align2::LEFT_CENTER,
                &label,
                font.clone(),
                col,
            );
            if active {
                p.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(tr.left() + 6.0, tr.bottom() - 2.0),
                        Pos2::new(tr.right() - 6.0, tr.bottom()),
                    ),
                    0.0,
                    c.accent,
                );
            }
            // Switch on press; the × closes on release.
            if pressed(ui, &resp) && !on_x {
                clicked = Some(*tab);
            }
        }
        if let Some(id) = close {
            self.app.close_terminal(id);
        }
        if let Some((tab, pos)) = context {
            self.app.panel.tab = tab;
            let kind = if tab == PanelTab::Console {
                crate::menu::ContextKind::Console
            } else {
                crate::menu::ContextKind::Terminal
            };
            self.app.focus = if tab == PanelTab::Console {
                Focus::Console
            } else {
                Focus::Terminal
            };
            self.app.open_context_menu(kind, (pos.x, pos.y));
        }
        // "+" new terminal, "x" kill the current one.
        let btn = |p: &Painter, ui: &mut egui::Ui, at: Pos2, id: &str, label: &str| -> bool {
            let br = Rect::from_center_size(at, Vec2::splat(18.0));
            let resp = ui.interact(br, Id::unique(id), Sense::click());
            if resp.hovered() {
                p.rect_filled(br, 3.0, mix(bg, c.fg, 0.1));
            }
            p.text(
                br.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(14.0),
                c.dim,
            );
            resp.clicked()
        };
        if btn(
            p,
            ui,
            Pos2::new(r.right() - 20.0, head.center().y),
            "panel-close",
            "x",
        ) {
            match self.app.panel.tab {
                PanelTab::Terminal(id) => self.app.close_terminal(id),
                PanelTab::Console => {
                    self.app.panel.visible = false;
                    self.app.focus = Focus::Editor;
                }
            }
        }
        if btn(
            p,
            ui,
            Pos2::new(r.right() - 42.0, head.center().y),
            "panel-new",
            "+",
        ) {
            self.app.new_terminal();
        }
        if let Some(tab) = clicked {
            self.app.panel.tab = tab;
            self.app.focus = if tab == PanelTab::Console {
                Focus::Console
            } else {
                Focus::Terminal
            };
        }
        let body = Rect::from_min_max(Pos2::new(r.left(), head.bottom()), r.max);
        match self.app.panel.tab {
            PanelTab::Terminal(id) => self.draw_terminal(ui, p, body, id, c, None),
            PanelTab::Console => self.draw_console(ui, p, body, c),
        }
    }

    pub(super) fn term_color(col: vt100::Color, default: Color32) -> Color32 {
        match col {
            vt100::Color::Default => default,
            vt100::Color::Idx(i) => {
                let (r, g, b) = crate::terminal::palette(i);
                Color32::from_rgb(r, g, b)
            }
            vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        }
    }

    pub(super) fn draw_terminal(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        id: usize,
        c: &Palette,
        tab: Option<usize>,
    ) {
        // In the panel, or (`tab`) as the shown tab of that editor group.
        let focused = self.app.modal.is_none()
            && match tab {
                Some(g) => self.app.focus == Focus::Editor && self.app.group == g,
                None => self.app.focus == Focus::Terminal,
            };
        let font = FontId::monospace((self.app.font_size - 1.0).max(8.0));
        let (cw, rh) = ui
            .ctx()
            .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let lh = (rh * 1.15).round();
        let inner = r.shrink2(Vec2::new(10.0, 4.0));
        let cols = ((inner.width() / cw).floor() as u16).max(10);
        let rows = ((inner.height() / lh).floor() as u16).max(2);
        let bg = terminal_bg(c);
        let Some(t) = self.app.terminals.iter_mut().find(|t| t.id == id) else {
            return;
        };
        t.resize(rows, cols);
        // Mouse: click focuses, drag selects (screen cells), wheel scrolls history.
        let resp = ui.interact(inner, Id::unique(("term", id)), Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            let dy = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                self.term_scroll_acc += dy / lh;
                let n = self.term_scroll_acc.trunc();
                self.term_scroll_acc -= n;
                if n != 0.0 {
                    t.scroll(n as isize);
                }
            }
        }
        let cell_at = |pt: Pos2| -> (u16, u16) {
            let row = ((pt.y - inner.top()) / lh)
                .floor()
                .clamp(0.0, rows as f32 - 1.0) as u16;
            let col = ((pt.x - inner.left()) / cw)
                .floor()
                .clamp(0.0, cols as f32 - 1.0) as u16;
            (row, col)
        };
        if let Some(pt) = resp.interact_pointer_pos() {
            if resp.drag_started() {
                let at = cell_at(pt);
                t.sel = Some((at, at));
            } else if resp.dragged() {
                if let Some((a, _)) = t.sel {
                    t.sel = Some((a, cell_at(pt)));
                }
            } else if resp.clicked() {
                t.sel = None;
            }
        }
        let focus_click = resp.clicked() || resp.drag_started();
        let term_menu = resp
            .secondary_clicked()
            .then(|| resp.interact_pointer_pos())
            .flatten();
        let sel = t.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let exited = t.is_exited();
        if focus_click {
            match tab {
                Some(g) => self.app.focus_group(g),
                None => self.app.focus = Focus::Terminal,
            }
        }
        let Some(t) = self.app.terminals.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let clip = p.with_clip_rect(r);
        let fg = c.fg;
        t.with_screen(|s| {
            let (crow, ccol) = s.cursor_position();
            for row in 0..rows {
                let y = inner.top() + row as f32 * lh;
                let mut job = LayoutJob::default();
                let mut col = 0u16;
                while col < cols {
                    let Some(cell) = s.cell(row, col) else { break };
                    let (mut f, mut b) = (
                        Self::term_color(cell.fgcolor(), fg),
                        Self::term_color(cell.bgcolor(), bg),
                    );
                    if cell.inverse() {
                        std::mem::swap(&mut f, &mut b);
                    }
                    if cell.dim() {
                        f = mix(b, f, 0.6);
                    }
                    let selected = sel.is_some_and(|((r0, c0), (r1, c1))| {
                        (row, col) >= (r0, c0) && (row, col) <= (r1, c1)
                    });
                    if selected {
                        b = c.sel;
                    }
                    let width = if cell.is_wide() { 2 } else { 1 };
                    if b != bg {
                        clip.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(inner.left() + col as f32 * cw, y),
                                Vec2::new(cw * width as f32, lh),
                            ),
                            0.0,
                            b,
                        );
                    }
                    let text = if cell.has_contents() {
                        cell.contents().to_string()
                    } else {
                        " ".to_string()
                    };
                    job.append(
                        &text,
                        0.0,
                        egui::TextFormat {
                            font_id: font.clone(),
                            color: if cell.bold() {
                                mix(f, Color32::WHITE, 0.15)
                            } else {
                                f
                            },
                            italics: cell.italic(),
                            underline: if cell.underline() {
                                Stroke::new(1.0, f)
                            } else {
                                Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                    col += width;
                }
                let g = p.layout_job(job);
                clip.galley(Pos2::new(inner.left(), y + (lh - rh) / 2.0), g, fg);
            }
            if !s.hide_cursor() && s.scrollback() == 0 && !exited {
                let cr = Rect::from_min_size(
                    Pos2::new(
                        inner.left() + ccol as f32 * cw,
                        inner.top() + crow as f32 * lh,
                    ),
                    Vec2::new(cw, lh),
                );
                if focused {
                    clip.rect_filled(cr, 0.0, mix(c.accent, bg, 0.35));
                } else {
                    clip.rect_stroke(cr, 0.0, Stroke::new(1.0, c.dim), StrokeKind::Inside);
                }
            }
            if s.scrollback() > 0 {
                clip.text(
                    r.right_top() + Vec2::new(-12.0, 6.0),
                    Align2::RIGHT_TOP,
                    format!("scrolled back {} lines", s.scrollback()),
                    FontId::proportional(11.0),
                    c.accent,
                );
            }
        });
        if exited {
            clip.text(
                Pos2::new(inner.left(), inner.bottom() - lh / 2.0),
                Align2::LEFT_CENTER,
                format!(
                    "[process exited with code {} — press any key to close]",
                    self.app
                        .terminals
                        .iter()
                        .find(|t| t.id == id)
                        .and_then(|t| *t.exit_code.lock().unwrap())
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into())
                ),
                font,
                c.dim,
            );
        }
        if let Some(pos) = term_menu {
            match tab {
                Some(g) => self.app.focus_group(g),
                None => self.app.focus = Focus::Terminal,
            }
            self.app
                .open_context_menu(crate::menu::ContextKind::Terminal, (pos.x, pos.y));
        }
    }

    pub(super) fn draw_console(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        use crate::log::Level;
        let focused = self.app.focus == Focus::Console && self.app.modal.is_none();
        let bg = terminal_bg(c);
        let small = FontId::proportional(11.5);
        let filt = &self.app.console.filter;
        let hint = format!(
            "{}{}{}type help",
            if self.app.console.show_debug {
                "debug output on · "
            } else {
                ""
            },
            if self.app.console.wrap {
                ""
            } else {
                "no wrap · "
            },
            if filt.is_empty() {
                String::new()
            } else {
                format!("filter \"{filt}\" · ")
            }
        );
        p.text(
            Pos2::new(r.right() - 64.0, r.top() - 13.0),
            Align2::RIGHT_CENTER,
            hint,
            small.clone(),
            c.dim,
        );
        let input = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - 26.0), r.max);
        let body = Rect::from_min_max(
            Pos2::new(r.left() + 12.0, r.top() + 2.0),
            Pos2::new(r.right() - 12.0, input.top()),
        );
        let entries = self.app.console_entries();
        let mono = FontId::monospace(12.0);
        let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&mono, 'M'));
        let lh = 17.0;
        let wrap_cols = ((body.width() / cw).floor() as usize).max(20);
        // Visual rows: (entry index, start char, end char).
        let lines: Vec<Vec<char>> = entries
            .iter()
            .map(|e| App::console_line(e).chars().collect())
            .collect();
        let mut rows: Vec<(usize, usize, usize)> = vec![];
        for (i, l) in lines.iter().enumerate() {
            if self.app.console.wrap && l.len() > wrap_cols {
                let mut s = 0;
                while s < l.len() {
                    rows.push((i, s, (s + wrap_cols).min(l.len())));
                    s += wrap_cols;
                }
            } else {
                rows.push((i, 0, l.len()));
            }
        }
        let n_vis = ((body.height() / lh).floor() as usize).max(1);
        let max_scroll = rows.len().saturating_sub(n_vis);
        let resp = ui.interact(body, Id::unique("console"), Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            let d = ui
                .ctx()
                .input(|i| (i.smooth_scroll_delta, i.modifiers.shift));
            let (dx, dy) = if d.1 { (d.0.y, 0.0) } else { (d.0.x, d.0.y) };
            if dy != 0.0 {
                let s = self.app.console.scroll as f32 + dy / lh;
                self.app.console.scroll = s.max(0.0).round() as usize;
            }
            if dx != 0.0 && !self.app.console.wrap {
                let longest = lines.iter().map(|l| l.len()).max().unwrap_or(0) as f32 * cw;
                self.app.console.hscroll = (self.app.console.hscroll - dx)
                    .clamp(0.0, (longest - body.width() + 40.0).max(0.0));
            }
        }
        self.app.console.scroll = self.app.console.scroll.min(max_scroll);
        let end = rows.len() - self.app.console.scroll;
        let start = end.saturating_sub(n_vis);
        let hx = if self.app.console.wrap {
            0.0
        } else {
            self.app.console.hscroll
        };
        // Hit testing for selection.
        let hit = |pt: Pos2| -> Option<crate::app::ConsolePos> {
            let vr = start + ((pt.y - body.top()) / lh).floor().max(0.0) as usize;
            let (ei, s, e) = *rows.get(vr.min(end.saturating_sub(1)))?;
            let col = (((pt.x - body.left() + hx) / cw).round().max(0.0) as usize + s).min(e);
            Some((entries[ei].id, col))
        };
        if let Some(pt) = resp.interact_pointer_pos() {
            if resp.double_clicked() {
                if let Some((id, col)) = hit(pt)
                    && let Some(i) = entries.iter().position(|e| e.id == id)
                {
                    let l = &lines[i];
                    let w = |ch: char| ch.is_alphanumeric() || ch == '_';
                    let (mut a, mut b) = (col.min(l.len()), col.min(l.len()));
                    while a > 0 && w(l[a - 1]) {
                        a -= 1;
                    }
                    while b < l.len() && w(l[b]) {
                        b += 1;
                    }
                    self.app.console.sel = Some(((id, a), (id, b)));
                }
            } else if resp.drag_started() {
                self.app.console.sel = hit(pt).map(|h| (h, h));
            } else if resp.dragged() {
                if let (Some((a, _)), Some(h)) = (self.app.console.sel, hit(pt)) {
                    self.app.console.sel = Some((a, h));
                }
            } else if resp.clicked() {
                self.app.console.sel = None;
            }
        }
        if resp.clicked() || resp.drag_started() {
            self.app.focus = Focus::Console;
        }
        if resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.app.focus = Focus::Console;
            self.app
                .open_context_menu(crate::menu::ContextKind::Console, (pos.x, pos.y));
        }
        let sel = self
            .app
            .console
            .sel
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let clip = p.with_clip_rect(body);
        for (row, &(ei, s, e)) in rows[start..end].iter().enumerate() {
            let y = body.top() + row as f32 * lh;
            let ent = &entries[ei];
            let x0 = body.left() - hx;
            // Selection highlight within this visual row.
            if let Some(((ai, ac), (bi, bc))) = sel
                && ent.id >= ai
                && ent.id <= bi
            {
                let from = if ent.id == ai { ac.max(s) } else { s };
                let to = if ent.id == bi { bc.min(e) } else { e };
                if to > from || (ent.id != bi && e == lines[ei].len()) {
                    let extra = if ent.id != bi && e == lines[ei].len() {
                        0.5
                    } else {
                        0.0
                    };
                    clip.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(x0 + (from - s) as f32 * cw, y),
                            Pos2::new(x0 + (to.max(from) - s) as f32 * cw + extra * cw, y + lh),
                        ),
                        0.0,
                        c.sel,
                    );
                }
            }
            // time [source] text, colored per part (only on the first visual row).
            let text: String = lines[ei][s..e].iter().collect();
            let level_col = match ent.level {
                Level::Debug => c.dim,
                Level::Info => c.fg,
                Level::Warn => c.warn,
                Level::Error => c.err,
            };
            let mut job = LayoutJob::default();
            let prefix_len = if s == 0 {
                (ent.time.len().min(8) + ent.source.len() + 4).min(text.chars().count())
            } else {
                0
            };
            let (pre, rest): (String, String) = (
                text.chars().take(prefix_len).collect(),
                text.chars().skip(prefix_len).collect(),
            );
            let fmt = |col| egui::TextFormat {
                font_id: mono.clone(),
                color: col,
                ..Default::default()
            };
            job.append(&pre, 0.0, fmt(mix(c.info, c.dim, 0.4)));
            job.append(&rest, 0.0, fmt(level_col));
            let g = p.layout_job(job);
            clip.galley(Pos2::new(x0, y + 1.0), g, c.fg);
        }
        if entries.is_empty() {
            clip.text(
                Pos2::new(body.left(), body.top() + 12.0),
                Align2::LEFT_CENTER,
                "No log output yet.",
                small.clone(),
                c.dim,
            );
        }
        if self.app.console.scroll > 0 {
            p.text(
                Pos2::new(r.right() - 12.0, input.top() - 4.0),
                Align2::RIGHT_BOTTOM,
                format!("{} newer line(s) below", self.app.console.scroll),
                small,
                c.accent,
            );
        }
        // Input line.
        p.rect_filled(input, 0.0, mix(bg, c.fg, 0.04));
        let pr = p.text(
            Pos2::new(r.left() + 12.0, input.center().y),
            Align2::LEFT_CENTER,
            ">",
            mono.clone(),
            c.accent,
        );
        draw_line_edit(
            p,
            input,
            pr.right() + 8.0,
            &self.app.console.input,
            mono.clone(),
            c,
            focused,
            "",
        );
        let ir = ui.interact(input, Id::unique("console-input"), Sense::click());
        if ir.clicked() {
            self.app.focus = Focus::Console;
        }
    }
}
