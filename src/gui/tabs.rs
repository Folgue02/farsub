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
                    .fonts_mut(|f| f.layout_no_wrap(b.name(), font.clone(), c.fg).size().x);
                (w + 66.0).clamp(100.0, 280.0)
            })
            .collect();
        let before: f32 = widths.iter().take(shown_pos).sum();
        let cur_w = widths.get(shown_pos).copied().unwrap_or(0.0);
        if before + cur_w > r.width() {
            x -= before + cur_w - r.width();
        }
        let clip = p.with_clip_rect(r);
        for (k, &i) in tabs.iter().enumerate() {
            let b = &self.app.buffers[i];
            let w = widths[k];
            let tr = Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w, r.height()));
            x += w;
            let active = Some(i) == shown;
            if active {
                clip.rect_filled(tr, 0.0, c.bg);
                // The accent bar marks the focused group's tab (Sublime dims the others).
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
            clip.text(
                Pos2::new(tr.left() + 32.0, tr.center().y),
                Align2::LEFT_CENTER,
                name,
                font.clone(),
                if active { c.fg } else { c.dim },
            );
            let close = Rect::from_center_size(
                Pos2::new(tr.right() - 16.0, tr.center().y),
                Vec2::splat(16.0),
            );
            let resp = ui.interact(tr, Id::unique(("tab", b.id)), Sense::click());
            let hovered_close = resp.hover_pos().is_some_and(|p| close.contains(p));
            if b.modified && !hovered_close {
                clip.circle_filled(close.center(), 4.0, c.dim);
            } else if active || resp.hovered() {
                let k = 4.0;
                let o = close.center();
                let st = Stroke::new(1.3, if hovered_close { c.fg } else { c.dim });
                clip.line_segment([o + Vec2::new(-k, -k), o + Vec2::new(k, k)], st);
                clip.line_segment([o + Vec2::new(-k, k), o + Vec2::new(k, -k)], st);
            }
            if resp.clicked() {
                click = Some((i, hovered_close));
            } else if resp.middle_clicked() {
                click = Some((i, true));
            }
            if resp.secondary_clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                tab_menu = Some((i, pos));
            }
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
