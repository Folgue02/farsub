//! GUI: the status bar.

use super::*;

impl Gui {
    // ================================================================= status bar

    pub(super) fn draw_status(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        p.rect_filled(r, 0.0, c.chrome);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, c.border));
        let font = FontId::proportional(12.0);
        let y = r.center().y;
        let mut left = String::new();
        let app = &self.app;
        if let Some(b) = app.buf() {
            left = format!("Line {}, Column {}", b.cursor.line + 1, b.cursor.col + 1);
            if b.multi() {
                // "3 selection regions" instead of the position.
                left = format!("{} selection regions", b.extra.len() + 1);
            } else if let Some((a, z)) = b.selection() {
                let n = b.text_range(a, z).chars().count();
                left.push_str(&format!("; {n} characters selected"));
            }
            if let Some(ds) = b.path.as_ref().and_then(|p| app.diagnostics.get(p)) {
                let e = ds.iter().filter(|d| d.severity == 1).count();
                let w = ds.iter().filter(|d| d.severity == 2).count();
                if e + w > 0 {
                    left.push_str(&format!(
                        "    {e} error{}, {w} warning{}",
                        if e == 1 { "" } else { "s" },
                        if w == 1 { "" } else { "s" }
                    ));
                }
            }
        }
        let x = p.text(
            Pos2::new(r.left() + 10.0, y),
            Align2::LEFT_CENTER,
            &left,
            font.clone(),
            c.dim,
        );
        // A status message, else the diagnostic under the cursor (else on its line), the
        // most severe first, colored by severity.
        let msg = app
            .status
            .as_ref()
            .map(|s| (s.0.clone(), c.accent))
            .or_else(|| {
                let (sev, text) = app.diagnostic_at_cursor()?;
                let col = match sev {
                    1 => c.err,
                    2 => c.warn,
                    3 => c.info,
                    _ => c.dim,
                };
                Some((text, col))
            });
        // Right side: each item is clickable.
        let mut right: Vec<(String, Option<crate::commands::Command>)> = vec![];
        if let Some(b) = app.buf() {
            let lsp = app.lsp_status();
            let has_servers = b
                .lang
                .as_ref()
                .and_then(|l| app.config.language(l))
                .is_some_and(|l| !l.servers.is_empty());
            if !lsp.is_empty() {
                right.push((lsp, Some(crate::commands::Command::LspInfo)));
            } else if has_servers {
                right.push(("no LSP".into(), Some(crate::commands::Command::LspInfo)));
            }
            if !b.encoding.is_utf8() {
                // The encoding shows only when it isn't plain UTF-8.
                right.push((
                    b.encoding.label(),
                    Some(crate::commands::Command::ConvertToUtf8),
                ));
            }
            right.push((
                if b.indent_unit == "\t" {
                    format!("Tab Size: {}", b.tab_size)
                } else {
                    format!("Spaces: {}", b.indent_unit.len())
                },
                Some(crate::commands::Command::Indentation),
            ));
            right.push((
                self.app.language_title(&b.hl.syntax),
                Some(crate::commands::Command::SetSyntax),
            ));
        }
        let mut x_right = r.right() - 6.0;
        let mut clicked = None;
        self.status_hits.clear();
        for (i, (text, cmd)) in right.iter().enumerate().rev() {
            let w = ui
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), c.dim).size().x);
            let item = Rect::from_min_max(
                Pos2::new(x_right - w - 16.0, r.top() + 2.0),
                Pos2::new(x_right, r.bottom() - 1.0),
            );
            x_right = item.left() - 4.0;
            let mut col = c.dim;
            if let Some(cmd) = cmd {
                self.status_hits.push((*cmd, item));
            }
            if cmd.is_some() {
                let resp = ui.interact(item, Id::unique(("status", i)), Sense::click());
                if resp.hovered() {
                    p.rect_filled(item, 3.0, mix(c.chrome, c.fg, 0.08));
                    col = c.fg;
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    clicked = *cmd;
                }
            }
            p.text(
                item.center(),
                Align2::CENTER_CENTER,
                text,
                font.clone(),
                col,
            );
        }
        let rr = Rect::from_min_max(Pos2::new(x_right, r.top()), r.max);
        if let Some(cmd) = clicked {
            self.app.run(cmd);
        }
        if let Some((m, col)) = msg {
            let clip = p.with_clip_rect(Rect::from_min_max(
                Pos2::new(x.right() + 24.0, r.top()),
                Pos2::new(rr.left() - 24.0, r.bottom()),
            ));
            clip.text(
                Pos2::new(x.right() + 24.0, y),
                Align2::LEFT_CENTER,
                m,
                font,
                col,
            );
        }
    }
}
