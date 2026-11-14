//! The Sublime-style GUI: everything is custom-painted with egui's painter so the editor,
//! sidebar, tabs and overlays look and behave like a code editor rather than a form.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, FontId, Id, LayerId, Order, Painter, Pos2, Rect, Sense, Stroke,
    StrokeKind, Vec2, text::LayoutJob,
};
use unicode_width::UnicodeWidthChar;

use crate::app::{App, AppEvent, EventTx, Focus, Modal, PromptKind};
use crate::config::{CaretStyle, Config};
use crate::editor::{Buffer, Pos};
use crate::highlight::Span;
use crate::keys::Key;
use crate::palette::Mode;

thread_local! {
    /// Tab display width of the buffer being drawn (set per frame from the buffer).
    static TAB: std::cell::Cell<usize> = const { std::cell::Cell::new(4) };
}
const UI_SIZE: f32 = 13.0;
const TAB_H: f32 = 32.0;
const SCROLLBAR_W: f32 = 12.0;
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
    term_scroll_acc: f32,
    last_cursor: Option<(usize, Pos, i32)>,
    /// Caret position/version and when it last changed: the blink phase restarts there so
    /// the caret is solid while typing or moving.
    caret_mark: (Option<(usize, Pos, i32)>, Instant),
    /// Where on the scrollbar thumb the drag started (pointer offset from the thumb's
    /// start), for the editor's vertical / horizontal scrollbars.
    vscroll_grab: Option<f32>,
    hscroll_grab: Option<f32>,
    /// Decoded icon-package images (None: failed to load, don't retry every frame).
    icon_tex: IconTextures,
    /// The focused group's editor rect (tests, popups).
    editor_rect: Rect,
    /// (char width, text left edge, first line top, line height) of the last frame.
    text_geom: (f32, f32, f32, f32),
    last_tree_sel: Option<usize>,
    last_title: String,
    find_count: Option<(String, i32, usize)>,
    pub menu_titles: Vec<Rect>,
    /// Clickable status-bar items of the last frame.
    pub status_hits: Vec<(crate::commands::Command, Rect)>,
    /// Screen rect of the hover popup last frame (the pointer may move onto it).
    hover_rect: Option<Rect>,
    /// Screen position of a mouse-hover popup's anchor (None = at the cursor).
    hover_anchor_px: Option<Pos2>,
    /// Where the keyboard-opened context menu should appear (cursor / selected row).
    pub keyboard_spot: Option<Pos2>,
    sidebar_sel_px: Option<Pos2>,
    #[cfg(target_os = "macos")]
    native_menu: Option<crate::native_menu::NativeMenu>,
}

