//! The Sublime-style GUI: everything is custom-painted with egui's painter so the editor,
//! sidebar, tabs and overlays look and behave like a code editor rather than a form.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, FontId, Id, LayerId, Order, Painter, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2, text::LayoutJob,
};
use unicode_width::UnicodeWidthChar;

use crate::app::{App, AppEvent, EventTx, Focus, Modal, PromptKind};
use crate::config::Config;
use crate::editor::{Buffer, Pos};
use crate::highlight::Span;
use crate::keys::Key;
use crate::palette::Mode;

const TAB: usize = 4;
const UI_SIZE: f32 = 13.0;
const TAB_H: f32 = 32.0;
const STATUS_H: f32 = 24.0;
const PANEL_H: f32 = 36.0;
const SIDEBAR_W: f32 = 250.0;
const ROW_H: f32 = 24.0;
const MENU_H: f32 = 26.0;
const MENU_ROW: f32 = 26.0;

struct Palette {
    bg: Color32,
    fg: Color32,
    dim: Color32,
    sel: Color32,
    line: Color32,
    gutter: Color32,
    side: Color32,
    chrome: Color32,
    border: Color32,
    accent: Color32,
    pop: Color32,
    pop_sel: Color32,
    err: Color32,
    warn: Color32,
    info: Color32,
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn colors(app: &App) -> Palette {
    let bg = app.syn.bg();
    let fg = app.syn.fg();
    let dark = (bg.r() as u32 + bg.g() as u32 + bg.b() as u32) < 384;
    let shade = if dark { Color32::BLACK } else { Color32::WHITE };
    Palette {
        bg,
        fg,
        dim: mix(fg, bg, 0.45),
        sel: app.syn.selection(),
        line: app.syn.line_highlight(),
        gutter: app.syn.gutter(),
        side: mix(bg, shade, 0.18),
        chrome: mix(bg, shade, 0.30),
        border: mix(bg, shade, 0.45),
        accent: Color32::from_rgb(0xF9, 0xAE, 0x58),
        pop: mix(bg, if dark { Color32::WHITE } else { Color32::BLACK }, 0.07),
        pop_sel: mix(
            app.syn.selection(),
            Color32::from_rgb(0x56, 0x8A, 0xF2),
            0.35,
        ),
        err: Color32::from_rgb(0xF0, 0x50, 0x50),
        warn: Color32::from_rgb(0xE6, 0xBE, 0x3C),
        info: Color32::from_rgb(0x64, 0xA0, 0xE6),
    }
}

pub struct Gui {
    pub app: App,
    rx: Receiver<AppEvent>,
    suppress_text: bool,
    scroll_acc: f32,
    last_cursor: Option<(usize, Pos, i32)>,
    last_tree_sel: Option<usize>,
    last_title: String,
    find_count: Option<(String, i32, usize)>,
    pub menu_titles: Vec<Rect>,
    #[cfg(target_os = "macos")]
    native_menu: Option<crate::native_menu::NativeMenu>,
}

impl Gui {
    pub fn new(cc: &eframe::CreationContext<'_>, root: PathBuf, config: Config) -> Self {
        let ctx = cc.egui_ctx.clone();
        let mut g = Self::headless(root, config);
        let tx = g.app_tx();
        tx.set_waker(move || ctx.request_repaint());
        #[cfg(target_os = "macos")]
        {
            g.native_menu = crate::native_menu::NativeMenu::install(&g.app, tx.clone());
        }
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        g.last_title.clear();
        g
    }

    /// Construct without a window (tests drive it through egui_kittest).
    pub fn headless(root: PathBuf, config: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let app = App::new(root, config, EventTx::new(tx));
        Gui {
            app,
            rx,
            suppress_text: false,
            scroll_acc: 0.0,
            last_cursor: None,
            last_tree_sel: None,
            last_title: String::new(),
            find_count: None,
            menu_titles: vec![],
            #[cfg(target_os = "macos")]
            native_menu: None,
        }
    }

    fn app_tx(&mut self) -> EventTx {
        self.app.event_tx()
    }

