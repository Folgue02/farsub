//! GUI: the in-window menu bar, dropdowns, keymap viewer and context menus.

use super::*;

impl Gui {
    pub(super) fn draw_menu_bar(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        p.rect_filled(r, 0.0, c.chrome);
        p.line_segment(
            [r.left_bottom(), r.right_bottom()],
            Stroke::new(1.0, c.border),
        );
        let font = FontId::proportional(UI_SIZE);
        let open = self.app.menu.map(|m| m.menu);
        let mut x = r.left() + 6.0;
        self.menu_titles.clear();
        let mut action: Option<Option<usize>> = None;
        let titles: Vec<String> = self.app.menus.iter().map(|m| m.title.clone()).collect();
        for (i, title) in titles.iter().enumerate() {
            let w = ui.ctx().fonts_mut(|f| {
                f.layout_no_wrap(title.to_string(), font.clone(), c.fg)
                    .size()
                    .x
            }) + 20.0;
            let tr = Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w, r.height()));
            x += w;
            self.menu_titles.push(tr);
            let resp = ui.interact(tr, Id::unique(("menu", i)), Sense::click());
            if open == Some(i) {
                p.rect_filled(tr.shrink2(Vec2::new(0.0, 2.0)), 3.0, c.pop_sel);
            } else if resp.hovered() {
                p.rect_filled(
                    tr.shrink2(Vec2::new(0.0, 2.0)),
                    3.0,
                    mix(c.chrome, c.fg, 0.08),
                );
            }
            p.text(
                tr.center(),
                Align2::CENTER_CENTER,
                title.as_str(),
                font.clone(),
                if open == Some(i) {
                    c.fg
                } else {
                    mix(c.fg, c.chrome, 0.15)
                },
            );
            // Menus open on press, like native menu bars.
            if pressed(ui, &resp) {
                action = Some(if open == Some(i) { None } else { Some(i) });
            } else if open.is_some() && open != Some(i) && resp.hovered() {
                action = Some(Some(i));
            }
        }
        p.text(
            Pos2::new(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            "F10 menu   ·   Cmd+Shift+/ shortcuts".replace(
                "Cmd",
                if cfg!(target_os = "macos") {
                    "Cmd"
                } else {
                    "Ctrl"
                },
            ),
            FontId::proportional(11.0),
            c.dim,
        );
        if let Some(a) = action {
            self.app.set_menu(a);
        }
    }

    pub(super) fn draw_menu_dropdown(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        full: Rect,
        c: &Palette,
    ) {
        let Some(m) = self.app.menu else { return };
        let Some(title) = self.menu_titles.get(m.menu).copied() else {
            return;
        };
        let items: Vec<Option<(crate::commands::Command, String, String)>> = self
            .app
            .menus
            .get(m.menu)
            .map(|mm| {
                mm.items
                    .iter()
                    .map(|i| {
                        i.as_ref()
                            .map(|i| (i.cmd, i.label.clone(), self.app.keymap.shortcut(i.cmd)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let font = FontId::proportional(UI_SIZE);
        let small = FontId::proportional(12.0);
        let mut label_w: f32 = 120.0;
        let mut key_w: f32 = 0.0;
        for it in items.iter().flatten() {
            let lw = ui
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(it.1.clone(), font.clone(), c.fg).size().x);
            let kw = ui.ctx().fonts_mut(|f| {
                f.layout_no_wrap(it.2.clone(), small.clone(), c.dim)
                    .size()
                    .x
            });
            label_w = label_w.max(lw);
            key_w = key_w.max(kw);
        }
        let w = label_w + key_w + 64.0;
        let h: f32 = items
            .iter()
            .map(|i| if i.is_some() { MENU_ROW } else { 9.0 })
            .sum::<f32>()
            + 8.0;
        let x = title.left().min(full.right() - w - 4.0);
        let r = Rect::from_min_size(Pos2::new(x, title.bottom()), Vec2::new(w, h));
        p.rect_filled(
            r.translate(Vec2::new(0.0, 4.0)),
            6.0,
            Color32::from_black_alpha(90),
        );
        p.rect_filled(r, 5.0, c.pop);
        p.rect_stroke(r, 5.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let mut y = r.top() + 4.0;
        let mut hovered = None;
        let mut clicked = None;
        for (i, it) in items.iter().enumerate() {
            let Some((cmd, label, keys)) = it else {
                p.line_segment(
                    [
                        Pos2::new(r.left() + 10.0, y + 4.5),
                        Pos2::new(r.right() - 10.0, y + 4.5),
                    ],
                    Stroke::new(1.0, c.border),
                );
                y += 9.0;
                continue;
            };
            let rr =
                Rect::from_min_size(Pos2::new(r.left() + 4.0, y), Vec2::new(w - 8.0, MENU_ROW));
            y += MENU_ROW;
            let enabled = self.app.enabled(*cmd);
            let (label, keys) = (label.clone(), keys.clone());
            if i == m.item && enabled {
                p.rect_filled(rr, 4.0, c.pop_sel);
            }
            let col = if enabled { c.fg } else { mix(c.fg, c.pop, 0.6) };
            p.text(
                Pos2::new(rr.left() + 12.0, rr.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                col,
            );
            p.text(
                Pos2::new(rr.right() - 12.0, rr.center().y),
                Align2::RIGHT_CENTER,
                keys,
                small.clone(),
                if enabled {
                    c.dim
                } else {
                    mix(c.dim, c.pop, 0.5)
                },
            );
            let resp = ui.interact(rr, Id::unique(("menuitem", m.menu, i)), Sense::click());
            if resp.hovered() && enabled {
                hovered = Some(i);
            }
            if resp.clicked() && enabled {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.app.menu_activate(m.menu, i);
            return;
        }
        if let Some(i) = hovered
            && let Some(ms) = &mut self.app.menu
        {
            ms.item = i;
        }
        // Clicking anywhere else closes the menu.
        let (pressed, pos) = ui
            .ctx()
            .input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
        if pressed
            && let Some(pos) = pos
            && !r.contains(pos)
            && !self.menu_titles.iter().any(|t| t.contains(pos))
        {
            self.app.menu = None;
        }
    }

    pub(super) fn draw_keymap(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        let Some(Modal::Keymap(v)) = &self.app.modal else {
            return;
        };
        let rows = crate::app::keymap_rows(&self.app.keymap, v.query.text());
        p.rect_filled(full, 0.0, Color32::from_black_alpha(90));
        let r = Rect::from_center_size(
            full.center(),
            Vec2::new(
                (full.width() * 0.75).clamp(480.0, 920.0),
                full.height() * 0.84,
            ),
        );
        p.rect_filled(
            r.translate(Vec2::new(0.0, 5.0)),
            8.0,
            Color32::from_black_alpha(90),
        );
        p.rect_filled(r, 6.0, c.pop);
        p.rect_stroke(r, 6.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let font = FontId::proportional(UI_SIZE);
        p.text(
            Pos2::new(r.left() + 18.0, r.top() + 22.0),
            Align2::LEFT_CENTER,
            "Keyboard Shortcuts",
            FontId::proportional(16.0),
            c.fg,
        );
        p.text(
            Pos2::new(r.right() - 18.0, r.top() + 22.0),
            Align2::RIGHT_CENTER,
            "type to filter · Up/Down select · Enter runs the command · Esc closes",
            FontId::proportional(11.5),
            c.dim,
        );
        let input = Rect::from_min_size(
            Pos2::new(r.left() + 14.0, r.top() + 42.0),
            Vec2::new(r.width() - 28.0, 30.0),
        );
        p.rect_filled(input, 4.0, c.bg);
        draw_line_edit(
            p,
            input.shrink(2.0),
            input.left() + 10.0,
            &v.query,
            FontId::proportional(14.0),
            c,
            true,
            "Search shortcuts or actions…",
        );

        // Flatten into lines: section headers + rows.
        let mut lines: Vec<(Option<&str>, Option<usize>)> = vec![];
        let mut last = "";
        for (i, row) in rows.iter().enumerate() {
            if row.section != last {
                lines.push((Some(row.section.as_str()), None));
                last = row.section.as_str();
            }
            lines.push((None, Some(i)));
        }
        let list = Rect::from_min_max(
            Pos2::new(r.left() + 8.0, input.bottom() + 8.0),
            Pos2::new(r.right() - 8.0, r.bottom() - 8.0),
        );
        let lh = 26.0;
        let n_vis = ((list.height() / lh).floor() as usize).max(1);
        let sel = v.sel.min(rows.len().saturating_sub(1));
        let sel_line = lines.iter().position(|l| l.1 == Some(sel)).unwrap_or(0);
        let mut scroll = v.scroll;
        if sel_line < scroll + 1 {
            scroll = sel_line.saturating_sub(1);
        } else if sel_line >= scroll + n_vis {
            scroll = sel_line + 1 - n_vis;
        }
        let resp = ui.interact(list, Id::unique("keymap-list"), Sense::click());
        let wheel = if resp.hovered() {
            ui.ctx().input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        let mut new_sel = None;
        if wheel != 0.0 {
            let max = lines.len().saturating_sub(n_vis);
            scroll = ((scroll as f32 - wheel / lh).round().max(0.0) as usize).min(max);
        }
        let key_col = list.left() + list.width() * 0.62;
        let clip = p.with_clip_rect(list);
        for (row, (header, idx)) in lines.iter().enumerate().skip(scroll).take(n_vis) {
            let y = list.top() + (row - scroll) as f32 * lh;
            let rr = Rect::from_min_size(Pos2::new(list.left(), y), Vec2::new(list.width(), lh));
            if let Some(h) = header {
                clip.text(
                    Pos2::new(rr.left() + 10.0, rr.center().y + 3.0),
                    Align2::LEFT_CENTER,
                    h.to_uppercase(),
                    FontId::proportional(11.0),
                    c.accent,
                );
                continue;
            }
            let i = idx.unwrap();
            let it = &rows[i];
            if i == sel {
                clip.rect_filled(rr, 4.0, c.pop_sel);
            }
            clip.text(
                Pos2::new(rr.left() + 18.0, rr.center().y),
                Align2::LEFT_CENTER,
                &it.desc,
                font.clone(),
                c.fg,
            );
            // Keys drawn as little caps.
            let mut kx = key_col;
            for (j, k) in it.keys.split(", ").filter(|k| !k.is_empty()).enumerate() {
                if j > 0 {
                    kx += 8.0;
                }
                let g = p
                    .ctx()
                    .fonts_mut(|f| f.layout_no_wrap(k.to_string(), FontId::monospace(12.0), c.fg));
                let kr = Rect::from_min_size(
                    Pos2::new(kx, rr.center().y - 10.0),
                    Vec2::new(g.size().x + 12.0, 20.0),
                );
                if kr.right() > rr.right() - 4.0 {
                    break;
                }
                clip.rect_filled(kr, 4.0, c.bg);
                clip.rect_stroke(kr, 4.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
                clip.galley(
                    Pos2::new(kr.left() + 6.0, kr.center().y - g.size().y / 2.0),
                    g,
                    c.fg,
                );
                kx = kr.right();
            }
            if resp.clicked()
                && let Some(pos) = resp.interact_pointer_pos()
                && rr.contains(pos)
            {
                new_sel = Some(i);
            }
        }
        if rows.is_empty() {
            clip.text(
                Pos2::new(list.left() + 18.0, list.top() + 16.0),
                Align2::LEFT_CENTER,
                "No matching shortcuts",
                font,
                c.dim,
            );
        }
        if let Some(Modal::Keymap(v)) = &mut self.app.modal {
            v.scroll = scroll;
            v.sel = new_sel.unwrap_or(sel);
        }
    }

    /// A right-click menu (mouse or Shift+F10).
    pub(super) fn draw_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        full: Rect,
        c: &Palette,
    ) {
        let Some(cm) = self.app.context_menu.clone() else {
            return;
        };
        let font = FontId::proportional(UI_SIZE);
        let small = FontId::proportional(12.0);
        let rows: Vec<Option<(crate::commands::Command, String, String, bool)>> = cm
            .items
            .iter()
            .map(|i| {
                i.as_ref().map(|i| {
                    (
                        i.cmd,
                        i.label.clone(),
                        self.app.keymap.shortcut(i.cmd),
                        self.app.enabled(i.cmd),
                    )
                })
            })
            .collect();
        let mut lw: f32 = 120.0;
        let mut kw: f32 = 0.0;
        for (_, l, k, _) in rows.iter().flatten() {
            lw = lw.max(
                ui.ctx()
                    .fonts_mut(|f| f.layout_no_wrap(l.clone(), font.clone(), c.fg).size().x),
            );
            kw = kw.max(
                ui.ctx()
                    .fonts_mut(|f| f.layout_no_wrap(k.clone(), small.clone(), c.dim).size().x),
            );
        }
        let w = lw + kw + 56.0;
        let h: f32 = rows
            .iter()
            .map(|r| if r.is_some() { MENU_ROW } else { 9.0 })
            .sum::<f32>()
            + 8.0;
        let at = match cm.pos {
            Some((x, y)) => Pos2::new(x, y),
            None => match cm.kind {
                crate::menu::ContextKind::Sidebar => self.sidebar_sel_px.unwrap_or(full.center()),
                crate::menu::ContextKind::Editor | crate::menu::ContextKind::Tab => {
                    self.keyboard_spot.unwrap_or(full.center())
                }
                _ => full.center() - Vec2::new(w / 2.0, 0.0),
            },
        };
        let x = at.x.min(full.right() - w - 4.0).max(full.left() + 4.0);
        let y = if at.y + h > full.bottom() - 4.0 {
            (at.y - h).max(full.top() + 4.0)
        } else {
            at.y
        };
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h));
        p.rect_filled(
            r.translate(Vec2::new(0.0, 4.0)),
            6.0,
            Color32::from_black_alpha(90),
        );
        p.rect_filled(r, 5.0, c.pop);
        p.rect_stroke(r, 5.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let mut yy = r.top() + 4.0;
        let mut hovered = None;
        let mut clicked = None;
        for (i, row) in rows.iter().enumerate() {
            let Some((_, label, keys, enabled)) = row else {
                p.line_segment(
                    [
                        Pos2::new(r.left() + 10.0, yy + 4.5),
                        Pos2::new(r.right() - 10.0, yy + 4.5),
                    ],
                    Stroke::new(1.0, c.border),
                );
                yy += 9.0;
                continue;
            };
            let rr =
                Rect::from_min_size(Pos2::new(r.left() + 4.0, yy), Vec2::new(w - 8.0, MENU_ROW));
            yy += MENU_ROW;
            if cm.sel == Some(i) && *enabled {
                p.rect_filled(rr, 4.0, c.pop_sel);
            }
            let col = if *enabled {
                c.fg
            } else {
                mix(c.fg, c.pop, 0.6)
            };
            p.text(
                Pos2::new(rr.left() + 12.0, rr.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                col,
            );
            p.text(
                Pos2::new(rr.right() - 12.0, rr.center().y),
                Align2::RIGHT_CENTER,
                keys,
                small.clone(),
                c.dim,
            );
            let resp = ui.interact(rr, Id::unique(("ctx", i)), Sense::click());
            if resp.hovered() && *enabled {
                hovered = Some(i);
            }
            if resp.clicked() && *enabled {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.app.context_activate(i);
            return;
        }
        if let Some(i) = hovered
            && let Some(cm) = &mut self.app.context_menu
        {
            cm.sel = Some(i);
        }
        // A click outside closes it (the right-click that opened it doesn't count).
        let (pressed, pos, secondary) = ui.ctx().input(|i| {
            (
                i.pointer.any_pressed(),
                i.pointer.interact_pos(),
                i.pointer.secondary_pressed(),
            )
        });
        if pressed
            && !secondary
            && let Some(pos) = pos
            && !r.contains(pos)
        {
            self.app.context_menu = None;
        }
    }
}
