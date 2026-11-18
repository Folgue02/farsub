//! GUI: the sidebar (folder tree).

use super::*;

impl Gui {
    // ================================================================= sidebar

    pub(super) fn draw_sidebar(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        p.rect_filled(r, 0.0, c.side);
        p.line_segment(
            [r.right_top(), r.right_bottom()],
            Stroke::new(1.0, c.border),
        );
        let focused = self.app.focus == Focus::Sidebar && self.app.modal.is_none();
        let ui_font = FontId::proportional(UI_SIZE);
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), TAB_H));
        p.text(
            Pos2::new(r.left() + 14.0, head.center().y),
            Align2::LEFT_CENTER,
            "FOLDERS",
            FontId::proportional(11.0),
            if focused { c.accent } else { c.dim },
        );
        // No folder open (an empty window): no project name in the header.
        let rootname = if self.app.project.folders.is_empty() && self.app.project.file.is_none() {
            String::new()
        } else {
            self.app.project.name()
        };
        p.with_clip_rect(
            head.shrink2(Vec2::new(70.0, 0.0))
                .translate(Vec2::new(30.0, 0.0)),
        )
        .text(
            Pos2::new(r.right() - 10.0, head.center().y),
            Align2::RIGHT_CENTER,
            rootname,
            FontId::proportional(11.0),
            c.dim,
        );
        let list = Rect::from_min_max(
            Pos2::new(r.left(), head.bottom()),
            Pos2::new(r.right() - 1.0, r.bottom()),
        );
        let rows = ((list.height() / ROW_H).floor() as usize).max(1);
        if self.app.tree.tick() {
            ui.ctx().request_repaint();
        }
        let t = &mut self.app.tree;
        // Follow keyboard selection; leave mouse-wheel scrolling alone otherwise.
        if self.last_tree_sel != Some(t.sel) || std::mem::take(&mut t.follow) {
            if t.sel < t.scroll {
                t.scroll = t.sel;
                self.side_px = 0.0;
            } else if t.sel >= t.scroll + rows {
                t.scroll = t.sel + 1 - rows;
                self.side_px = 0.0;
            }
            self.last_tree_sel = Some(t.sel);
        }
        let resp = ui.interact(list, Id::unique("sidebar"), Sense::click());
        if resp.hovered() {
            let dy = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                // Smooth: pixels, with whole rows carried into the row position.
                let max = t.nodes.len().saturating_sub(rows);
                let mut px = self.side_px - dy;
                while px >= ROW_H && t.scroll < max {
                    t.scroll += 1;
                    px -= ROW_H;
                }
                while px < 0.0 && t.scroll > 0 {
                    t.scroll -= 1;
                    px += ROW_H;
                }
                self.side_px = if t.scroll >= max {
                    0.0
                } else {
                    px.clamp(0.0, ROW_H - 0.01)
                };
            }
        }
        let hover_pos = resp.hover_pos();
        let current = self
            .app
            .buffers
            .get(self.app.cur)
            .and_then(|b| b.path.clone());
        let cut: Vec<PathBuf> = match &self.app.file_clip {
            Some((v, true)) => v.clone(),
            _ => vec![],
        };
        let t = &self.app.tree;
        let clip = p.with_clip_rect(list);
        // Rows have animated heights while a folder opens/closes.
        let mut y = list.top() - self.side_px;
        let mut hits: Vec<(usize, Rect)> = vec![];
        let mut i = t.scroll;
        while y < list.bottom() {
            let Some(n) = t.nodes.get(i) else { break };
            let f = t.row_factor(i);
            let h = ROW_H * f;
            if h < 0.5 {
                i += 1;
                continue;
            }
            let rr = Rect::from_min_size(Pos2::new(list.left(), y), Vec2::new(list.width(), h));
            let row_clip = clip.with_clip_rect(rr);
            // Content slides in from above as the row grows.
            let full = Rect::from_min_size(
                Pos2::new(rr.left(), rr.bottom() - ROW_H),
                Vec2::new(rr.width(), ROW_H),
            );
            let alpha = f.powf(1.5);
            let fade = |col: Color32| mix(c.side, col, alpha);
            let marked = t.marked.contains(&n.path);
            let hovered = hover_pos.is_some_and(|hp| rr.contains(hp));
            if i == t.sel {
                row_clip.rect_filled(
                    rr,
                    0.0,
                    fade(if focused {
                        c.pop_sel
                    } else {
                        mix(c.side, c.sel, 0.6)
                    }),
                );
            } else if marked {
                row_clip.rect_filled(rr, 0.0, fade(mix(c.side, c.accent, 0.12)));
            } else if hovered {
                row_clip.rect_filled(rr, 0.0, mix(c.side, c.fg, 0.07));
            }
            if marked {
                row_clip.rect_filled(
                    Rect::from_min_size(rr.min, Vec2::new(3.0, h)),
                    0.0,
                    c.accent,
                );
            }
            let x = list.left() + 12.0 + n.depth as f32 * 14.0;
            if n.is_dir {
                disclosure(
                    &row_clip,
                    Pos2::new(x + 3.0, full.center().y),
                    t.arrow(&n.path),
                    fade(if hovered { c.fg } else { c.dim }),
                );
            }
            draw_icon(
                &row_clip,
                &self.app.config,
                &mut self.icon_tex,
                Pos2::new(x + 21.0, full.center().y),
                &n.name,
                n.is_dir,
                n.is_dir && t.arrow(&n.path) > 0.5,
                alpha,
                c.side,
            );
            let is_cur = current.as_deref() == Some(n.path.as_path());
            let is_cut = cut.iter().any(|p| n.path.starts_with(p));
            let mut col = if is_cur {
                c.accent
            } else if n.is_dir {
                c.fg
            } else {
                mix(c.fg, c.side, 0.15)
            };
            if is_cut {
                col = mix(col, c.side, 0.55);
            }
            if n.ignored {
                // Matched by .gitignore: listed, but dimmed (like Sublime / VS Code).
                col = mix(col, c.side, 0.5);
            }
            row_clip.text(
                Pos2::new(x + 33.0, full.center().y),
                Align2::LEFT_CENTER,
                &n.name,
                ui_font.clone(),
                fade(col),
            );
            if i == t.sel {
                self.sidebar_sel_px = Some(Pos2::new(rr.left() + 24.0, rr.bottom()));
            }
            hits.push((i, rr));
            y += h;
            i += 1;
        }
        if t.nodes.is_empty() {
            let clip = p.with_clip_rect(list);
            let lines: &[&str] = if self.app.project.folders.is_empty() {
                &["No folder open", "File › Open Folder…"]
            } else {
                &["(empty folder — press a to add a file)"]
            };
            for (i, l) in lines.iter().enumerate() {
                clip.text(
                    Pos2::new(list.left() + 14.0, list.top() + 14.0 + i as f32 * 20.0),
                    Align2::LEFT_CENTER,
                    *l,
                    ui_font.clone(),
                    c.dim,
                );
            }
        }
        // A row reacts on press (folders open/close, files get selected); a double-click
        // opens a file (folders already toggled on each press).
        let hit = |pos: Pos2| hits.iter().find(|(_, r)| r.contains(pos)).map(|h| h.0);
        if pressed(ui, &resp)
            && let Some(i) = resp.hover_pos().and_then(hit)
        {
            let dir = self.app.tree.nodes.get(i).is_some_and(|n| n.is_dir);
            self.app.tree_click(i, dir);
        } else if resp.double_clicked()
            && let Some(i) = resp.interact_pointer_pos().and_then(hit)
            && self.app.tree.nodes.get(i).is_some_and(|n| !n.is_dir)
        {
            self.app.tree_click(i, true);
        }
        // Right-click selects the row (if any) and opens the sidebar menu.
        if resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            if let Some(&(i, _)) = hits.iter().find(|(_, r)| r.contains(pos)) {
                self.app.tree_click(i, false);
            }
            self.app.focus = Focus::Sidebar;
            self.app
                .open_context_menu(crate::menu::ContextKind::Sidebar, (pos.x, pos.y));
        }
    }
}