    /// Pump background events (LSP, file index) without drawing.
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        while let Ok(ev) = self.rx.try_recv() {
            self.app.on_app_event(ev);
            any = true;
        }
        self.app.sync_lsp();
        any
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            match ev {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    self.suppress_text = false;
                    if let Some(k) = Key::from_egui(key, modifiers) {
                        let handled = self.app.on_key(k);
                        if handled && k.alt {
                            // Alt+letter produced a shortcut; drop the character it also typed.
                            self.suppress_text = true;
                        }
                    }
                }
                egui::Event::Text(s) => {
                    if std::mem::take(&mut self.suppress_text) {
                        continue;
                    }
                    self.app.on_text(&s);
                }
                egui::Event::Ime(egui::ImeEvent::Commit(s)) => self.app.on_text(&s),
                egui::Event::Paste(s) => self.app.on_paste(&s),
                egui::Event::Copy => self.app.on_copy(false),
                egui::Event::Cut => self.app.on_copy(true),
                _ => {}
            }
        }
        // The character an Alt-shortcut types arrives in the same batch; never carry the
        // flag into a later frame.
        self.suppress_text = false;
    }

    /// One full frame: input, background events, drawing.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.pump();
        self.handle_input(&ctx);
        self.pump();

        if ctx.input(|i| i.viewport().close_requested())
            && !self.app.quit
            && self.app.any_modified()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.app.run(crate::commands::Command::Quit);
        }
        if self.app.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        #[cfg(target_os = "macos")]
        if let Some(m) = &mut self.native_menu {
            m.sync(&self.app);
        }
        if let Some(t) = self.app.pending_copy.take() {
            ctx.copy_text(t);
        }
        let title = self.app.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        self.update_find_count();
        // The whole window, regardless of any margin on the Ui we were handed.
        let full = ctx.content_rect();
        let c = colors(&self.app);
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, c.bg);

        let mut rest = full;
        let show_bar = self.app.menu_bar_visible || self.app.menu.is_some();
        let bar = show_bar.then(|| {
            let r = Rect::from_min_size(full.min, Vec2::new(full.width(), MENU_H));
            rest.min.y = r.bottom();
            r
        });
        let status = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - STATUS_H), full.max);
        rest.max.y = status.top();
        let panel = matches!(&self.app.modal, Some(Modal::Prompt(_))).then(|| {
            let r = Rect::from_min_max(Pos2::new(rest.left(), rest.bottom() - PANEL_H), rest.max);
            rest.max.y = r.top();
            r
        });
        if self.app.sidebar_visible {
            let side =
                Rect::from_min_max(rest.min, Pos2::new(rest.left() + SIDEBAR_W, rest.bottom()));
            rest.min.x = side.right();
            self.draw_sidebar(ui, &painter, side, &c);
        }
        let tabs = Rect::from_min_max(rest.min, Pos2::new(rest.right(), rest.top() + TAB_H));
        let editor = Rect::from_min_max(Pos2::new(rest.left(), tabs.bottom()), rest.max);
        self.draw_tabs(ui, &painter, tabs, &c);
        let cursor_px = self.draw_editor(ui, &painter, editor, &c);
        self.draw_status(&painter, status, &c);
        if let Some(r) = panel {
            self.draw_prompt(&painter, r, &c);
        }

        let over = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("overlay")));
        if let Some(p) = cursor_px {
            self.draw_completion(ui, &over, editor, p, &c);
            self.draw_hover(&over, editor, p, &c);
        }
        match &self.app.modal {
            Some(Modal::Palette(_)) => self.draw_palette(ui, &over, full, &c),
            Some(Modal::Confirm(_)) => self.draw_confirm(&over, full, &c),
            Some(Modal::Keymap(_)) => self.draw_keymap(ui, &over, full, &c),
            Some(Modal::Servers(_)) => self.draw_servers(ui, &over, full, &c),
            _ => {}
        }
        if let Some(r) = bar {
            self.draw_menu_bar(ui, &painter, r, &c);
            self.draw_menu_dropdown(ui, &over, full, &c);
        }

        if self.app.hl_pending {
            ctx.request_repaint();
        }
        if self.app.status.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        self.app.tick();
    }

    fn update_find_count(&mut self) {
        let Some(Modal::Prompt(p)) = &self.app.modal else {
            return;
        };
        if !matches!(p.kind, PromptKind::Find { .. }) {
            return;
        }
        let Some(b) = self.app.buf() else { return };
        let key = (p.input.clone(), b.version);
        if self
            .find_count
            .as_ref()
            .is_some_and(|f| f.0 == key.0 && f.1 == key.1)
        {
            return;
        }
        let n = b
            .lines
            .iter()
            .map(|l| Buffer::matches_in_line(l, &key.0).len())
            .sum();
        self.find_count = Some((key.0, key.1, n));
    }

    // ================================================================= metrics

    fn mono(&self) -> FontId {
        FontId::monospace(self.app.font_size)
    }

    fn metrics(&self, ui: &egui::Ui) -> (f32, f32, f32) {
        let font = self.mono();
        let (cw, rh) = ui
            .ctx()
            .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let lh = (rh * 1.35).round();
        (cw, lh, rh)
    }

    // ================================================================= sidebar

    fn draw_sidebar(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
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
        let rootname = self
            .app
            .root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
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
        if self.last_tree_sel != Some(t.sel) {
            if t.sel < t.scroll {
                t.scroll = t.sel;
            } else if t.sel >= t.scroll + rows {
                t.scroll = t.sel + 1 - rows;
            }
            self.last_tree_sel = Some(t.sel);
        }
        let resp = ui.interact(list, Id::unique("sidebar"), Sense::click());
        if resp.hovered() {
            let dy = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                let n = t.nodes.len().saturating_sub(rows) as f32;
                let s = (t.scroll as f32 - dy / ROW_H).clamp(0.0, n);
                t.scroll = s.round() as usize;
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
        let mut y = list.top();
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
            row_clip.text(
                Pos2::new(x + 14.0, full.center().y),
                Align2::LEFT_CENTER,
                &n.name,
                ui_font.clone(),
                fade(col),
            );
            hits.push((i, rr));
            y += h;
            i += 1;
        }
        if t.nodes.is_empty() {
            p.text(
                Pos2::new(list.left() + 14.0, list.top() + 14.0),
                Align2::LEFT_CENTER,
                "(empty folder — press a to add a file)",
                ui_font,
                c.dim,
            );
        }
        if (resp.clicked() || resp.double_clicked())
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(&(i, _)) = hits.iter().find(|(_, r)| r.contains(pos))
        {
            let open =
                resp.double_clicked() || self.app.tree.nodes.get(i).is_some_and(|n| n.is_dir);
            self.app.tree_click(i, open);
        }
    }

    // ================================================================= tabs

    fn draw_tabs(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        p.rect_filled(r, 0.0, c.chrome);
        let font = FontId::proportional(UI_SIZE);
        let mut x = r.left();
        let mut click: Option<(usize, bool)> = None;
        // Scroll the strip so the active tab is visible.
        let widths: Vec<f32> = self
            .app
            .buffers
            .iter()
            .map(|b| {
                let w = ui
                    .ctx()
                    .fonts_mut(|f| f.layout_no_wrap(b.name(), font.clone(), c.fg).size().x);
                (w + 48.0).clamp(90.0, 260.0)
            })
            .collect();
        let before: f32 = widths.iter().take(self.app.cur).sum();
        let cur_w = widths.get(self.app.cur).copied().unwrap_or(0.0);
        if before + cur_w > r.width() {
            x -= before + cur_w - r.width();
        }
        let clip = p.with_clip_rect(r);
        for (i, b) in self.app.buffers.iter().enumerate() {
            let w = widths[i];
            let tr = Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w, r.height()));
            x += w;
            let active = i == self.app.cur;
            if active {
                clip.rect_filled(tr, 0.0, c.bg);
                clip.rect_filled(
                    Rect::from_min_size(tr.min, Vec2::new(w, 2.0)),
                    0.0,
                    c.accent,
                );
            }
            clip.line_segment(
                [tr.right_top(), tr.right_bottom()],
                Stroke::new(1.0, c.border),
            );
            let name = b.name();
            clip.text(
                Pos2::new(tr.left() + 14.0, tr.center().y),
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
        }
        if let Some((i, close)) = click {
            self.app.cur = i;
            self.app.focus = Focus::Editor;
            if close {
                self.app.run(crate::commands::Command::CloseBuffer);
            }
        }
    }

    // ================================================================= editor

    /// Returns the cursor's screen position (for popups).
    fn draw_editor(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        c: &Palette,
    ) -> Option<Pos2> {
        if self.app.buffers.is_empty() {
            self.draw_welcome(p, r, c);
            return None;
        }
        let (cw, lh, rh) = self.metrics(ui);
        let font = self.mono();
        let visible = ((r.height() / lh).ceil() as usize).max(1);
        self.app.page = ((r.height() / lh).floor() as usize).max(1);
        let focused = self.app.focus == Focus::Editor && self.app.modal.is_none();
        let search = self.app.search.clone().filter(|s| !s.is_empty() && matches!(&self.app.modal, Some(Modal::Prompt(p)) if matches!(p.kind, PromptKind::Find { .. })));
        let cur = self.app.cur;
        let path = self.app.buffers[cur].path.clone();
        let empty = Vec::new();
        let diags = path
            .as_ref()
            .and_then(|p| self.app.diagnostics.get(p))
            .unwrap_or(&empty);
        let b: &mut Buffer = &mut self.app.buffers[cur];

        let digits = b.lines.len().to_string().len().max(3) as f32;
        let gutter_w = (digits + 3.0) * cw;
        let text_left = r.left() + gutter_w;
        let text_cols = (((r.width() - gutter_w) / cw).floor() as usize).max(1);

        // Mouse: click, drag, wheel.
        let resp = ui.interact(r, Id::unique("editor"), Sense::click_and_drag());
        let to_pos = |b: &Buffer, pt: Pos2| -> Pos {
            let line = (b.scroll_y as f32 + ((pt.y - r.top()) / lh).floor().max(0.0)) as usize;
            let line = line.min(b.lines.len() - 1);
            let target = ((pt.x - text_left) / cw + b.scroll_x as f32).max(0.0);
            let mut dcol = 0.0;
            let mut col = 0;
            for ch in b.lines[line].chars() {
                let w = char_w(ch, dcol as usize) as f32;
                if dcol + w / 2.0 > target {
                    break;
                }
                dcol += w;
                col += 1;
            }
            Pos::new(line, col)
        };
        let mut click: Option<(Pos, u32, bool)> = None;
        let mut drag: Option<Pos> = None;
        if let Some(pt) = resp.interact_pointer_pos() {
            let shift = ui.ctx().input(|i| i.modifiers.shift);
            if resp.triple_clicked() {
                click = Some((to_pos(b, pt), 3, false));
            } else if resp.double_clicked() {
                click = Some((to_pos(b, pt), 2, false));
            } else if resp.drag_started() || resp.clicked() {
                click = Some((to_pos(b, pt), 1, shift));
            } else if resp.dragged() {
                drag = Some(to_pos(b, pt));
            }
        }
        if resp.hovered() {
            let d = ui.ctx().input(|i| i.smooth_scroll_delta);
            if d.y != 0.0 {
                self.scroll_acc -= d.y / lh;
                let n = self.scroll_acc.trunc();
                self.scroll_acc -= n;
                let max = b.lines.len().saturating_sub(1) as f32;
                b.scroll_y = (b.scroll_y as f32 + n).clamp(0.0, max) as usize;
            }
            if d.x != 0.0 {
                b.scroll_x = (b.scroll_x as f32 - d.x / cw).max(0.0) as usize;
            }
        }

        // Keep the cursor visible when it moved (not when the user scrolled).
        let key = (b.id, b.cursor, b.version);
        let cur_dcol = display_col(&b.lines[b.cursor.line], b.cursor.col);
        if self.last_cursor != Some(key) {
            let margin = 2.min(self.app.page / 3);
            if b.cursor.line < b.scroll_y + margin {
                b.scroll_y = b.cursor.line.saturating_sub(margin);
            } else if b.cursor.line + margin >= b.scroll_y + self.app.page {
                b.scroll_y = (b.cursor.line + margin + 1).saturating_sub(self.app.page);
            }
            if cur_dcol < b.scroll_x {
                b.scroll_x = cur_dcol.saturating_sub(4);
            } else if cur_dcol + 2 >= b.scroll_x + text_cols {
                b.scroll_x = cur_dcol + 4 - text_cols.min(cur_dcol + 4);
            }
            self.last_cursor = Some(key);
        }
        b.scroll_y = b.scroll_y.min(b.lines.len().saturating_sub(1));

        let deadline = Instant::now() + Duration::from_millis(12);
        let hl_done =
            b.hl.ensure(&self.app.syn, &b.lines, b.scroll_y + visible, deadline);

        let sel = b.selection();
        let clip = p.with_clip_rect(r);
        let text_clip = p.with_clip_rect(Rect::from_min_max(
            Pos2::new(text_left - 2.0, r.top()),
            r.max,
        ));
        let x0 = text_left - b.scroll_x as f32 * cw;
        for row in 0..visible {
            let ln = b.scroll_y + row;
            if ln >= b.lines.len() {
                break;
            }
            let y = r.top() + row as f32 * lh;
            let line = &b.lines[ln];
            let lr = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), lh));
            if ln == b.cursor.line && sel.is_none() {
                clip.rect_filled(lr, 0.0, c.line);
            }
            // Gutter.
            let worst = diags
                .iter()
                .filter(|d| d.start.line <= ln && d.end.line >= ln)
                .map(|d| d.severity)
                .min();
            if let Some(s) = worst {
                let col = match s {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                clip.circle_filled(Pos2::new(r.left() + cw * 0.8, y + lh / 2.0), 3.0, col);
            }
            clip.text(
                Pos2::new(text_left - cw * 1.5, y + lh / 2.0),
                Align2::RIGHT_CENTER,
                (ln + 1).to_string(),
                font.clone(),
                if ln == b.cursor.line { c.fg } else { c.gutter },
            );
            // Selection.
            if let Some((a, z)) = sel
                && a.line <= ln
                && z.line >= ln
            {
                let s = if a.line == ln {
                    display_col(line, a.col)
                } else {
                    0
                };
                let mut e = if z.line == ln {
                    display_col(line, z.col)
                } else {
                    display_col(line, usize::MAX) + 1
                };
                if e == s && z.line != ln {
                    e = s + 1;
                }
                text_clip.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(x0 + s as f32 * cw, y),
                        Pos2::new(x0 + e as f32 * cw, y + lh),
                    ),
                    2.0,
                    c.sel,
                );
            }
            // Search matches.
            if let Some(q) = &search {
                for (s, e) in Buffer::matches_in_line(line, q) {
                    let sc = display_col(line, line[..s].chars().count());
                    let ec = display_col(line, line[..e].chars().count());
                    text_clip.rect_stroke(
                        Rect::from_min_max(
                            Pos2::new(x0 + sc as f32 * cw, y + 1.0),
                            Pos2::new(x0 + ec as f32 * cw, y + lh - 1.0),
                        ),
                        2.0,
                        Stroke::new(1.0, c.accent),
                        StrokeKind::Inside,
                    );
                }
            }
            // Text.
            let spans = b.hl.lines.get(ln);
            let job = line_job(line, spans, &font, c.fg, b.scroll_x, text_cols + 2);
            let galley = p.layout_job(job);
            let skipped = skipped_cols(line, b.scroll_x);
            text_clip.galley(
                Pos2::new(text_left + skipped as f32 * cw, y + (lh - rh) / 2.0),
                galley,
                c.fg,
            );
            // Diagnostics underline (squiggle).
            for d in diags
                .iter()
                .filter(|d| d.start.line <= ln && d.end.line >= ln)
            {
                let s = if d.start.line == ln {
                    display_col(line, d.start.col)
                } else {
                    0
                };
                let mut e = if d.end.line == ln {
                    display_col(line, d.end.col)
                } else {
                    display_col(line, usize::MAX)
                };
                if e <= s {
                    e = s + 1;
                }
                let col = match d.severity {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                squiggle(
                    &text_clip,
                    x0 + s as f32 * cw,
                    x0 + e as f32 * cw,
                    y + lh - 2.5,
                    col,
                );
            }
        }
        // Cursor.
        let cx = x0 + cur_dcol as f32 * cw;
        let cy = r.top() + (b.cursor.line as f32 - b.scroll_y as f32) * lh;
        let cursor_px = (cy >= r.top() && cy < r.bottom()).then_some(Pos2::new(cx, cy));
        if focused && cursor_px.is_some() {
            text_clip.rect_filled(
                Rect::from_min_size(Pos2::new(cx - 1.0, cy + 2.0), Vec2::new(2.0, lh - 4.0)),
                0.0,
                c.accent,
            );
        }
        if b.scroll_y > 0 {
            // Subtle shadow under the tab bar when scrolled, like Sublime.
            for k in 0..6 {
                let a = 40 - k * 6;
                clip.line_segment(
                    [
                        Pos2::new(r.left(), r.top() + k as f32),
                        Pos2::new(r.right(), r.top() + k as f32),
                    ],
                    Stroke::new(1.0, Color32::from_black_alpha(a as u8)),
                );
            }
        }
        self.app.hl_pending = !hl_done;
        if let Some((pos, n, ext)) = click {
            self.app.editor_click(pos, n, ext);
        } else if let Some(pos) = drag {
            self.app.editor_drag(pos);
        }
        cursor_px
    }

    fn draw_welcome(&self, p: &Painter, r: Rect, c: &Palette) {
        let lines = [
            ("Goto Anything", "Cmd+P"),
            ("Command Palette", "Cmd+Shift+P"),
            ("Focus Sidebar", "Cmd+0"),
            ("Toggle Sidebar", "Cmd+B"),
            ("New File", "Cmd+N"),
            ("Open Folder", "Cmd+O"),
            ("Settings & Languages", "Cmd+,"),
        ];
        let font = FontId::proportional(15.0);
        let top = r.center().y - lines.len() as f32 * 15.0;
        for (i, (a, k)) in lines.iter().enumerate() {
            let y = top + i as f32 * 30.0;
            p.text(
                Pos2::new(r.center().x - 12.0, y),
                Align2::RIGHT_CENTER,
                *a,
                font.clone(),
                c.dim,
            );
            p.text(
                Pos2::new(r.center().x + 12.0, y),
                Align2::LEFT_CENTER,
                *k,
                font.clone(),
                c.fg,
            );
        }
    }

    // ================================================================= status bar

    fn draw_status(&self, p: &Painter, r: Rect, c: &Palette) {
        p.rect_filled(r, 0.0, c.chrome);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, c.border));
        let font = FontId::proportional(12.0);
        let y = r.center().y;
        let mut left = String::new();
        let app = &self.app;
        if let Some(b) = app.buf() {
            left = format!("Line {}, Column {}", b.cursor.line + 1, b.cursor.col + 1);
            if let Some((a, z)) = b.selection() {
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
        let msg = app.status.as_ref().map(|s| s.0.clone()).or_else(|| {
            let b = app.buf()?;
            let ds = app.diagnostics.get(b.path.as_ref()?)?;
            ds.iter()
                .filter(|d| d.start.line <= b.cursor.line && d.end.line >= b.cursor.line)
                .min_by_key(|d| d.severity)
                .map(|d| d.message.lines().next().unwrap_or("").to_string())
        });
        let mut right = vec![];
        if let Some(b) = app.buf() {
            let lsp = app.lsp_status();
            if !lsp.is_empty() {
                right.push(lsp);
            }
            right.push(if b.indent_unit == "\t" {
                "Tab Size: 4".into()
            } else {
                format!("Spaces: {}", b.indent_unit.len())
            });
            right.push(b.hl.syntax.clone());
        }
        let rtext = right.join("     ");
        let rr = p.text(
            Pos2::new(r.right() - 12.0, y),
            Align2::RIGHT_CENTER,
            &rtext,
            font.clone(),
            c.dim,
        );
        if let Some(m) = msg {
            let clip = p.with_clip_rect(Rect::from_min_max(
                Pos2::new(x.right() + 24.0, r.top()),
                Pos2::new(rr.left() - 24.0, r.bottom()),
            ));
            clip.text(
                Pos2::new(x.right() + 24.0, y),
                Align2::LEFT_CENTER,
                m,
                font,
                c.accent,
            );
        }
    }

    // ================================================================= overlays

    fn draw_prompt(&self, p: &Painter, r: Rect, c: &Palette) {
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
        if matches!(pr.kind, PromptKind::Find { .. }) {
            extra = match &self.find_count {
                Some((_, _, n)) if !pr.input.is_empty() => {
                    format!("{n} matches    Enter next · Shift+Enter previous · Esc close")
                }
                _ => "Enter next · Shift+Enter previous · Esc close".into(),
            };
        } else if pr.kind_is_path() {
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
        let font = FontId::monospace(UI_SIZE);
        let clip = p.with_clip_rect(field.shrink(2.0));
        let tx = clip.text(
            Pos2::new(field.left() + 8.0, field.center().y),
            Align2::LEFT_CENTER,
            &pr.input,
            font.clone(),
            c.fg,
        );
        let before: String = pr.input.chars().take(pr.cursor).collect();
        let bw = p
            .ctx()
            .fonts_mut(|f| f.layout_no_wrap(before, font.clone(), c.fg).size().x);
        let cx = tx.left() + bw;
        clip.rect_filled(
            Rect::from_min_size(
                Pos2::new(cx, field.top() + 5.0),
                Vec2::new(2.0, field.height() - 10.0),
            ),
            0.0,
            c.accent,
        );
    }

    fn draw_palette(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        let Some(Modal::Palette(pal)) = &self.app.modal else {
            return;
        };
        let w = (full.width() * 0.6).clamp(360.0, 720.0);
        let max_rows = 12usize;
        let n = pal.shown.len().min(max_rows);
        let line_mode = pal.mode == Mode::Line;
        let rows = if line_mode { 1 } else { n.max(1) };
        let h = 52.0 + rows as f32 * 30.0;
        let r = Rect::from_min_size(
            Pos2::new(full.center().x - w / 2.0, full.top() + 40.0),
            Vec2::new(w, h),
        );
        p.rect_filled(
            r.expand(1.0).translate(Vec2::new(0.0, 4.0)),
            8.0,
            Color32::from_black_alpha(90),
        );
        p.rect_filled(r, 6.0, c.pop);
        p.rect_stroke(r, 6.0, Stroke::new(1.0, c.border), StrokeKind::Inside);
        let input = Rect::from_min_size(r.min + Vec2::new(10.0, 10.0), Vec2::new(w - 20.0, 32.0));
        p.rect_filled(input, 4.0, c.bg);
        let font = FontId::proportional(14.0);
        let tr = p.text(
            Pos2::new(input.left() + 10.0, input.center().y),
            Align2::LEFT_CENTER,
            &pal.query,
            font.clone(),
            c.fg,
        );
        let cx = if pal.query.is_empty() {
            input.left() + 10.0
        } else {
            tr.right() + 1.0
        };
        p.rect_filled(
            Rect::from_min_size(Pos2::new(cx, input.top() + 7.0), Vec2::new(2.0, 18.0)),
            0.0,
            c.accent,
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
                Pos2::new(r.left() + 6.0, top + row as f32 * 30.0),
                Vec2::new(w - 12.0, 30.0),
            );
            if i == pal.sel {
                p.rect_filled(rr, 4.0, c.pop_sel);
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

    fn draw_menu_bar(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
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
            if resp.clicked() {
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

    fn draw_menu_dropdown(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
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

    fn draw_keymap(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        let Some(Modal::Keymap(v)) = &self.app.modal else {
            return;
        };
        let rows = crate::app::keymap_rows(&self.app.keymap, &v.query);
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
        let tr = p.text(
            Pos2::new(input.left() + 10.0, input.center().y),
            Align2::LEFT_CENTER,
            if v.query.is_empty() {
                "Search shortcuts or actions…"
            } else {
                &v.query
            },
            FontId::proportional(14.0),
            if v.query.is_empty() { c.dim } else { c.fg },
        );
        let cx = if v.query.is_empty() {
            input.left() + 10.0
        } else {
            tr.right() + 1.0
        };
        p.rect_filled(
            Rect::from_min_size(Pos2::new(cx, input.top() + 6.0), Vec2::new(2.0, 18.0)),
            0.0,
            c.accent,
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

    fn draw_servers(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        use crate::app::ServerStatus as St;
        let Some(Modal::Servers(v)) = &self.app.modal else {
            return;
        };
        let rows = self.app.server_rows();
        let sel = v.sel.min(rows.len().saturating_sub(1));
        let details = v.details;
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
                St::Missing if srv.install.is_some() => (c.dim, "not installed".to_string()),
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

    fn draw_confirm(&self, p: &Painter, full: Rect, c: &Palette) {
        let Some(Modal::Confirm(cf)) = &self.app.modal else {
            return;
        };
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

    fn draw_completion(
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
        let first = comp.sel.saturating_sub(rows - 1);
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
            if i == comp.sel {
                clip.rect_filled(rr, 3.0, c.pop_sel);
            }
            clip.text(
                Pos2::new(rr.left() + 6.0, rr.center().y),
                Align2::LEFT_CENTER,
                it.kind,
                FontId::proportional(11.0),
                c.info,
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
                comp.sel = i;
            }
            self.app.on_key(Key::new(crate::keys::KeyCode::Enter));
        }
    }

    fn draw_hover(&self, p: &Painter, editor: Rect, at: Pos2, c: &Palette) {
        let Some(hv) = &self.app.hover else { return };
        let text = hv.lines.join("\n");
        let font = FontId::monospace(12.5);
        let max_w = (editor.width() - 40.0).min(640.0);
        let g = p.ctx().fonts_mut(|f| f.layout(text, font, c.fg, max_w));
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
    }
}

impl eframe::App for Gui {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn on_exit(&mut self) {
        self.app.lsp.shutdown();
    }
}

// ===================================================================== text helpers

fn char_w(c: char, at: usize) -> usize {
    if c == '\t' {
        TAB - at % TAB
    } else {
        c.width().unwrap_or(1).max(1)
    }
}

pub fn display_col(line: &str, col: usize) -> usize {
    let mut w = 0;
    for c in line.chars().take(col) {
        w += char_w(c, w);
    }
    w
}

/// Display columns fully hidden by horizontal scroll (where the visible text starts).
fn skipped_cols(line: &str, scroll_x: usize) -> usize {
    let mut w = 0;
    for c in line.chars() {
        let cw = char_w(c, w);
        if w + cw > scroll_x {
            break;
        }
        w += cw;
    }
    w.saturating_sub(scroll_x)
}

/// Build the visible part of a line as a layout job with syntax colors; tabs expand to spaces.
fn line_job(
    line: &str,
    spans: Option<&Vec<Span>>,
    font: &FontId,
    fg: Color32,
    scroll_x: usize,
    cols: usize,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut dcol = 0;
    let mut text = String::new();
    let mut cur: Option<(Color32, bool, bool)> = None;
    let mut si = 0;
    let flush = |job: &mut LayoutJob, text: &mut String, st: Option<(Color32, bool, bool)>| {
        if let Some((col, italic, underline)) = st
            && !text.is_empty()
        {
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: col,
                    italics: italic,
                    underline: if underline {
                        Stroke::new(1.0, col)
                    } else {
                        Stroke::NONE
                    },
                    ..Default::default()
                },
            );
            text.clear();
        }
    };
    for (bi, ch) in line.char_indices() {
        let w = char_w(ch, dcol);
        let start = dcol;
        dcol += w;
        if start < scroll_x {
            continue;
        }
        if start > scroll_x + cols {
            break;
        }
        let mut st = (fg, false, false);
        if let Some(sp) = spans {
            while si < sp.len() && sp[si].1 <= bi {
                si += 1;
            }
            if let Some(&(s, e, hs)) = sp.get(si)
                && s <= bi
                && bi < e
            {
                st = (hs.fg, hs.italic, hs.underline);
            }
        }
        if cur != Some(st) {
            flush(&mut job, &mut text, cur);
            cur = Some(st);
        }
        if ch == '\t' {
            text.push_str(&" ".repeat(w));
        } else if ch.is_control() {
            text.push('?');
        } else {
            text.push(ch);
        }
    }
    flush(&mut job, &mut text, cur);
    job
}

/// Folder disclosure triangle (drawn, so it never depends on font coverage).
/// Folder disclosure triangle, rotated from ▸ (0.0) to ▾ (1.0). Drawn as a shape so it
/// never depends on font coverage.
fn disclosure(p: &Painter, at: Pos2, open: f32, col: Color32) {
    let a = open.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    let (sn, cs) = a.sin_cos();
    let rot = |v: Vec2| at + Vec2::new(v.x * cs - v.y * sn, v.x * sn + v.y * cs);
    let pts = vec![
        rot(Vec2::new(-2.0, -4.0)),
        rot(Vec2::new(3.0, 0.0)),
        rot(Vec2::new(-2.0, 4.0)),
    ];
    p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
}

fn squiggle(p: &Painter, x0: f32, x1: f32, y: f32, col: Color32) {
    let mut pts = vec![];
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        pts.push(Pos2::new(x, if up { y - 1.5 } else { y + 1.0 }));
        x += 2.5;
        up = !up;
    }
    if pts.len() >= 2 {
        p.add(egui::Shape::line(pts, Stroke::new(1.0, col)));
    }
}
