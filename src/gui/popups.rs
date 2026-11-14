//! GUI: completion and hover popups.

use super::*;

impl Gui {
    pub(super) fn draw_completion(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        editor: Rect,
        at: Pos2,
        c: &Palette,
    ) {
        let Some(comp) = &self.app.completion else {
            return;
        };
        if comp.shown.is_empty() {
            return;
        }
        let (cw, lh, _) = self.metrics(ui);
        let font = self.mono();
        let rows = comp.shown.len().min(10);
        let label_w = comp
            .shown
            .iter()
            .map(|&i| comp.items[i].label.chars().count())
            .max()
            .unwrap_or(10)
            .min(40) as f32
            * cw;
        let w = (label_w + 260.0).min(editor.width() - 20.0);
        let h = rows as f32 * lh + 8.0;
        let start_cols = self
            .app
            .buf()
            .map(|b| b.cursor.col.saturating_sub(comp.start.col))
            .unwrap_or(0) as f32;
        let mut x = at.x - start_cols * cw - 34.0;
        x = x.clamp(editor.left(), editor.right() - w);
        let below = at.y + lh + h <= editor.bottom();
        let y = if below { at.y + lh } else { at.y - h };
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
        p.rect_filled(
            r.translate(Vec2::new(0.0, 3.0)),
            6.0,
            Color32::from_black_alpha(80),
        );
        p.rect_filled(r, 4.0, c.pop);
        p.rect_stroke(r, 4.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let first = comp.sel.unwrap_or(0).saturating_sub(rows - 1);
        let clip = p.with_clip_rect(r);
        let mut clicked = None;
        for (row, (i, &idx)) in comp
            .shown
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let it = &comp.items[idx];
            let rr = Rect::from_min_size(
                Pos2::new(r.left() + 4.0, r.top() + 4.0 + row as f32 * lh),
                Vec2::new(w - 8.0, lh),
            );
            if Some(i) == comp.sel {
                clip.rect_filled(rr, 3.0, c.pop_sel);
            }
            kind_icon(
                &clip,
                Pos2::new(rr.left() + 15.0, rr.center().y),
                it.kind,
                c.pop,
            );
            clip.text(
                Pos2::new(rr.left() + 40.0, rr.center().y),
                Align2::LEFT_CENTER,
                &it.label,
                font.clone(),
                c.fg,
            );
            let dclip = p.with_clip_rect(Rect::from_min_max(
                Pos2::new(rr.left() + 50.0 + label_w, rr.top()),
                rr.max,
            ));
            dclip.text(
                Pos2::new(rr.right() - 8.0, rr.center().y),
                Align2::RIGHT_CENTER,
                &it.detail,
                FontId::proportional(11.5),
                c.dim,
            );
            if ui
                .interact(rr, Id::unique(("comp", row)), Sense::click())
                .clicked()
            {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            if let Some(comp) = &mut self.app.completion {
                comp.sel = Some(i);
            }
            self.app.on_key(Key::new(crate::keys::KeyCode::Enter));
        }
    }

    pub(super) fn draw_hover(&mut self, p: &Painter, editor: Rect, at: Pos2, c: &Palette) {
        let Some(hv) = &self.app.hover else { return };
        let max_w = (editor.width() - 40.0).min(640.0);
        let mut job = LayoutJob::default();
        job.wrap.max_width = max_w;
        let mono = FontId::monospace(12.5);
        // Diagnostics first (colored by severity), then the server's hover text.
        for (sev, msg) in &hv.diagnostics {
            let col = match sev {
                1 => c.err,
                2 => c.warn,
                _ => c.info,
            };
            job.append(
                &format!("{msg}\n"),
                0.0,
                egui::TextFormat {
                    font_id: FontId::proportional(13.0),
                    color: col,
                    ..Default::default()
                },
            );
        }
        if !hv.diagnostics.is_empty() && !hv.lines.is_empty() {
            job.append(
                "\n",
                0.0,
                egui::TextFormat {
                    font_id: FontId::proportional(6.0),
                    color: c.dim,
                    ..Default::default()
                },
            );
        }
        job.append(
            &hv.lines.join("\n"),
            0.0,
            egui::TextFormat {
                font_id: mono,
                color: c.fg,
                ..Default::default()
            },
        );
        let g = p.layout_job(job);
        let size = g.size() + Vec2::new(20.0, 16.0);
        let h = size.y.min(editor.height() * 0.6);
        let x = at.x.clamp(
            editor.left() + 4.0,
            (editor.right() - size.x - 4.0).max(editor.left()),
        );
        let above = at.y - h - 4.0 >= editor.top();
        let y = if above { at.y - h - 4.0 } else { at.y + 26.0 };
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(size.x, h));
        p.rect_filled(
            r.translate(Vec2::new(0.0, 3.0)),
            6.0,
            Color32::from_black_alpha(80),
        );
        p.rect_filled(r, 4.0, c.pop);
        p.rect_stroke(r, 4.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        p.with_clip_rect(r.shrink(4.0))
            .galley(r.min + Vec2::new(10.0, 8.0), g, c.fg);
        self.hover_rect = Some(r);
    }
}
