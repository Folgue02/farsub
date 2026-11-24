//! GUI: tab strips.

use super::*;

impl Gui {
    // ================================================================= tabs

    /// The buffer a group shows this frame.
    pub(super) fn shown_in(&self, g: usize) -> Option<usize> {
        if g == self.app.group {
            self.app.buf().map(|_| self.app.cur)
        } else {
            self.app.group_shown(g)
        }
    }

    pub(super) fn draw_tabs(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        c: &Palette,
        g: usize,
    ) {
        p.rect_filled(r, 0.0, c.chrome);
        // The clock, pomodoro and alarm take the right end of the rightmost strip. Their
        // left edge is slanted: tabs are drawn under it (its background covers them along
        // the diagonal) and the middle of the slant is where tabs end, for the mouse and
        // for scrolling a tab into view.
        let strip = r;
        let timers = if g + 1 == self.app.layout.max(1) {
            self.draw_timers(ui, p, r, c, false)
        } else {
            None
        };
        let (r, clip_right) = match timers {
            Some((x, s)) => (
                Rect::from_min_max(r.min, Pos2::new(x + s / 2.0, r.bottom())),
                x + s,
            ),
            None => (r, r.right()),
        };
        let font = FontId::proportional(UI_SIZE);
        let mut x = r.left();
        let mut click: Option<(usize, bool)> = None;
        let mut tab_menu: Option<(usize, Pos2)> = None;
        let tabs = self.app.group_tabs(g);
        let shown = self.shown_in(g);
        let focused_group = g == self.app.group;
        let shown_pos = tabs.iter().position(|&t| Some(t) == shown).unwrap_or(0);
        // Scroll the strip so the active tab is visible.
        let mono = FontId::monospace(UI_SIZE);
        // Terminal tabs: None while the shell runs, else its exit code.
        let term_exit: Vec<Option<Option<u32>>> = tabs
            .iter()
            .map(|&i| {
                let id = self.app.buffers[i].term?;
                let t = self.app.terminals.iter().find(|t| t.id == id)?;
                Some(
                    t.is_exited()
                        .then(|| t.exit_code.lock().unwrap().unwrap_or(1)),
                )
            })
            .collect();
        let widths: Vec<f32> = tabs
            .iter()
            .zip(&term_exit)
            .map(|(&i, ex)| {
                let b = &self.app.buffers[i];
                if b.pinned {
                    return PINNED_W;
                }
                let (label, f) = match ex {
                    Some(code) => (term_label(b, *code), mono.clone()),
                    None => (tab_label(b), font.clone()),
                };
                let w = ui
                    .ctx()
                    .fonts_mut(|fs| fs.layout_no_wrap(label, f, c.fg).size().x);
                (w + 66.0).clamp(100.0, 280.0)
            })
            .collect();
        let before: f32 = widths.iter().take(shown_pos).sum();
        let cur_w = widths.get(shown_pos).copied().unwrap_or(0.0);
        let total: f32 = widths.iter().sum();
        let (mut off, last) = self.tab_scroll.get(&g).copied().unwrap_or((0.0, None));
        // A newly shown tab is scrolled into view.
        if last != shown {
            if before < off {
                off = before;
            } else if before + cur_w > off + r.width() {
                off = before + cur_w - r.width();
            }
        }
        // The wheel (either direction) scrolls the strip sideways when tabs overflow.
        if ui.rect_contains_pointer(r) {
            let d = ui.ctx().input(|i| i.smooth_scroll_delta);
            off -= if d.x.abs() > d.y.abs() { d.x } else { d.y };
        }
        off = off.clamp(0.0, (total - r.width()).max(0.0));
        self.tab_scroll.insert(g, (off, shown));
        x -= off;
        let clip = p.with_clip_rect(Rect::from_min_max(r.min, Pos2::new(clip_right, r.bottom())));
        let green = {
            let (r, g, b) = crate::terminal::palette(2);
            Color32::from_rgb(r, g, b)
        };
        let tbg = terminal_bg(c);
        for (k, &i) in tabs.iter().enumerate() {
            let b = &self.app.buffers[i];
            let w = widths[k];
            let tr = Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w, r.height()));
            x += w;
            let active = Some(i) == shown;
            let term = term_exit[k];
            let pinned = b.pinned;
            // A terminal tab has the terminal's darker background (a little lighter when
            // not shown) and a green bar instead of the accent.
            let bg = match (term.is_some(), active) {
                (true, true) => tbg,
                (true, false) => mix(c.chrome, tbg, 0.5),
                (false, true) => c.bg,
                (false, false) => c.chrome,
            };
            if bg != c.chrome {
                clip.rect_filled(tr, 0.0, bg);
            }
            if active {
                // The bar marks the focused group's tab (the others are dimmed).
                let bar = match (focused_group, term.is_some()) {
                    (false, _) => c.border,
                    (true, true) => green,
                    (true, false) => c.accent,
                };
                clip.rect_filled(Rect::from_min_size(tr.min, Vec2::new(w, 2.0)), 0.0, bar);
            }
            // The last pinned tab has a heavier edge, between the pinned and the rest.
            let last_pin = pinned
                && tabs
                    .get(k + 1)
                    .is_some_and(|&n| !self.app.buffers[n].pinned);
            clip.line_segment(
                [tr.right_top(), tr.right_bottom()],
                Stroke::new(if last_pin { 2.0 } else { 1.0 }, c.border),
            );
            let name = b.name();
            let icon_at = if pinned {
                tr.center()
            } else {
                Pos2::new(tr.left() + 19.0, tr.center().y)
            };
            let alpha = if active { 1.0 } else { 0.75 };
            match term {
                Some(code) => {
                    let col = if code.is_some() { c.err } else { green };
                    draw_terminal_icon(&clip, icon_at, mix(bg, col, alpha));
                }
                None => draw_icon(
                    &clip,
                    &self.app.config,
                    &mut self.icon_tex,
                    icon_at,
                    &name,
                    false,
                    false,
                    alpha,
                    bg,
                ),
            }
            // The text stops short of the × (a long title is cut off); a terminal's exit
            // code sits right before the ×.
            let mut text_end = tr.right() - 28.0;
            if let Some(Some(code)) = term
                && !pinned
            {
                let ex = clip.text(
                    Pos2::new(text_end, tr.center().y),
                    Align2::RIGHT_CENTER,
                    format!("exit {code}"),
                    mono.clone(),
                    c.err,
                );
                text_end = ex.left() - 8.0;
            }
            let label = if pinned {
                Rect::from_center_size(icon_at, Vec2::ZERO)
            } else {
                let text_clip = clip
                    .with_clip_rect(Rect::from_min_max(tr.min, Pos2::new(text_end, tr.bottom())));
                text_clip.text(
                    Pos2::new(tr.left() + 32.0, tr.center().y),
                    Align2::LEFT_CENTER,
                    &name,
                    if term.is_some() {
                        mono.clone()
                    } else {
                        font.clone()
                    },
                    if active { c.fg } else { c.dim },
                )
            };
            if b.deleted && !pinned {
                // The file is gone from disk.
                clip.text(
                    Pos2::new(label.right() + 4.0, tr.center().y),
                    Align2::LEFT_CENTER,
                    "(deleted)",
                    font.clone(),
                    c.err,
                );
            }
            let close = Rect::from_center_size(
                Pos2::new(tr.right() - 16.0, tr.center().y),
                Vec2::splat(16.0),
            );
            // Only the part of the tab inside the strip takes the mouse.
            let hit = tr.intersect(r);
            let resp = hit
                .is_positive()
                .then(|| ui.interact(hit, Id::unique(("tab", b.id)), Sense::click_and_drag()));
            // A pinned tab has no ×; its name shows on hover.
            let resp = resp.map(|r| if pinned { r.on_hover_text(&name) } else { r });
            let hovered_close = !pinned
                && resp
                    .as_ref()
                    .and_then(|r| r.hover_pos())
                    .is_some_and(|p| close.contains(p));
            let hovered = resp.as_ref().is_some_and(|r| r.hovered());
            if pinned {
                if b.modified {
                    clip.circle_filled(icon_at + Vec2::new(8.0, -7.0), 3.0, c.dim);
                }
            } else if b.modified && !hovered_close {
                clip.circle_filled(close.center(), 4.0, c.dim);
            } else if active || hovered {
                let k = 4.0;
                let o = close.center();
                let st = Stroke::new(1.3, if hovered_close { c.fg } else { c.dim });
                clip.line_segment([o + Vec2::new(-k, -k), o + Vec2::new(k, k)], st);
                clip.line_segment([o + Vec2::new(-k, k), o + Vec2::new(k, -k)], st);
            }
            let Some(resp) = resp else { continue };
            self.tab_hits.push((g, b.id, hit));
            if resp.drag_started() && self.close_press != Some(b.id) {
                self.tab_drag = Some(b.id);
            }
            // The tab switches on press. Its × closes on release, and only if the press
            // was on the × too: switching can scroll the tab and slide its × under the
            // mouse, and that release isn't a click on the ×.
            if pressed(ui, &resp) {
                self.close_press = hovered_close.then_some(b.id);
                if !hovered_close {
                    click = Some((i, false));
                }
            } else if (resp.clicked() && hovered_close && self.close_press == Some(b.id))
                || resp.middle_clicked()
            {
                click = Some((i, true));
            }
            if resp.secondary_clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                tab_menu = Some((i, pos));
            }
        }
        if timers.is_some() {
            self.draw_timers(ui, p, strip, c, true);
        }
        if let Some((i, pos)) = tab_menu {
            self.app.set_cur(i);
            self.app.focus = Focus::Editor;
            self.app
                .open_context_menu(crate::menu::ContextKind::Tab, (pos.x, pos.y));
        }
        if let Some((i, close)) = click {
            self.app.set_cur(i);
            self.app.focus = Focus::Editor;
            if close {
                self.app.run(crate::commands::Command::CloseBuffer);
            }
        }
    }
}