impl Gui {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        project: crate::project::Project,
        config: Config,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        let mut g = Self::with_project(project, config);
        let tx = g.app_tx();
        let ctx2 = ctx.clone();
        tx.set_waker(move || ctx.request_repaint());
        crate::log::set_waker(move || ctx2.request_repaint());
        if let Some(d) = &g.app.config.data_dir {
            crate::log::set_file(&d.join("logs").join("farsub.log"));
        }
        crate::log::info(
            "app",
            format!(
                "farsub {} — {}",
                env!("CARGO_PKG_VERSION"),
                g.app.project.name()
            ),
        );
        #[cfg(target_os = "macos")]
        {
            g.native_menu = crate::native_menu::NativeMenu::install(&g.app, tx.clone());
        }
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        g.last_title.clear();
        g
    }

    /// Construct without a window (tests drive it through egui_kittest).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn headless(root: PathBuf, config: Config) -> Self {
        Self::with_project(crate::project::Project::from_folders(vec![root]), config)
    }

    pub fn with_project(project: crate::project::Project, config: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        let app = App::new(project, config, EventTx::new(tx));
        Gui {
            app,
            rx,
            suppress_text: false,
            scroll_acc: 0.0,
            term_scroll_acc: 0.0,
            last_cursor: None,
            caret_mark: (None, Instant::now()),
            vscroll_grab: None,
            hscroll_grab: None,
            icon_tex: HashMap::new(),
            editor_rect: Rect::NOTHING,
            text_geom: (0.0, 0.0, 0.0, 0.0),
            last_tree_sel: None,
            last_title: String::new(),
            find_count: None,
            menu_titles: vec![],
            status_hits: vec![],
            hover_rect: None,
            hover_anchor_px: None,
            keyboard_spot: None,
            sidebar_sel_px: None,
            #[cfg(target_os = "macos")]
            native_menu: None,
        }
    }

    #[cfg(test)]
    /// (char width, text left edge, y of the first visible line's middle).
    #[cfg(test)]
    pub fn text_metrics_for_test(&self) -> (f32, f32, f32) {
        let (cw, left, top, lh) = self.text_geom;
        (cw, left, top + lh / 2.0)
    }

    #[cfg(test)]
    /// Screen point of a buffer position's character cell middle (tests drive the mouse).
    #[cfg(test)]
    /// Icon images loaded so far: (path, decoded ok).
    pub fn icon_textures_for_test(&self) -> Vec<(PathBuf, bool)> {
        self.icon_tex
            .iter()
            .map(|(p, t)| (p.clone(), t.is_some()))
            .collect()
    }

    #[cfg(test)]
    /// The focused group's editor area last frame.
    pub fn editor_rect_for_test(&self) -> Rect {
        self.editor_rect
    }

    #[cfg(test)]
    pub fn screen_pos_for_test(&self, line: usize, col: usize) -> Pos2 {
        let (cw, left, top, lh) = self.text_geom;
        let b = self.app.buf().unwrap();
        let x = left + (display_col(&b.lines[line], col) as f32 - b.scroll_x as f32 + 0.5) * cw;
        Pos2::new(x, top + (line as f32 - b.scroll_y as f32 + 0.5) * lh)
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
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    self.suppress_text = false;
                    // Shift+digit arrives as its symbol ("!" for 1); shortcuts like
                    // Ctrl+Shift+1 mean the digit key, so use its physical position.
                    let key = match physical_key {
                        Some(pk) if modifiers.shift && is_digit_key(pk) => pk,
                        _ => key,
                    };
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
        // Every widget is custom-drawn and keys are routed by the app, so egui's own widget
        // focus must never hold: otherwise Tab/Ctrl+Shift+Tab walk it onto a menu-bar button
        // and the next Enter "clicks" it.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        // Whole-UI zoom from the config (egui scales every font and widget together).
        let scale = self.app.config.editor.ui_scale;
        let scale = if scale.is_finite() {
            scale.clamp(0.5, 3.0)
        } else {
            1.0
        };
        if (ctx.zoom_factor() - scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(scale);
        }
        self.pump();
        self.handle_input(&ctx);
        self.pump();
        self.app.reap_terminals();

        if ctx.input(|i| i.viewport().close_requested()) && !self.app.quit {
            if self.app.config.editor.hot_exit {
                // Unsaved work goes into the workspace; it comes back next launch.
                self.app.save_workspace();
            } else if self.app.any_modified() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.app.run(crate::commands::Command::Quit);
            }
        }
        self.app.autosave_tick();
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
            let max_w = (full.width() * 0.7).max(160.0);
            self.app.sidebar_width = self.app.sidebar_width.clamp(120.0, max_w);
            let side = Rect::from_min_max(
                rest.min,
                Pos2::new(rest.left() + self.app.sidebar_width, rest.bottom()),
            );
            rest.min.x = side.right();
            self.draw_sidebar(ui, &painter, side, &c);
            // Drag the sidebar's edge to resize it (double-click resets).
            let handle = Rect::from_min_max(
                Pos2::new(side.right() - 3.0, side.top()),
                Pos2::new(side.right() + 3.0, side.bottom()),
            );
            let h = ui.interact(
                handle,
                Id::unique("sidebar-resize"),
                Sense::click_and_drag(),
            );
            if h.hovered() || h.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                painter.line_segment(
                    [side.right_top(), side.right_bottom()],
                    Stroke::new(2.0, c.accent),
                );
            }
            if h.dragged() {
                self.app.sidebar_width =
                    (self.app.sidebar_width + h.drag_delta().x).clamp(120.0, max_w);
            }
            if h.double_clicked() {
                self.app.sidebar_width = SIDEBAR_W;
            }
        }
        // The bottom panel (terminals, console) sits under the editor column.
        let bottom = self.app.panel.visible.then(|| {
            let h = self
                .app
                .panel
                .height
                .clamp(80.0, (rest.height() - TAB_H - 60.0).max(80.0));
            let r = Rect::from_min_max(Pos2::new(rest.left(), rest.bottom() - h), rest.max);
            rest.max.y = r.top();
            r
        });
        // Editor groups side by side (Sublime's Layout → Columns), each with its own tabs.
        let n = self.app.layout.max(1);
        if self.app.group_split.len() != n {
            self.app.group_split = vec![1.0 / n as f32; n];
        }
        self.app.hl_pending = false;
        let mut editor = rest;
        let mut cursor_px = None;
        let mut x = rest.left();
        for g in 0..n {
            let w = if g + 1 == n {
                rest.right() - x
            } else {
                (rest.width() * self.app.group_split[g]).round()
            };
            let col = Rect::from_min_max(Pos2::new(x, rest.top()), Pos2::new(x + w, rest.bottom()));
            x += w;
            let tabs = Rect::from_min_max(col.min, Pos2::new(col.right(), col.top() + TAB_H));
            let ed = Rect::from_min_max(Pos2::new(col.left(), tabs.bottom()), col.max);
            self.draw_tabs(ui, &painter, tabs, &c, g);
            let px = self.draw_editor(ui, &painter, ed, &c, g);
            if g == self.app.group {
                editor = ed;
                cursor_px = px;
                self.editor_rect = ed;
            }
            if g + 1 < n {
                // Divider between groups; drag it to resize the columns.
                painter.line_segment(
                    [
                        Pos2::new(col.right(), col.top()),
                        Pos2::new(col.right(), col.bottom()),
                    ],
                    Stroke::new(1.0, c.border),
                );
                let handle = Rect::from_min_max(
                    Pos2::new(col.right() - 3.0, col.top()),
                    Pos2::new(col.right() + 3.0, col.bottom()),
                );
                let h = ui.interact(
                    handle,
                    Id::unique(("group-divider", g)),
                    Sense::click_and_drag(),
                );
                if h.hovered() || h.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if h.dragged() && rest.width() > 0.0 {
                    let d = h.drag_delta().x / rest.width();
                    let min = 120.0 / rest.width();
                    let (a, b) = (self.app.group_split[g], self.app.group_split[g + 1]);
                    let na = (a + d).clamp(min, a + b - min);
                    self.app.group_split[g] = na;
                    self.app.group_split[g + 1] = a + b - na;
                }
                if h.double_clicked() {
                    self.app.group_split = vec![1.0 / n as f32; n];
                }
            }
        }
        self.draw_status(ui, &painter, status, &c);
        if let Some(r) = panel {
            self.draw_prompt(&painter, r, &c);
        }
        if let Some(r) = bottom {
            self.draw_bottom_panel(ui, &painter, r, &c);
        }

        let over = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("overlay")));
        if let Some(p) = cursor_px {
            self.draw_completion(ui, &over, editor, p, &c);
        }
        self.hover_rect = None;
        if let Some(p) = self.hover_anchor_px.or(cursor_px) {
            self.draw_hover(&over, editor, p, &c);
        }
        if self.app.context_menu.is_some() {
            self.draw_context_menu(ui, &over, full, &c);
        }
        match &self.app.modal {
            Some(Modal::Palette(_)) => self.draw_palette(ui, &over, full, &c),
            Some(Modal::Confirm(_)) => self.draw_confirm(&over, full, &c),
            Some(Modal::Keymap(_)) => self.draw_keymap(ui, &over, full, &c),
            Some(Modal::Servers(_)) => self.draw_servers(ui, &over, full, &c),
            Some(Modal::LspInfo(_)) => self.draw_lsp_info(ui, &over, full, &c),
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
        let rootname = self.app.project.name();
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

    // ================================================================= tabs

    /// The buffer a group shows this frame.
    fn shown_in(&self, g: usize) -> Option<usize> {
        if g == self.app.group {
            self.app.buf().map(|_| self.app.cur)
        } else {
            self.app.group_shown(g)
        }
    }

    fn draw_tabs(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette, g: usize) {
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

    // ================================================================= editor

    /// Returns the cursor's screen position (for popups).
    fn draw_editor(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        c: &Palette,
        g: usize,
    ) -> Option<Pos2> {
        let active_group = g == self.app.group;
        let Some(cur) = self.shown_in(g) else {
            if self.app.buffers.is_empty() && g == 0 {
                self.draw_welcome(p, r, c);
            }
            // An empty group still takes focus when clicked.
            let resp = ui.interact(r, Id::unique(("editor-pane", g)), Sense::click());
            if resp.clicked() || resp.secondary_clicked() {
                self.app.focus_group(g);
            }
            return None;
        };
        let (cw, lh, rh) = self.metrics(ui);
        let font = self.mono();
        let visible = ((r.height() / lh).ceil() as usize).max(1);
        if active_group {
            self.app.page = ((r.height() / lh).floor() as usize).max(1);
        }
        let focused = active_group && self.app.focus == Focus::Editor && self.app.modal.is_none();
        let search = self.app.search.clone().filter(|s| !s.is_empty() && active_group && matches!(&self.app.modal, Some(Modal::Prompt(p)) if matches!(p.kind, PromptKind::Find { .. })));
        let path = self.app.buffers[cur].path.clone();
        let empty = Vec::new();
        let diags = path
            .as_ref()
            .and_then(|p| self.app.diagnostics.get(p))
            .unwrap_or(&empty);
        let b: &mut Buffer = &mut self.app.buffers[cur];
        TAB.with(|t| t.set(b.tab_size));
        let guides = self.app.config.editor.indent_guides;
        let line_highlight = self.app.config.editor.highlight_current_line;
        let guide_col = self.app.syn.guide().unwrap_or(mix(c.bg, c.fg, 0.12));
        let active_guide_col = self.app.syn.active_guide().unwrap_or(mix(c.bg, c.fg, 0.32));

        let digits = b.lines.len().to_string().len().max(3) as f32;
        let gutter_w = (digits + 3.0) * cw;
        let text_left = r.left() + gutter_w;
        if active_group {
            self.text_geom = (cw, text_left, r.top(), lh);
        }
        // Leave room for the vertical scrollbar on the right.
        let text_cols = (((r.width() - gutter_w - SCROLLBAR_W) / cw).floor() as usize).max(1);

        // Mouse: click, drag, wheel.
        let resp = ui.interact(r, Id::unique(("editor-pane", g)), Sense::click_and_drag());
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
        // Pointer: I-beam over text, but the normal arrow over selected text (like Sublime).
        // Uses the character cell under the pointer, not the nearest caret position.
        let over_selection = |b: &Buffer, pt: Pos2| -> bool {
            let Some((a, z)) = b.selection() else {
                return false;
            };
            let line = b.scroll_y + ((pt.y - r.top()) / lh).floor().max(0.0) as usize;
            if line >= b.lines.len() {
                return false;
            }
            let target = (pt.x - text_left) / cw + b.scroll_x as f32;
            let mut dcol = 0.0;
            for (col, ch) in b.lines[line].chars().enumerate() {
                let w = char_w(ch, dcol as usize) as f32;
                if target >= dcol && target < dcol + w {
                    let p = Pos::new(line, col);
                    return a <= p && p < z;
                }
                dcol += w;
            }
            false
        };
        // The character cell under the pointer, for mouse hover (LSP info / diagnostics).
        let char_under = |b: &Buffer, pt: Pos2| -> Option<Pos> {
            let line = b.scroll_y + ((pt.y - r.top()) / lh).floor().max(0.0) as usize;
            if line >= b.lines.len() || pt.x < text_left {
                return None;
            }
            let target = (pt.x - text_left) / cw + b.scroll_x as f32;
            let mut dcol = 0.0;
            for (col, ch) in b.lines[line].chars().enumerate() {
                let w = char_w(ch, dcol as usize) as f32;
                if target >= dcol && target < dcol + w {
                    return (!ch.is_whitespace()).then_some(Pos::new(line, col));
                }
                dcol += w;
            }
            None
        };
        let hover_at = if !active_group || resp.dragged() || resp.is_pointer_button_down_on() {
            None
        } else {
            resp.hover_pos().and_then(|hp| char_under(b, hp))
        };
        let mut right_click: Option<(Pos, Pos2, bool)> = None;
        if resp.secondary_clicked()
            && let Some(pt) = resp.interact_pointer_pos()
        {
            right_click = Some((to_pos(b, pt), pt, over_selection(b, pt)));
        }
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        } else if let Some(hp) = resp.hover_pos().filter(|hp| hp.x >= text_left - 2.0) {
            let icon = if over_selection(b, hp) {
                egui::CursorIcon::Default
            } else {
                egui::CursorIcon::Text
            };
            ui.ctx().set_cursor_icon(icon);
        }
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
        if resp.contains_pointer() {
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
        if active_group && self.last_cursor != Some(key) {
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
        // Indent guides: one line per indentation level; the guide of the block around the
        // cursor is highlighted (Sublime's "active" guide).
        let unit = if b.indent_unit == "\t" {
            b.tab_size
        } else {
            b.indent_unit.len()
        }
        .max(1);
        let active = if guides {
            let w = b.indent_width(b.cursor.line);
            // On a block's opening line, highlight the block below it.
            let below = b
                .lines
                .get(b.cursor.line + 1)
                .map(|_| b.indent_width(b.cursor.line + 1))
                .unwrap_or(0);
            let col = if below > w {
                w
            } else if w >= unit {
                w - unit
            } else {
                usize::MAX
            };
            if col == usize::MAX {
                None
            } else {
                let (mut top, mut bot) = (b.cursor.line, b.cursor.line);
                while top > 0 && b.cursor.line - top < 3000 && b.indent_width(top - 1) > col {
                    top -= 1;
                }
                while bot + 1 < b.lines.len()
                    && bot - b.cursor.line < 3000
                    && b.indent_width(bot + 1) > col
                {
                    bot += 1;
                }
                if b.indent_width(b.cursor.line) <= col {
                    top = b.cursor.line + 1;
                }
                Some((col, top, bot))
            }
        } else {
            None
        };
        for row in 0..visible {
            let ln = b.scroll_y + row;
            if ln >= b.lines.len() {
                break;
            }
            let y = r.top() + row as f32 * lh;
            let line = &b.lines[ln];
            let lr = Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(r.width(), lh));
            if line_highlight && ln == b.cursor.line && sel.is_none() {
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
            if guides {
                let w = b.indent_width(ln);
                let mut col = 0;
                while col + unit <= w {
                    let x = x0 + col as f32 * cw + 0.5;
                    let is_active =
                        active.is_some_and(|(ac, t, bt)| ac == col && ln >= t && ln <= bt);
                    text_clip.line_segment(
                        [Pos2::new(x, y), Pos2::new(x, y + lh)],
                        Stroke::new(
                            1.0,
                            if is_active {
                                active_guide_col
                            } else {
                                guide_col
                            },
                        ),
                    );
                    col += unit;
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
            let ed = &self.app.config.editor;
            let mark = Some((b.id, b.cursor, b.version));
            if self.caret_mark.0 != mark {
                self.caret_mark = (mark, Instant::now());
            }
            // Only animate while the window has focus (no idle repaints in the background).
            let animate = ui.ctx().input(|i| i.focused);
            let style = if animate {
                ed.caret_style
            } else {
                CaretStyle::Solid
            };
            let (alpha, next) = caret_alpha(style, self.caret_mark.1.elapsed());
            if let Some(next) = next {
                ui.ctx().request_repaint_after(next);
            }
            let w = 1.0 + ed.caret_extra_width.clamp(0.0, 20.0);
            let top = cy - ed.caret_extra_top.clamp(-lh / 2.0, lh);
            let bottom = cy + lh + ed.caret_extra_bottom.clamp(-lh / 2.0, lh);
            if alpha > 0.0 && bottom > top {
                text_clip.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(cx - w / 2.0, top),
                        Pos2::new(cx + w / 2.0, bottom),
                    ),
                    0.0,
                    c.accent.gamma_multiply(alpha),
                );
            }
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
        // Scrollbars (Sublime-style: thin, wider on hover). Vertical: drag the thumb or click
        // the track to jump there; diagnostics are marked along it.
        let max_scroll = b.lines.len().saturating_sub(1);
        let page = visible.saturating_sub(1).max(1);
        if max_scroll > 0 {
            let track = Rect::from_min_max(Pos2::new(r.right() - SCROLLBAR_W, r.top()), r.max);
            let sresp = ui.interact(track, Id::unique(("vscroll", g)), Sense::click_and_drag());
            let wide = sresp.hovered() || sresp.dragged();
            let total = (max_scroll + page) as f32;
            let thumb_h = (track.height() * page as f32 / total).clamp(24.0, track.height());
            let span = (track.height() - thumb_h).max(1.0);
            let thumb_y = |sy: usize| track.top() + span * sy as f32 / max_scroll as f32;
            for d in diags.iter() {
                let y = track.top() + track.height() * d.start.line as f32 / b.lines.len() as f32;
                let col = match d.severity {
                    1 => c.err,
                    2 => c.warn,
                    _ => c.info,
                };
                clip.rect_filled(
                    Rect::from_min_size(Pos2::new(track.right() - 4.0, y), Vec2::new(3.0, 2.0)),
                    0.0,
                    col,
                );
            }
            if let Some(pt) = sresp.interact_pointer_pos() {
                let at = thumb_y(b.scroll_y);
                if sresp.drag_started()
                    || (sresp.is_pointer_button_down_on() && self.vscroll_grab.is_none())
                {
                    // Grabbing the thumb keeps the grip point; the track jumps there first.
                    let on_thumb = pt.y >= at && pt.y <= at + thumb_h;
                    self.vscroll_grab = Some(if on_thumb { pt.y - at } else { thumb_h / 2.0 });
                }
                let grab = self.vscroll_grab.unwrap_or(thumb_h / 2.0);
                let f = ((pt.y - grab - track.top()) / span).clamp(0.0, 1.0);
                b.scroll_y = (f * max_scroll as f32).round() as usize;
            }
            if !sresp.is_pointer_button_down_on() {
                self.vscroll_grab = None;
            }
            if wide {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            let tw = if wide { 8.0 } else { 5.0 };
            let ty = thumb_y(b.scroll_y);
            clip.rect_filled(
                Rect::from_min_size(
                    Pos2::new(track.right() - tw - 2.0, ty),
                    Vec2::new(tw, thumb_h),
                ),
                tw / 2.0,
                mix(c.bg, c.fg, if wide { 0.35 } else { 0.2 }),
            );
        }
        // Horizontal: only when lines are wider than the pane.
        let widest = b
            .lines
            .iter()
            .map(|l| display_col(l, usize::MAX))
            .max()
            .unwrap_or(0);
        if widest + 2 > text_cols || b.scroll_x > 0 {
            let track = Rect::from_min_max(
                Pos2::new(text_left, r.bottom() - SCROLLBAR_W),
                Pos2::new(r.right() - SCROLLBAR_W, r.bottom()),
            );
            let hresp = ui.interact(track, Id::unique(("hscroll", g)), Sense::click_and_drag());
            let wide = hresp.hovered() || hresp.dragged();
            let max_x = (widest + 4)
                .saturating_sub(text_cols)
                .max(b.scroll_x)
                .max(1);
            let total = (max_x + text_cols) as f32;
            let thumb_w = (track.width() * text_cols as f32 / total).clamp(24.0, track.width());
            let span = (track.width() - thumb_w).max(1.0);
            let thumb_x = |sx: usize| track.left() + span * sx as f32 / max_x as f32;
            if let Some(pt) = hresp.interact_pointer_pos() {
                let at = thumb_x(b.scroll_x);
                if hresp.drag_started()
                    || (hresp.is_pointer_button_down_on() && self.hscroll_grab.is_none())
                {
                    let on_thumb = pt.x >= at && pt.x <= at + thumb_w;
                    self.hscroll_grab = Some(if on_thumb { pt.x - at } else { thumb_w / 2.0 });
                }
                let grab = self.hscroll_grab.unwrap_or(thumb_w / 2.0);
                let f = ((pt.x - grab - track.left()) / span).clamp(0.0, 1.0);
                b.scroll_x = (f * max_x as f32).round() as usize;
            }
            if !hresp.is_pointer_button_down_on() {
                self.hscroll_grab = None;
            }
            if wide {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            let th = if wide { 8.0 } else { 5.0 };
            clip.rect_filled(
                Rect::from_min_size(
                    Pos2::new(thumb_x(b.scroll_x), track.bottom() - th - 2.0),
                    Vec2::new(thumb_w, th),
                ),
                th / 2.0,
                mix(c.bg, c.fg, if wide { 0.35 } else { 0.2 }),
            );
        }
        self.app.hl_pending |= !hl_done;
        if !active_group {
            // A click in another group focuses it first, then acts there.
            if click.is_some() || right_click.is_some() {
                self.app.set_cur(cur);
                self.app.focus = Focus::Editor;
                self.app.completion = None;
                self.app.hover = None;
            } else {
                return None;
            }
        }
        // Where a mouse-hover popup points.
        self.hover_anchor_px = self
            .app
            .hover
            .as_ref()
            .and_then(|h| h.anchor)
            .and_then(|a| {
                let b = self.app.buf()?;
                let line = b.lines.get(a.line)?;
                let row = a.line.checked_sub(b.scroll_y)?;
                Some(Pos2::new(
                    x0 + display_col(line, a.col) as f32 * cw,
                    r.top() + row as f32 * lh,
                ))
            });
        self.keyboard_spot = cursor_px.map(|p| p + Vec2::new(0.0, lh));
        if let Some((pos, n, ext)) = click {
            self.app.editor_click(pos, n, ext);
        } else if let Some(pos) = drag {
            self.app.editor_drag(pos);
        }
        if let Some((pos, pt, in_sel)) = right_click {
            // Like VS Code: right-click outside the selection moves the cursor there first.
            if !in_sel {
                self.app.editor_click(pos, 1, false);
            }
            self.app.focus = Focus::Editor;
            self.app
                .open_context_menu(crate::menu::ContextKind::Editor, (pt.x, pt.y));
        }
        let over_popup = self
            .hover_rect
            .zip(ui.ctx().pointer_hover_pos())
            .is_some_and(|(hr, p)| hr.expand(8.0).contains(p));
        if let Some(wait) = self.app.mouse_hover(hover_at, over_popup) {
            ui.ctx().request_repaint_after(wait);
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

    fn draw_status(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
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
        // Right side, Sublime style: each item is clickable.
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
            right.push((
                if b.indent_unit == "\t" {
                    format!("Tab Size: {}", b.tab_size)
                } else {
                    format!("Spaces: {}", b.indent_unit.len())
                },
                Some(crate::commands::Command::Indentation),
            ));
            right.push((
                b.hl.syntax.clone(),
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

    /// The bottom panel: a tab strip (terminals, CONSOLE) and the active view.
    fn draw_bottom_panel(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        use crate::app::PanelTab;
        let bg = mix(c.bg, Color32::BLACK, 0.25);
        p.rect_filled(r, 0.0, bg);
        p.line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, c.border));
        // Resize by dragging the top edge.
        let edge = Rect::from_min_max(
            r.left_top() - Vec2::new(0.0, 3.0),
            r.right_top() + Vec2::new(0.0, 3.0),
        );
        let er = ui.interact(edge, Id::unique("panel-resize"), Sense::drag());
        if er.hovered() || er.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            p.line_segment([r.left_top(), r.right_top()], Stroke::new(2.0, c.accent));
        }
        if er.dragged() {
            self.app.panel.height = (self.app.panel.height - er.drag_delta().y).clamp(80.0, 1200.0);
        }
        // Tab strip.
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), 26.0));
        let font = FontId::proportional(11.0);
        let mut x = r.left() + 8.0;
        let mut tabs: Vec<(PanelTab, String)> = self
            .app
            .terminals
            .iter()
            .enumerate()
            .map(|(i, t)| {
                (
                    PanelTab::Terminal(t.id),
                    format!("TERMINAL {}: {}", i + 1, t.title()),
                )
            })
            .collect();
        tabs.push((PanelTab::Console, "CONSOLE".into()));
        let mut clicked = None;
        let mut close: Option<usize> = None;
        let mut context: Option<(PanelTab, Pos2)> = None;
        for (tab, label) in &tabs {
            let label: String = label.chars().take(36).collect();
            let is_term = matches!(tab, PanelTab::Terminal(_));
            let w = ui
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), c.fg).size().x)
                + if is_term { 38.0 } else { 20.0 };
            let tr = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(w, head.height()));
            x += w;
            let active = *tab == self.app.panel.tab;
            let resp = ui.interact(
                tr,
                Id::unique(("panel-tab", format!("{tab:?}"))),
                Sense::click(),
            );
            // Terminal tabs: × close button (shown on hover / when active), middle-click
            // closes, right-click opens the terminal's context menu.
            if let PanelTab::Terminal(id) = tab {
                let xr = Rect::from_center_size(
                    Pos2::new(tr.right() - 12.0, tr.center().y),
                    Vec2::splat(14.0),
                );
                let over_x = resp.hover_pos().is_some_and(|hp| xr.contains(hp));
                if active || resp.hovered() {
                    if over_x {
                        p.rect_filled(xr, 3.0, mix(bg, c.fg, 0.12));
                    }
                    let k = 3.5;
                    let o = xr.center();
                    let st = Stroke::new(1.2, if over_x { c.fg } else { c.dim });
                    p.line_segment([o + Vec2::new(-k, -k), o + Vec2::new(k, k)], st);
                    p.line_segment([o + Vec2::new(-k, k), o + Vec2::new(k, -k)], st);
                }
                if (resp.clicked() && over_x) || resp.middle_clicked() {
                    close = Some(*id);
                }
            }
            if resp.secondary_clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                context = Some((*tab, pos));
            }
            let col = if active {
                c.fg
            } else if resp.hovered() {
                mix(c.dim, c.fg, 0.5)
            } else {
                c.dim
            };
            p.text(
                Pos2::new(tr.left() + 10.0, tr.center().y),
                Align2::LEFT_CENTER,
                &label,
                font.clone(),
                col,
            );
            if active {
                p.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(tr.left() + 6.0, tr.bottom() - 2.0),
                        Pos2::new(tr.right() - 6.0, tr.bottom()),
                    ),
                    0.0,
                    c.accent,
                );
            }
            if resp.clicked() && close.is_none() {
                clicked = Some(*tab);
            }
        }
        if let Some(id) = close {
            self.app.close_terminal(id);
        }
        if let Some((tab, pos)) = context {
            self.app.panel.tab = tab;
            let kind = if tab == PanelTab::Console {
                crate::menu::ContextKind::Console
            } else {
                crate::menu::ContextKind::Terminal
            };
            self.app.focus = if tab == PanelTab::Console {
                Focus::Console
            } else {
                Focus::Terminal
            };
            self.app.open_context_menu(kind, (pos.x, pos.y));
        }
        // "+" new terminal, "x" kill the current one.
        let btn = |p: &Painter, ui: &mut egui::Ui, at: Pos2, id: &str, label: &str| -> bool {
            let br = Rect::from_center_size(at, Vec2::splat(18.0));
            let resp = ui.interact(br, Id::unique(id), Sense::click());
            if resp.hovered() {
                p.rect_filled(br, 3.0, mix(bg, c.fg, 0.1));
            }
            p.text(
                br.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(14.0),
                c.dim,
            );
            resp.clicked()
        };
        if btn(
            p,
            ui,
            Pos2::new(r.right() - 20.0, head.center().y),
            "panel-close",
            "x",
        ) {
            match self.app.panel.tab {
                PanelTab::Terminal(id) => self.app.close_terminal(id),
                PanelTab::Console => {
                    self.app.panel.visible = false;
                    self.app.focus = Focus::Editor;
                }
            }
        }
        if btn(
            p,
            ui,
            Pos2::new(r.right() - 42.0, head.center().y),
            "panel-new",
            "+",
        ) {
            self.app.new_terminal();
        }
        if let Some(tab) = clicked {
            self.app.panel.tab = tab;
            self.app.focus = if tab == PanelTab::Console {
                Focus::Console
            } else {
                Focus::Terminal
            };
        }
        let body = Rect::from_min_max(Pos2::new(r.left(), head.bottom()), r.max);
        match self.app.panel.tab {
            PanelTab::Terminal(id) => self.draw_terminal(ui, p, body, id, c),
            PanelTab::Console => self.draw_console(ui, p, body, c),
        }
    }

    fn term_color(col: vt100::Color, default: Color32) -> Color32 {
        match col {
            vt100::Color::Default => default,
            vt100::Color::Idx(i) => {
                let (r, g, b) = crate::terminal::palette(i);
                Color32::from_rgb(r, g, b)
            }
            vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        }
    }

    fn draw_terminal(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, id: usize, c: &Palette) {
        let focused = self.app.focus == Focus::Terminal && self.app.modal.is_none();
        let font = FontId::monospace((self.app.font_size - 1.0).max(8.0));
        let (cw, rh) = ui
            .ctx()
            .fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let lh = (rh * 1.15).round();
        let inner = r.shrink2(Vec2::new(10.0, 4.0));
        let cols = ((inner.width() / cw).floor() as u16).max(10);
        let rows = ((inner.height() / lh).floor() as u16).max(2);
        let bg = mix(c.bg, Color32::BLACK, 0.25);
        let Some(t) = self.app.terminals.iter_mut().find(|t| t.id == id) else {
            return;
        };
        t.resize(rows, cols);
        // Mouse: click focuses, drag selects (screen cells), wheel scrolls history.
        let resp = ui.interact(inner, Id::unique(("term", id)), Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            let dy = ui.ctx().input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                self.term_scroll_acc += dy / lh;
                let n = self.term_scroll_acc.trunc();
                self.term_scroll_acc -= n;
                if n != 0.0 {
                    t.scroll(n as isize);
                }
            }
        }
        let cell_at = |pt: Pos2| -> (u16, u16) {
            let row = ((pt.y - inner.top()) / lh)
                .floor()
                .clamp(0.0, rows as f32 - 1.0) as u16;
            let col = ((pt.x - inner.left()) / cw)
                .floor()
                .clamp(0.0, cols as f32 - 1.0) as u16;
            (row, col)
        };
        if let Some(pt) = resp.interact_pointer_pos() {
            if resp.drag_started() {
                let at = cell_at(pt);
                t.sel = Some((at, at));
            } else if resp.dragged() {
                if let Some((a, _)) = t.sel {
                    t.sel = Some((a, cell_at(pt)));
                }
            } else if resp.clicked() {
                t.sel = None;
            }
        }
        if resp.clicked() || resp.drag_started() {
            self.app.focus = Focus::Terminal;
        }
        let term_menu = resp
            .secondary_clicked()
            .then(|| resp.interact_pointer_pos())
            .flatten();
        let sel = t.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let exited = t.is_exited();
        let clip = p.with_clip_rect(r);
        let fg = c.fg;
        t.with_screen(|s| {
            let (crow, ccol) = s.cursor_position();
            for row in 0..rows {
                let y = inner.top() + row as f32 * lh;
                let mut job = LayoutJob::default();
                let mut col = 0u16;
                while col < cols {
                    let Some(cell) = s.cell(row, col) else { break };
                    let (mut f, mut b) = (
                        Self::term_color(cell.fgcolor(), fg),
                        Self::term_color(cell.bgcolor(), bg),
                    );
                    if cell.inverse() {
                        std::mem::swap(&mut f, &mut b);
                    }
                    if cell.dim() {
                        f = mix(b, f, 0.6);
                    }
                    let selected = sel.is_some_and(|((r0, c0), (r1, c1))| {
                        (row, col) >= (r0, c0) && (row, col) <= (r1, c1)
                    });
                    if selected {
                        b = c.sel;
                    }
                    let width = if cell.is_wide() { 2 } else { 1 };
                    if b != bg {
                        clip.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(inner.left() + col as f32 * cw, y),
                                Vec2::new(cw * width as f32, lh),
                            ),
                            0.0,
                            b,
                        );
                    }
                    let text = if cell.has_contents() {
                        cell.contents().to_string()
                    } else {
                        " ".to_string()
                    };
                    job.append(
                        &text,
                        0.0,
                        egui::TextFormat {
                            font_id: font.clone(),
                            color: if cell.bold() {
                                mix(f, Color32::WHITE, 0.15)
                            } else {
                                f
                            },
                            italics: cell.italic(),
                            underline: if cell.underline() {
                                Stroke::new(1.0, f)
                            } else {
                                Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                    col += width;
                }
                let g = p.layout_job(job);
                clip.galley(Pos2::new(inner.left(), y + (lh - rh) / 2.0), g, fg);
            }
            if !s.hide_cursor() && s.scrollback() == 0 && !exited {
                let cr = Rect::from_min_size(
                    Pos2::new(
                        inner.left() + ccol as f32 * cw,
                        inner.top() + crow as f32 * lh,
                    ),
                    Vec2::new(cw, lh),
                );
                if focused {
                    clip.rect_filled(cr, 0.0, mix(c.accent, bg, 0.35));
                } else {
                    clip.rect_stroke(cr, 0.0, Stroke::new(1.0, c.dim), StrokeKind::Inside);
                }
            }
            if s.scrollback() > 0 {
                clip.text(
                    r.right_top() + Vec2::new(-12.0, 6.0),
                    Align2::RIGHT_TOP,
                    format!("scrolled back {} lines", s.scrollback()),
                    FontId::proportional(11.0),
                    c.accent,
                );
            }
        });
        if exited {
            clip.text(
                Pos2::new(inner.left(), inner.bottom() - lh / 2.0),
                Align2::LEFT_CENTER,
                format!(
                    "[process exited with code {} — press any key to close]",
                    self.app
                        .terminals
                        .iter()
                        .find(|t| t.id == id)
                        .and_then(|t| *t.exit_code.lock().unwrap())
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into())
                ),
                font,
                c.dim,
            );
        }
        if let Some(pos) = term_menu {
            self.app.focus = Focus::Terminal;
            self.app
                .open_context_menu(crate::menu::ContextKind::Terminal, (pos.x, pos.y));
        }
    }

    fn draw_console(&mut self, ui: &mut egui::Ui, p: &Painter, r: Rect, c: &Palette) {
        use crate::log::Level;
        let focused = self.app.focus == Focus::Console && self.app.modal.is_none();
        let bg = mix(c.bg, Color32::BLACK, 0.25);
        let small = FontId::proportional(11.5);
        let filt = &self.app.console.filter;
        let hint = format!(
            "{}{}{}type help",
            if self.app.console.show_debug {
                "debug output on · "
            } else {
                ""
            },
            if self.app.console.wrap {
                ""
            } else {
                "no wrap · "
            },
            if filt.is_empty() {
                String::new()
            } else {
                format!("filter \"{filt}\" · ")
            }
        );
        p.text(
            Pos2::new(r.right() - 64.0, r.top() - 13.0),
            Align2::RIGHT_CENTER,
            hint,
            small.clone(),
            c.dim,
        );
        let input = Rect::from_min_max(Pos2::new(r.left(), r.bottom() - 26.0), r.max);
        let body = Rect::from_min_max(
            Pos2::new(r.left() + 12.0, r.top() + 2.0),
            Pos2::new(r.right() - 12.0, input.top()),
        );
        let entries = self.app.console_entries();
        let mono = FontId::monospace(12.0);
        let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&mono, 'M'));
        let lh = 17.0;
        let wrap_cols = ((body.width() / cw).floor() as usize).max(20);
        // Visual rows: (entry index, start char, end char).
        let lines: Vec<Vec<char>> = entries
            .iter()
            .map(|e| App::console_line(e).chars().collect())
            .collect();
        let mut rows: Vec<(usize, usize, usize)> = vec![];
        for (i, l) in lines.iter().enumerate() {
            if self.app.console.wrap && l.len() > wrap_cols {
                let mut s = 0;
                while s < l.len() {
                    rows.push((i, s, (s + wrap_cols).min(l.len())));
                    s += wrap_cols;
                }
            } else {
                rows.push((i, 0, l.len()));
            }
        }
        let n_vis = ((body.height() / lh).floor() as usize).max(1);
        let max_scroll = rows.len().saturating_sub(n_vis);
        let resp = ui.interact(body, Id::unique("console"), Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            let d = ui
                .ctx()
                .input(|i| (i.smooth_scroll_delta, i.modifiers.shift));
            let (dx, dy) = if d.1 { (d.0.y, 0.0) } else { (d.0.x, d.0.y) };
            if dy != 0.0 {
                let s = self.app.console.scroll as f32 + dy / lh;
                self.app.console.scroll = s.max(0.0).round() as usize;
            }
            if dx != 0.0 && !self.app.console.wrap {
                let longest = lines.iter().map(|l| l.len()).max().unwrap_or(0) as f32 * cw;
                self.app.console.hscroll = (self.app.console.hscroll - dx)
                    .clamp(0.0, (longest - body.width() + 40.0).max(0.0));
            }
        }
        self.app.console.scroll = self.app.console.scroll.min(max_scroll);
        let end = rows.len() - self.app.console.scroll;
        let start = end.saturating_sub(n_vis);
        let hx = if self.app.console.wrap {
            0.0
        } else {
            self.app.console.hscroll
        };
        // Hit testing for selection.
        let hit = |pt: Pos2| -> Option<crate::app::ConsolePos> {
            let vr = start + ((pt.y - body.top()) / lh).floor().max(0.0) as usize;
            let (ei, s, e) = *rows.get(vr.min(end.saturating_sub(1)))?;
            let col = (((pt.x - body.left() + hx) / cw).round().max(0.0) as usize + s).min(e);
            Some((entries[ei].id, col))
        };
        if let Some(pt) = resp.interact_pointer_pos() {
            if resp.double_clicked() {
                if let Some((id, col)) = hit(pt)
                    && let Some(i) = entries.iter().position(|e| e.id == id)
                {
                    let l = &lines[i];
                    let w = |ch: char| ch.is_alphanumeric() || ch == '_';
                    let (mut a, mut b) = (col.min(l.len()), col.min(l.len()));
                    while a > 0 && w(l[a - 1]) {
                        a -= 1;
                    }
                    while b < l.len() && w(l[b]) {
                        b += 1;
                    }
                    self.app.console.sel = Some(((id, a), (id, b)));
                }
            } else if resp.drag_started() {
                self.app.console.sel = hit(pt).map(|h| (h, h));
            } else if resp.dragged() {
                if let (Some((a, _)), Some(h)) = (self.app.console.sel, hit(pt)) {
                    self.app.console.sel = Some((a, h));
                }
            } else if resp.clicked() {
                self.app.console.sel = None;
            }
        }
        if resp.clicked() || resp.drag_started() {
            self.app.focus = Focus::Console;
        }
        if resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.app.focus = Focus::Console;
            self.app
                .open_context_menu(crate::menu::ContextKind::Console, (pos.x, pos.y));
        }
        let sel = self
            .app
            .console
            .sel
            .map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let clip = p.with_clip_rect(body);
        for (row, &(ei, s, e)) in rows[start..end].iter().enumerate() {
            let y = body.top() + row as f32 * lh;
            let ent = &entries[ei];
            let x0 = body.left() - hx;
            // Selection highlight within this visual row.
            if let Some(((ai, ac), (bi, bc))) = sel
                && ent.id >= ai
                && ent.id <= bi
            {
                let from = if ent.id == ai { ac.max(s) } else { s };
                let to = if ent.id == bi { bc.min(e) } else { e };
                if to > from || (ent.id != bi && e == lines[ei].len()) {
                    let extra = if ent.id != bi && e == lines[ei].len() {
                        0.5
                    } else {
                        0.0
                    };
                    clip.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(x0 + (from - s) as f32 * cw, y),
                            Pos2::new(x0 + (to.max(from) - s) as f32 * cw + extra * cw, y + lh),
                        ),
                        0.0,
                        c.sel,
                    );
                }
            }
            // time [source] text, colored per part (only on the first visual row).
            let text: String = lines[ei][s..e].iter().collect();
            let level_col = match ent.level {
                Level::Debug => c.dim,
                Level::Info => c.fg,
                Level::Warn => c.warn,
                Level::Error => c.err,
            };
            let mut job = LayoutJob::default();
            let prefix_len = if s == 0 {
                (ent.time.len().min(8) + ent.source.len() + 4).min(text.chars().count())
            } else {
                0
            };
            let (pre, rest): (String, String) = (
                text.chars().take(prefix_len).collect(),
                text.chars().skip(prefix_len).collect(),
            );
            let fmt = |col| egui::TextFormat {
                font_id: mono.clone(),
                color: col,
                ..Default::default()
            };
            job.append(&pre, 0.0, fmt(mix(c.info, c.dim, 0.4)));
            job.append(&rest, 0.0, fmt(level_col));
            let g = p.layout_job(job);
            clip.galley(Pos2::new(x0, y + 1.0), g, c.fg);
        }
        if entries.is_empty() {
            clip.text(
                Pos2::new(body.left(), body.top() + 12.0),
                Align2::LEFT_CENTER,
                "No log output yet.",
                small.clone(),
                c.dim,
            );
        }
        if self.app.console.scroll > 0 {
            p.text(
                Pos2::new(r.right() - 12.0, input.top() - 4.0),
                Align2::RIGHT_BOTTOM,
                format!("{} newer line(s) below", self.app.console.scroll),
                small,
                c.accent,
            );
        }
        // Input line.
        p.rect_filled(input, 0.0, mix(bg, c.fg, 0.04));
        let pr = p.text(
            Pos2::new(r.left() + 12.0, input.center().y),
            Align2::LEFT_CENTER,
            ">",
            mono.clone(),
            c.accent,
        );
        let it = &self.app.console.input;
        let tr = p.text(
            Pos2::new(pr.right() + 8.0, input.center().y),
            Align2::LEFT_CENTER,
            it,
            mono.clone(),
            c.fg,
        );
        if focused {
            let before: String = it.chars().take(self.app.console.cursor).collect();
            let bw = p
                .ctx()
                .fonts_mut(|f| f.layout_no_wrap(before, mono.clone(), c.fg).size().x);
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(tr.left() + bw, input.top() + 5.0),
                    Vec2::new(2.0, input.height() - 10.0),
                ),
                0.0,
                c.accent,
            );
        }
        let ir = ui.interact(input, Id::unique("console-input"), Sense::click());
        if ir.clicked() {
            self.app.focus = Focus::Console;
        }
    }

    fn draw_lsp_info(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
        use crate::app::Tone;
        let Some(Modal::LspInfo(v)) = &self.app.modal else {
            return;
        };
        let lines = self.app.lsp_info();
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

    fn draw_hover(&mut self, p: &Painter, editor: Rect, at: Pos2, c: &Palette) {
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

    /// A right-click menu (mouse or Shift+F10).
    fn draw_context_menu(&mut self, ui: &mut egui::Ui, p: &Painter, full: Rect, c: &Palette) {
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

impl eframe::App for Gui {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn on_exit(&mut self) {
        self.app.save_workspace();
        self.app.lsp.shutdown();
    }
}

// ===================================================================== text helpers

fn char_w(c: char, at: usize) -> usize {
    if c == '\t' {
        let t = TAB.with(|t| t.get()).max(1);
        t - at % t
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

/// Completion kind icon: a small tinted tile with a letter, colored like VS Code's symbol
/// icons (purple = callables, blue = values, orange = types). Drawn as shapes + ASCII so it
/// never depends on icon-font coverage.
fn kind_icon(p: &Painter, center: Pos2, kind: &str, bg: Color32) {
    let purple = Color32::from_rgb(0xB1, 0x80, 0xD7);
    let blue = Color32::from_rgb(0x75, 0xBE, 0xFF);
    let orange = Color32::from_rgb(0xEE, 0x9D, 0x28);
    let grey = Color32::from_rgb(0xA0, 0xA0, 0xA8);
    let (glyph, col) = match kind {
        "fn" => ("f", purple),
        "meth" => ("m", purple),
        "ctor" => ("c", purple),
        "var" => ("v", blue),
        "field" => ("F", blue),
        "prop" => ("p", blue),
        "const" => ("K", blue),
        "val" | "unit" => ("1", blue),
        "iface" => ("I", blue),
        "class" => ("C", orange),
        "struct" => ("S", orange),
        "enum" => ("E", orange),
        "variant" => ("e", orange),
        "type" => ("T", orange),
        "event" => ("!", orange),
        "mod" => ("M", grey),
        "kw" => ("k", grey),
        "snip" => ("s", grey),
        "file" => ("d", grey),
        "dir" => ("D", grey),
        "ref" => ("r", grey),
        "op" => ("+", grey),
        "color" => ("#", grey),
        _ => ("t", grey),
    };
    let r = Rect::from_center_size(center, Vec2::splat(16.0));
    p.rect_filled(r, 3.0, mix(bg, col, 0.22));
    p.rect_stroke(
        r,
        3.0,
        Stroke::new(1.0, mix(bg, col, 0.55)),
        StrokeKind::Inside,
    );
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::monospace(11.0),
        col,
    );
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

/// Caret opacity `elapsed` after it last moved, and when the next frame is needed.
/// The caret stays solid for a moment after moving (like Sublime), then animates.
pub fn caret_alpha(style: CaretStyle, elapsed: Duration) -> (f32, Option<Duration>) {
    const HOLD: f32 = 0.5; // seconds solid after a move
    const PERIOD: f32 = 1.0; // one full off/on cycle
    let t = elapsed.as_secs_f32();
    match style {
        CaretStyle::Solid => (1.0, None),
        _ if t < HOLD => (1.0, Some(Duration::from_secs_f32(HOLD - t))),
        CaretStyle::Blink => {
            let phase = (t - HOLD) % PERIOD;
            let half = PERIOD / 2.0;
            let alpha = if phase < half { 0.0 } else { 1.0 };
            let left = if phase < half {
                half - phase
            } else {
                PERIOD - phase
            };
            (alpha, Some(Duration::from_secs_f32(left.max(0.001))))
        }
        CaretStyle::Smooth => {
            // Cosine fade: 1 → 0 → 1 over a period, eased so it lingers at both ends.
            let phase = ((t - HOLD) % PERIOD) / PERIOD;
            let alpha = 0.5 + 0.5 * (phase * std::f32::consts::TAU).cos();
            (alpha, Some(Duration::from_millis(16)))
        }
    }
}

fn is_digit_key(k: egui::Key) -> bool {
    use egui::Key::*;
    matches!(
        k,
        Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9
    )
}

/// Draw a file or folder icon centred at `at` (sidebar, tabs, Goto Anything). Folders are a
/// drawn folder shape (open or closed); files a coloured badge with a short label, both
/// from icons.toml.
type IconTextures = HashMap<PathBuf, Option<egui::TextureHandle>>;

/// Draw a package icon image if it loads; returns false to fall back to the drawn icon.
fn draw_icon_image(
    p: &Painter,
    tex: &mut IconTextures,
    path: &Path,
    at: Pos2,
    tint: Color32,
) -> bool {
    let t = tex.entry(path.to_path_buf()).or_insert_with(|| {
        let img = image::open(path).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        Some(p.ctx().load_texture(
            format!("icon:{}", path.display()),
            ci,
            egui::TextureOptions::LINEAR,
        ))
    });
    let Some(t) = t else { return false };
    let r = Rect::from_center_size(at, Vec2::splat(16.0));
    p.image(
        t.id(),
        r,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        tint,
    );
    true
}

#[allow(clippy::too_many_arguments)]
fn draw_icon(
    p: &Painter,
    cfg: &crate::config::Config,
    tex: &mut IconTextures,
    at: Pos2,
    name: &str,
    is_dir: bool,
    open: bool,
    alpha: f32,
    bg: Color32,
) {
    if !cfg.editor.file_icons {
        return;
    }
    let fade = |c: Color32| mix(bg, c, alpha);
    let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
    if is_dir {
        let img = if open {
            &cfg.folder_images.1
        } else {
            &cfg.folder_images.0
        };
        if let Some(path) = img
            && draw_icon_image(p, tex, path, at, tint)
        {
            return;
        }
    } else if let Some(path) = cfg.icon_for(name).and_then(|d| d.image.as_ref())
        && draw_icon_image(p, tex, Path::new(path), at, tint)
    {
        return;
    }
    let hex = |s: Option<&str>, d: Color32| {
        s.and_then(crate::config::parse_hex_color)
            .map(|(r, g, b)| Color32::from_rgb(r, g, b))
            .unwrap_or(d)
    };
    if is_dir {
        let col = fade(hex(
            cfg.folder_color.as_deref(),
            Color32::from_rgb(0x8F, 0xA1, 0xB3),
        ));
        let body = Rect::from_center_size(at + Vec2::new(0.0, 1.0), Vec2::new(14.0, 10.0));
        // Tab on the top-left edge.
        p.rect_filled(
            Rect::from_min_size(body.min - Vec2::new(0.0, 2.0), Vec2::new(6.0, 3.0)),
            1.0,
            col,
        );
        if open {
            p.rect_filled(body, 1.5, mix(bg, col, 0.55));
            // The open flap, slanted forward.
            let top = body.top() + 3.5;
            p.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(body.left() + 2.5, top),
                    Pos2::new(body.right() + 1.0, top),
                    Pos2::new(body.right() - 1.5, body.bottom()),
                    Pos2::new(body.left(), body.bottom()),
                ],
                col,
                Stroke::NONE,
            ));
        } else {
            p.rect_filled(body, 1.5, col);
        }
        return;
    }
    let def = cfg.icon_for(name);
    let label = def.map(|d| d.label.as_str()).unwrap_or("");
    let base = hex(
        def.map(|d| d.color.as_str()),
        Color32::from_rgb(0x7D, 0x85, 0x90),
    );
    let col = fade(base);
    if label.is_empty() {
        // Plain document with a folded corner.
        let r = Rect::from_center_size(at, Vec2::new(10.0, 13.0));
        let k = 3.5;
        p.add(egui::Shape::convex_polygon(
            vec![
                r.left_top(),
                Pos2::new(r.right() - k, r.top()),
                Pos2::new(r.right(), r.top() + k),
                r.right_bottom(),
                r.left_bottom(),
            ],
            col,
            Stroke::NONE,
        ));
        return;
    }
    let font = FontId::proportional(if label.chars().count() >= 3 { 7.5 } else { 8.5 });
    let galley = p.layout_no_wrap(label.to_string(), font, Color32::WHITE);
    let w = (galley.size().x + 5.0).max(14.0);
    let r = Rect::from_center_size(at, Vec2::new(w, 13.0));
    p.rect_filled(r, 3.0, col);
    // Dark text on light badges, light text on dark ones.
    let lum = 0.299 * base.r() as f32 + 0.587 * base.g() as f32 + 0.114 * base.b() as f32;
    let ink = if lum > 140.0 {
        Color32::from_rgb(0x1E, 0x1E, 0x1E)
    } else {
        Color32::WHITE
    };
    let pos = r.center() - galley.size() / 2.0;
    // Short labels are drawn twice, a hair apart, for a semi-bold look; three letters
    // would smudge together, so they stay regular.
    if label.chars().count() < 3 {
        p.galley(pos + Vec2::new(0.35, 0.0), galley.clone(), fade(ink));
    }
    p.galley(pos, galley, fade(ink));
}
