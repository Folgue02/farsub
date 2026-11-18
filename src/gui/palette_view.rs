//! GUI: the input prompt and palettes (Goto Anything, commands, symbols…).

use super::*;

impl Gui {
    // ================================================================= overlays

    pub(super) fn draw_prompt(&self, p: &Painter, r: Rect, c: &Palette) {
        let Some(Modal::Prompt(pr)) = &self.app.modal else {
            return;
        };
        p.rect_filled(r, 0.0, c.chrome);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, c.border));
        let ui_font = FontId::proportional(UI_SIZE);
        let label = p.text(
            Pos2::new(r.left() + 12.0, r.center().y),
            Align2::LEFT_CENTER,
            &pr.title,
            ui_font.clone(),
            c.dim,
        );
        let mut extra = String::new();
        if pr.kind_is_path() {
            extra = "Tab completes · Enter confirms · Esc cancels".into();
        }
        let hint = p.text(
            Pos2::new(r.right() - 12.0, r.center().y),
            Align2::RIGHT_CENTER,
            extra,
            FontId::proportional(11.5),
            c.dim,
        );
        let field = Rect::from_min_max(
            Pos2::new(label.right() + 12.0, r.top() + 6.0),
            Pos2::new(hint.left() - 12.0, r.bottom() - 6.0),
        );
        p.rect_filled(field, 3.0, c.bg);
        p.rect_stroke(field, 3.0, Stroke::new(1.0, c.accent), StrokeKind::Inside);
        draw_line_edit(
            p,
            field.shrink(2.0),
            field.left() + 8.0,
            &pr.input,
            FontId::monospace(UI_SIZE),
            c,
            true,
            "",
        );
    }

    pub(super) fn draw_palette(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        let Some(Modal::Palette(pal)) = &self.app.modal else {
            return;
        };
        let w = (full.width() * 0.6).clamp(360.0, 720.0);
        // Files get Sublime's two-line rows: icon, bold name, path underneath.
        let file_rows = matches!(pal.mode, Mode::Files | Mode::Buffers);
        let row_h = if file_rows { 44.0 } else { 30.0 };
        let max_rows = if file_rows { 9usize } else { 12 };
        let n = pal.shown.len().min(max_rows);
        let line_mode = pal.mode == Mode::Line;
        let rows = if line_mode { 1 } else { n.max(1) };
        let h = 52.0 + rows as f32 * row_h;
        let r = Rect::from_min_size(
            Pos2::new(full.center().x - w / 2.0, full.top() + 40.0),
            Vec2::new(w, h),
        );
        p.rect_filled(
            r.expand(1.0).translate(Vec2::new(0.0, 4.0)),
            8.0,
            Color32::from_black_alpha(90),
        );
        block(ui, r, "palette");
        p.rect_filled(r, 6.0, c.pop);
        p.rect_stroke(r, 6.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let input = Rect::from_min_size(r.min + Vec2::new(10.0, 10.0), Vec2::new(w - 20.0, 32.0));
        p.rect_filled(input, 4.0, c.bg);
        let font = FontId::proportional(14.0);
        draw_line_edit(
            p,
            input.shrink(2.0),
            input.left() + 10.0,
            &pal.query,
            font.clone(),
            c,
            true,
            "",
        );
        let sub = if pal.loading {
            format!("{} · loading…", pal.title)
        } else {
            pal.title.clone()
        };
        let sub_clip = p.with_clip_rect(input);
        sub_clip.text(
            Pos2::new(input.right() - 10.0, input.center().y),
            Align2::RIGHT_CENTER,
            sub,
            FontId::proportional(11.0),
            c.dim,
        );

        let top = input.bottom() + 8.0;
        if line_mode {
            let total = self.app.buf().map(|b| b.lines.len()).unwrap_or(0);
            p.text(
                Pos2::new(r.left() + 20.0, top + 15.0),
                Align2::LEFT_CENTER,
                format!("Type a line number (1–{total}) and press Enter"),
                font,
                c.dim,
            );
            return;
        }
        if pal.shown.is_empty() && !pal.loading {
            p.text(
                Pos2::new(r.left() + 20.0, top + 15.0),
                Align2::LEFT_CENTER,
                "No matches",
                font,
                c.dim,
            );
            return;
        }
        let first = pal.sel.saturating_sub(max_rows - 1);
        let mut clicked = None;
        for (row, (i, (idx, hits))) in pal
            .shown
            .iter()
            .enumerate()
            .skip(first)
            .take(max_rows)
            .enumerate()
        {
            let it = &pal.items[*idx];
            let rr = Rect::from_min_size(
                Pos2::new(r.left() + 6.0, top + row as f32 * row_h),
                Vec2::new(w - 12.0, row_h),
            );
            if i == pal.sel {
                p.rect_filled(rr, 4.0, c.pop_sel);
            }
            if file_rows {
                // Files: the label is the relative path (Goto Anything) or the name
                // (open files, whose path is in `detail`).
                let (name, path, name_off, path_hits) = if pal.mode == Mode::Files {
                    let name = it.label.rsplit('/').next().unwrap_or(&it.label).to_string();
                    let off = it.label.chars().count() - name.chars().count();
                    (name, it.label.clone(), off, true)
                } else {
                    (it.label.clone(), it.detail.clone(), 0, false)
                };
                let bg = if i == pal.sel { c.pop_sel } else { c.pop };
                draw_icon(
                    p,
                    &self.app.config,
                    &mut self.icon_tex,
                    Pos2::new(rr.left() + 20.0, rr.center().y),
                    &name,
                    false,
                    false,
                    1.0,
                    bg,
                );
                let tx = rr.left() + 38.0;
                let clip = p.with_clip_rect(rr.shrink2(Vec2::new(4.0, 0.0)));
                let line = |text: &str, offset: usize, use_hits: bool, size: f32, base: Color32| {
                    let mut job = LayoutJob::default();
                    for (ci, ch) in text.chars().enumerate() {
                        let hit = use_hits && hits.contains(&(ci + offset));
                        job.append(
                            &ch.to_string(),
                            0.0,
                            egui::TextFormat {
                                font_id: FontId::proportional(size),
                                color: if hit { c.accent } else { base },
                                ..Default::default()
                            },
                        );
                    }
                    p.layout_job(job)
                };
                let ng = line(&name, name_off, true, 15.0, c.fg);
                let ny = rr.top() + 5.0;
                // Faux bold: the name drawn twice a hair apart.
                clip.galley(Pos2::new(tx + 0.45, ny), ng.clone(), c.fg);
                clip.galley(Pos2::new(tx, ny), ng, c.fg);
                let pg = line(&path, 0, path_hits, 11.5, c.dim);
                clip.galley(Pos2::new(tx, rr.bottom() - 6.0 - pg.size().y), pg, c.dim);
                if ui
                    .interact(rr, Id::unique(("pal", row)), Sense::click())
                    .clicked()
                {
                    clicked = Some(i);
                }
                continue;
            }
            let mut job = LayoutJob::default();
            for (ci, ch) in it.label.chars().enumerate() {
                let hit = hits.contains(&ci);
                job.append(
                    &ch.to_string(),
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color: if hit { c.accent } else { c.fg },
                        ..Default::default()
                    },
                );
            }
            let g = p.layout_job(job);
            let dw = p
                .ctx()
                .fonts_mut(|f| {
                    f.layout_no_wrap(it.detail.clone(), FontId::proportional(12.0), c.dim)
                        .size()
                        .x
                })
                .min(w * 0.45);
            let label_clip = p.with_clip_rect(Rect::from_min_max(
                rr.min,
                Pos2::new(rr.right() - dw - 20.0, rr.bottom()),
            ));
            label_clip.galley(
                Pos2::new(rr.left() + 10.0, rr.center().y - g.size().y / 2.0),
                g,
                c.fg,
            );
            let detail_clip = p.with_clip_rect(rr);
            detail_clip.text(
                Pos2::new(rr.right() - 10.0, rr.center().y),
                Align2::RIGHT_CENTER,
                &it.detail,
                FontId::proportional(12.0),
                c.dim,
            );
            if ui
                .interact(rr, Id::unique(("pal", row)), Sense::click())
                .clicked()
            {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            if let Some(Modal::Palette(pal)) = &mut self.app.modal {
                pal.sel = i;
            }
            self.app.on_key(Key::new(crate::keys::KeyCode::Enter));
        }
    }
}
