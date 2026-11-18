//! GUI: LSP Info, the language server manager and confirmations.

use super::*;

impl Gui {
    pub(super) fn draw_lsp_info(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        full: Rect,
        c: &Palette,
    ) {
        use crate::app::Tone;
        let Some(Modal::LspInfo(v)) = &self.app.modal else {
            return;
        };
        let lines = self.app.lsp_info();
        block(ui, full, "lsp-info");
        p.rect_filled(full, 0.0, Color32::from_black_alpha(90));
        let r = Rect::from_center_size(
            full.center(),
            Vec2::new(
                (full.width() * 0.75).clamp(520.0, 980.0),
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
        p.text(
            Pos2::new(r.left() + 18.0, r.top() + 24.0),
            Align2::LEFT_CENTER,
            "LSP Info",
            FontId::proportional(16.0),
            c.fg,
        );
        p.text(
            Pos2::new(r.right() - 18.0, r.top() + 24.0),
            Align2::RIGHT_CENTER,
            "i install · r restart servers · s server manager · Up/Down scroll · Esc close",
            FontId::proportional(11.5),
            c.dim,
        );
        let green = Color32::from_rgb(0x7F, 0xD1, 0x7A);
        let body = Rect::from_min_max(
            Pos2::new(r.left() + 10.0, r.top() + 46.0),
            Pos2::new(r.right() - 10.0, r.bottom() - 10.0),
        );
        let lh = 20.0;
        let n_vis = ((body.height() / lh).floor() as usize).max(1);
        let max_scroll = lines.len().saturating_sub(n_vis);
        let resp = ui.interact(body, Id::unique("lspinfo"), Sense::hover());
        let mut scroll = v.scroll.min(max_scroll);
        if resp.hovered() {
            let dy = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                scroll = ((scroll as f32 - dy / lh).round().max(0.0) as usize).min(max_scroll);
            }
        }
        let clip = p.with_clip_rect(body);
        for (row, (tone, text)) in lines.iter().enumerate().skip(scroll).take(n_vis) {
            let y = body.top() + (row - scroll) as f32 * lh + lh / 2.0;
            let (font, col) = match tone {
                Tone::Heading => (FontId::proportional(12.5), c.accent),
                Tone::Text => (FontId::monospace(12.5), c.fg),
                Tone::Dim => (FontId::monospace(12.0), c.dim),
                Tone::Good => (FontId::monospace(12.5), green),
                Tone::Warn => (FontId::monospace(12.5), c.warn),
                Tone::Bad => (FontId::monospace(12.5), c.err),
            };
            clip.text(
                Pos2::new(body.left() + 8.0, y),
                Align2::LEFT_CENTER,
                text,
                font,
                col,
            );
        }
        if !self.app.installing.is_empty()
            || self.app.lsp.clients.values().any(|c| c.alive && !c.ready)
        {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        if let Some(Modal::LspInfo(v)) = &mut self.app.modal {
            v.scroll = scroll;
        }
    }

    pub(super) fn draw_servers(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        use crate::app::ServerStatus as St;
        let Some(Modal::Servers(v)) = &self.app.modal else {
            return;
        };
        let rows = self.app.server_rows();
        let sel = v.sel.min(rows.len().saturating_sub(1));
        let details = v.details;
        block(ui, full, "servers");
        p.rect_filled(full, 0.0, Color32::from_black_alpha(90));
        let r = Rect::from_center_size(
            full.center(),
            Vec2::new(
                (full.width() * 0.75).clamp(520.0, 960.0),
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
        let small = FontId::proportional(11.5);
        p.text(
            Pos2::new(r.left() + 18.0, r.top() + 24.0),
            Align2::LEFT_CENTER,
            "Language Servers",
            FontId::proportional(16.0),
            c.fg,
        );
        p.text(
            Pos2::new(r.right() - 18.0, r.top() + 24.0),
            Align2::RIGHT_CENTER,
            "i install · u update · x uninstall · Enter details · Esc close",
            small.clone(),
            c.dim,
        );
        let data = self
            .app
            .config
            .data_dir
            .clone()
            .map(|d| d.join("servers").display().to_string())
            .unwrap_or_default();
        p.text(
            Pos2::new(r.left() + 18.0, r.top() + 46.0),
            Align2::LEFT_CENTER,
            format!("Installs go to {data}"),
            small.clone(),
            c.dim,
        );
        let green = Color32::from_rgb(0x7F, 0xD1, 0x7A);
        let detail_h = if details {
            (r.height() * 0.42).max(160.0)
        } else {
            0.0
        };
        let list = Rect::from_min_max(
            Pos2::new(r.left() + 8.0, r.top() + 62.0),
            Pos2::new(r.right() - 8.0, r.bottom() - 8.0 - detail_h),
        );
        let lh = 30.0;
        let n_vis = ((list.height() / lh).floor() as usize).max(1);
        let first = sel.saturating_sub(n_vis - 1);
        let clip = p.with_clip_rect(list);
        let mut clicked = None;
        for (row, (i, srv)) in rows.iter().enumerate().skip(first).take(n_vis).enumerate() {
            let rr = Rect::from_min_size(
                Pos2::new(list.left(), list.top() + row as f32 * lh),
                Vec2::new(list.width(), lh),
            );
            let resp = ui.interact(rr, Id::unique(("srv", i)), Sense::click());
            if i == sel {
                clip.rect_filled(rr, 4.0, c.pop_sel);
            } else if resp.hovered() {
                clip.rect_filled(rr, 4.0, mix(c.pop, c.fg, 0.05));
            }
            let st = self.app.server_status(srv);
            let (dot, label) = match &st {
                St::Installed(_) => (green, "installed".to_string()),
                St::OnPath(_) => (c.info, "on PATH".to_string()),
                St::Installing => (c.accent, "installing…".to_string()),
                St::Failed(_) => (c.err, "install failed".to_string()),
                St::Missing if srv.install.as_ref().is_some_and(|i| i.supported_here()) => {
                    (c.dim, "not installed".to_string())
                }
                St::Missing => (c.dim, "not found (install manually)".to_string()),
            };
            clip.circle_filled(Pos2::new(rr.left() + 16.0, rr.center().y), 4.5, dot);
            let nr = clip.text(
                Pos2::new(rr.left() + 30.0, rr.center().y),
                Align2::LEFT_CENTER,
                &srv.name,
                FontId::proportional(14.0),
                c.fg,
            );
            let langs = self.app.config.languages_using(&srv.name).join(", ");
            clip.text(
                Pos2::new(nr.right() + 14.0, rr.center().y),
                Align2::LEFT_CENTER,
                langs,
                small.clone(),
                c.dim,
            );
            clip.text(
                Pos2::new(rr.right() - 12.0, rr.center().y),
                Align2::RIGHT_CENTER,
                label,
                small.clone(),
                dot,
            );
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if details && let Some(srv) = rows.get(sel) {
            let d = Rect::from_min_max(
                Pos2::new(r.left() + 14.0, list.bottom() + 6.0),
                Pos2::new(r.right() - 14.0, r.bottom() - 12.0),
            );
            p.rect_filled(d, 4.0, c.bg);
            let st = self.app.server_status(srv);
            let location = match &st {
                St::Installed(b) | St::OnPath(b) => b.display().to_string(),
                _ => "—".into(),
            };
            let mut lines = vec![
                srv.description.clone(),
                format!("command   {} {}", srv.command, srv.args.join(" ")),
                format!(
                    "languages {}",
                    self.app.config.languages_using(&srv.name).join(", ")
                ),
                format!(
                    "roots     {}",
                    if srv.root_markers.is_empty() {
                        "(git root or file folder)".into()
                    } else {
                        srv.root_markers.join(", ")
                    }
                ),
                format!(
                    "install   {}",
                    srv.install
                        .as_ref()
                        .map(|i| i.source())
                        .unwrap_or_else(|| "manual".into())
                ),
                format!("location  {location}"),
            ];
            if let St::Failed(e) = &st {
                lines.push(format!("error     {e}"));
            }
            if let Some(log) = self.app.install_log.get(&srv.name) {
                lines.push(String::new());
                lines.extend(
                    log.iter()
                        .rev()
                        .take(8)
                        .rev()
                        .map(|l| l.replace(['\n', '\r'], " ")),
                );
            }
            let dc = p.with_clip_rect(d.shrink(6.0));
            for (k, l) in lines.iter().enumerate() {
                dc.text(
                    Pos2::new(d.left() + 12.0, d.top() + 16.0 + k as f32 * 18.0),
                    Align2::LEFT_CENTER,
                    l,
                    FontId::monospace(12.0),
                    if k == 0 { c.fg } else { c.dim },
                );
            }
        }
        if rows.is_empty() {
            p.text(
                list.left_top() + Vec2::new(18.0, 16.0),
                Align2::LEFT_CENTER,
                "No servers configured",
                font,
                c.dim,
            );
        }
        if self.app.installing.is_empty() {
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        if let Some(i) = clicked
            && let Some(Modal::Servers(v)) = &mut self.app.modal
        {
            if v.sel == i {
                v.details = !v.details;
            }
            v.sel = i;
        }
    }

    pub(super) fn draw_confirm(&self, ui: &egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        let Some(Modal::Confirm(cf)) = &self.app.modal else {
            return;
        };
        block(ui, full, "confirm");
        p.rect_filled(full, 0.0, Color32::from_black_alpha(80));
        let font = FontId::proportional(14.0);
        let tw = p.ctx().fonts_mut(|f| {
            f.layout_no_wrap(cf.message.clone(), font.clone(), c.fg)
                .size()
                .x
        });
        let w = (tw + 60.0).clamp(320.0, full.width() - 40.0);
        let r = Rect::from_center_size(
            full.center() - Vec2::new(0.0, full.height() * 0.15),
            Vec2::new(w, 110.0),
        );
        p.rect_filled(r, 6.0, c.pop);
        p.rect_stroke(r, 6.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        p.text(
            Pos2::new(r.center().x, r.top() + 36.0),
            Align2::CENTER_CENTER,
            &cf.message,
            font,
            c.fg,
        );
        p.text(
            Pos2::new(r.center().x, r.bottom() - 30.0),
            Align2::CENTER_CENTER,
            "Y / Enter — yes        N / Esc — no",
            FontId::proportional(12.0),
            c.dim,
        );
    }
}
