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
        let widths: Vec<f32> = tabs
            .iter()
            .map(|&i| &self.app.buffers[i])
            .map(|b| {
                let w = ui
                    .ctx()
                    .fonts_mut(|f| f.layout_no_wrap(tab_label(b), font.clone(), c.fg).size().x);
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
        let clip = p.with_clip_rect(Rect::from_min_max(
            r.min,
            Pos2::new(clip_right, r.bottom()),
        ));
        for (k, &i) in tabs.iter().enumerate() {
            let b = &self.app.buffers[i];
            let w = widths[k];
            let tr = Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w, r.height()));
            x += w;
            let active = Some(i) == shown;
            if active {
                clip.rect_filled(tr, 0.0, c.bg);
                // The accent bar marks the focused group's tab (the others are dimmed).
                clip.rect_filled(
                    Rect::from_min_size(tr.min, Vec2::new(w, 2.0)),
                    0.0,
                    if focused_group { c.accent } else { c.border },
                );
            }
            clip.line_segment(
                [tr.right_top(), tr.right_bottom()],
                Stroke::new(1.0, c.border),
            );
            let name = b.name();
            draw_icon(
                &clip,
                &self.app.config,
                &mut self.icon_tex,
                Pos2::new(tr.left() + 19.0, tr.center().y),
                &name,
                false,
                false,
                if active { 1.0 } else { 0.75 },
                if active { c.bg } else { c.chrome },
            );
            let label = clip.text(
                Pos2::new(tr.left() + 32.0, tr.center().y),
                Align2::LEFT_CENTER,
                &name,
                font.clone(),
                if active { c.fg } else { c.dim },
            );
            if b.deleted {
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
                .then(|| ui.interact(hit, Id::unique(("tab", b.id)), Sense::click()));
            let hovered_close = resp
                .as_ref()
                .and_then(|r| r.hover_pos())
                .is_some_and(|p| close.contains(p));
            let hovered = resp.as_ref().is_some_and(|r| r.hovered());
            if b.modified && !hovered_close {
                clip.circle_filled(close.center(), 4.0, c.dim);
            } else if active || hovered {
                let k = 4.0;
                let o = close.center();
                let st = Stroke::new(1.3, if hovered_close { c.fg } else { c.dim });
                clip.line_segment([o + Vec2::new(-k, -k), o + Vec2::new(k, k)], st);
                clip.line_segment([o + Vec2::new(-k, k), o + Vec2::new(k, -k)], st);
            }
            let Some(resp) = resp else { continue };
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

/// What a tab shows (for sizing): the name, plus "(deleted)" if the file is gone.
fn tab_label(b: &Buffer) -> String {
    if b.deleted {
        format!("{} (deleted)", b.name())
    } else {
        b.name()
    }
}
