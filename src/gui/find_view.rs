//! GUI: the find panel (Find, Replace, Find in Files) at the bottom of the window.

use super::*;
use crate::app::{FindField, FindMode, FindOpt, opt_command, opt_name};
use crate::commands::Command;

/// Height of one row of the panel.
pub(super) const FIND_ROW: f32 = 34.0;

/// What a click on the panel asks for (applied after drawing).
enum Click {
    Field(FindField),
    Opt(FindOpt),
    Cmd(Command),
    Files(bool),
    AddFolder,
}

impl Gui {
    /// The panel's height for its mode (0 when closed).
    pub(super) fn find_panel_height(&self) -> f32 {
        if !self.app.find.open {
            return 0.0;
        }
        self.app.find.fields().len() as f32 * FIND_ROW + 8.0
    }

    pub(super) fn draw_find_panel(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        p.rect_filled(r, 0.0, c.chrome);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, c.border));
        let count = self.app.find_count();
        let ui_font = FontId::proportional(UI_SIZE);
        let small = FontId::proportional(11.5);
        let mode = self.app.find.mode;
        let focused = self.app.focus == Focus::Find;
        let mut click: Option<Click> = None;
        let pointer = ui.ctx().pointer_hover_pos();

        // Toggles, left of the first row.
        let opts = self.app.find.opts();
        let top = r.top() + 4.0;
        let mut x = r.left() + 10.0;
        for &o in opts {
            let label = match o {
                FindOpt::Regex => ".*",
                FindOpt::Case => "Aa",
                FindOpt::Word => "\"ab\"",
                FindOpt::Wrap => "Wrap",
                FindOpt::InSelection => "In sel.",
                FindOpt::Context => "Context",
            };
            let w = p
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(label.into(), small.clone(), c.fg).size().x)
                .max(16.0)
                + 14.0;
            let b = Rect::from_min_size(Pos2::new(x, top + 5.0), Vec2::new(w, FIND_ROW - 10.0));
            x += w + 4.0;
            let on = self.app.find_opt(o);
            let hovered = pointer.is_some_and(|q| b.contains(q));
            p.rect_filled(
                b,
                3.0,
                if on {
                    mix(c.chrome, c.accent, 0.35)
                } else if hovered {
                    mix(c.chrome, c.fg, 0.08)
                } else {
                    c.chrome
                },
            );
            p.rect_stroke(
                b,
                3.0,
                Stroke::new(1.0, if on { c.accent } else { c.border }),
                StrokeKind::Inside,
            );
            p.text(
                b.center(),
                Align2::CENTER_CENTER,
                label,
                small.clone(),
                if on { c.fg } else { c.dim },
            );
            let keys = self.app.keymap.shortcut(opt_command(o));
            let tip = if keys.is_empty() {
                format!("{} ({})", opt_name(o), if on { "on" } else { "off" })
            } else {
                format!(
                    "{} ({}) — {keys}",
                    opt_name(o),
                    if on { "on" } else { "off" }
                )
            };
            let resp = ui
                .interact(b, Id::unique(("find-opt", o as u8)), Sense::click())
                .on_hover_text(tip);
            if resp.clicked() {
                click = Some(Click::Opt(o));
            }
        }
        let label_x = x + 8.0;
        let label_w = 58.0;

        // Buttons per row, right-aligned.
        let buttons = |f: FindField| -> Vec<(&'static str, Click)> {
            match (mode, f) {
                (FindMode::Files, FindField::Find) => vec![("Find", Click::Files(false))],
                (FindMode::Files, FindField::Where) => vec![("Add Folder…", Click::AddFolder)],
                (FindMode::Files, FindField::Replace) => vec![("Replace", Click::Files(true))],
                (_, FindField::Replace) => vec![
                    ("Replace", Click::Cmd(Command::ReplaceNext)),
                    ("Replace All", Click::Cmd(Command::ReplaceAll)),
                ],
                _ => vec![
                    ("Find", Click::Cmd(Command::FindNext)),
                    ("Prev", Click::Cmd(Command::FindPrev)),
                ],
            }
        };
        let fields = self.app.find.fields();
        for (row, &f) in fields.iter().enumerate() {
            let y0 = top + row as f32 * FIND_ROW;
            let rr = Rect::from_min_max(
                Pos2::new(label_x, y0),
                Pos2::new(r.right() - 10.0, y0 + FIND_ROW),
            );
            let name = match f {
                FindField::Find => "Find",
                FindField::Where => "Where",
                FindField::Replace => "Replace",
            };
            p.text(
                Pos2::new(rr.left() + label_w, rr.center().y),
                Align2::RIGHT_CENTER,
                name,
                ui_font.clone(),
                c.dim,
            );
            // Buttons from the right edge.
            let mut bx = rr.right();
            // Placed from the right edge, so the first button ends up leftmost.
            for (text, action) in buttons(f).into_iter().rev() {
                let w = p.ctx().fonts_mut(|g| {
                    g.layout_no_wrap(text.into(), ui_font.clone(), c.fg)
                        .size()
                        .x
                }) + 22.0;
                let b = Rect::from_min_max(
                    Pos2::new(bx - w, rr.top() + 5.0),
                    Pos2::new(bx, rr.bottom() - 5.0),
                );
                bx -= w + 6.0;
                let resp = ui.interact(b, Id::unique(("find-btn", row, text)), Sense::click());
                p.rect_filled(
                    b,
                    3.0,
                    if resp.hovered() {
                        mix(c.chrome, c.fg, 0.14)
                    } else {
                        mix(c.chrome, c.fg, 0.07)
                    },
                );
                p.rect_stroke(b, 3.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
                p.text(
                    b.center(),
                    Align2::CENTER_CENTER,
                    text,
                    ui_font.clone(),
                    c.fg,
                );
                if resp.clicked() {
                    click = Some(action);
                }
            }
            // Count after the Find field; an error after the field it's about.
            let mut status_w = 0.0;
            let err = self
                .app
                .find
                .error
                .as_ref()
                .filter(|e| e.0 == f)
                .map(|e| e.1.clone());
            let status: Option<(String, Color32)> = match (f, &err, count) {
                (_, Some(e), _) => Some((e.clone(), c.err)),
                (FindField::Find, None, Some((_, 0))) => Some(("No results".into(), c.err)),
                (FindField::Find, None, Some((Some(i), n))) => {
                    Some((format!("{} of {}", i + 1, n), c.dim))
                }
                (FindField::Find, None, Some((None, n))) => Some((
                    format!("{n} match{}", if n == 1 { "" } else { "es" }),
                    c.dim,
                )),
                _ => None,
            };
            let bad = err.is_some() || (f == FindField::Find && matches!(count, Some((_, 0))));
            if let Some((s, col)) = status {
                let s: String = if s.chars().count() > 48 {
                    s.chars().take(47).chain(['…']).collect()
                } else {
                    s
                };
                let g = p.text(
                    Pos2::new(bx - 4.0, rr.center().y),
                    Align2::RIGHT_CENTER,
                    s,
                    small.clone(),
                    col,
                );
                status_w = g.width() + 12.0;
            }
            let field = Rect::from_min_max(
                Pos2::new(rr.left() + label_w + 10.0, rr.top() + 4.0),
                Pos2::new(bx - 4.0 - status_w, rr.bottom() - 4.0),
            );
            if field.width() < 40.0 {
                continue;
            }
            let active = focused && self.app.find.field == f;
            p.rect_filled(field, 3.0, c.bg);
            p.rect_stroke(
                field,
                3.0,
                Stroke::new(
                    1.0,
                    if bad {
                        c.err
                    } else if active {
                        c.accent
                    } else {
                        c.border
                    },
                ),
                StrokeKind::Inside,
            );
            let placeholder = match f {
                FindField::Find => "",
                FindField::Where => "<project folders> — or folders, *.rs, -target/",
                FindField::Replace if self.app.find_opts.regex => "Replace with ($1 = group 1)",
                FindField::Replace => "Replace with",
            };
            draw_line_edit(
                p,
                field.shrink(2.0),
                field.left() + 8.0,
                self.app.find.field_ref(f),
                FontId::monospace(UI_SIZE),
                c,
                active,
                placeholder,
            );
            if ui
                .interact(field, Id::unique(("find-field", row)), Sense::click())
                .clicked()
            {
                click = Some(Click::Field(f));
            }
        }
        match click {
            Some(Click::Field(f)) => self.app.focus_find_field(f),
            Some(Click::Opt(o)) => self.app.toggle_find_opt(o),
            Some(Click::Cmd(cmd)) => self.app.run(cmd),
            Some(Click::Files(replace)) => self.app.run_find_in_files(replace),
            Some(Click::AddFolder) => {
                self.app.file_request = Some(crate::app::FileRequest::FindFolder)
            }
            None => {}
        }
    }
}