impl Gui {
    /// While a tab is dragged: mark where it would go (between two tabs of a strip, or a
    /// whole other group when over its editor) and move it there on release.
    pub(super) fn drop_tab(&mut self, ui: &egui::Ui, p: &Painter, c: &Palette) {
        let Some(id) = self.tab_drag else { return };
        let (pos, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
        let from = self.app.buf_index_of(id).map(|i| self.app.buffers[i].group);
        let target = pos.and_then(|pos| {
            let g = self
                .group_cols
                .iter()
                .position(|(_, col)| col.contains(pos))?;
            let (strip, col) = self.group_cols[g];
            if !strip.contains(pos) {
                // Over an editor: the end of that group (nothing to do over its own).
                let ed = Rect::from_min_max(Pos2::new(col.left(), strip.bottom()), col.max);
                return (from != Some(g)).then_some((g, None, ed));
            }
            let tabs: Vec<_> = self.tab_hits.iter().filter(|t| t.0 == g).collect();
            let before = tabs.iter().find(|t| pos.x < t.2.center().x);
            let x = match (before, tabs.last()) {
                (Some(t), _) => t.2.left(),
                (None, Some(t)) => t.2.right(),
                (None, None) => strip.left(),
            };
            let bar = Rect::from_min_max(
                Pos2::new(x - 1.0, strip.top()),
                Pos2::new(x + 1.0, strip.bottom()),
            );
            Some((g, before.map(|t| t.1), bar))
        });
        if down {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some((_, before, r)) = target {
                if before.is_none() && r.width() > 2.0 {
                    p.rect_filled(r, 0.0, c.accent.gamma_multiply(0.15));
                } else {
                    p.rect_filled(r, 0.0, c.accent);
                }
            }
            return;
        }
        self.tab_drag = None;
        if let Some((g, before, _)) = target {
            self.app.move_tab(id, g, before);
        }
    }
}

/// Width of a pinned (icon-only) tab.
const PINNED_W: f32 = 40.0;

/// A terminal tab's text (for sizing): its title, plus "exit N" once the shell exited.
fn term_label(b: &Buffer, code: Option<u32>) -> String {
    match code {
        Some(code) => format!("{}  exit {code}", b.name()),
        None => b.name(),
    }
}

/// The terminal tab's icon: a prompt (">_") in a rounded frame.
fn draw_terminal_icon(p: &Painter, at: Pos2, col: Color32) {
    let r = Rect::from_center_size(at, Vec2::new(15.0, 12.0));
    p.rect_stroke(r, 2.0, Stroke::new(1.2, col), StrokeKind::Inside);
    let st = Stroke::new(1.3, col);
    let o = r.left_center() + Vec2::new(3.5, 0.0);
    p.line_segment([o + Vec2::new(0.0, -2.5), o + Vec2::new(2.5, 0.0)], st);
    p.line_segment([o + Vec2::new(2.5, 0.0), o + Vec2::new(0.0, 2.5)], st);
    p.line_segment([o + Vec2::new(4.5, 2.5), o + Vec2::new(8.0, 2.5)], st);
}

/// What a tab shows (for sizing): the name, plus "(deleted)" if the file is gone.
fn tab_label(b: &Buffer) -> String {
    if b.deleted {
        format!("{} (deleted)", b.name())
    } else {
        b.name()
    }
}
