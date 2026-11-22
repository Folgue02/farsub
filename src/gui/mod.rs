//! The GUI: everything is custom-painted with egui's painter so the editor,
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

use crate::app::{App, AppEvent, EventTx, Focus, Modal};
use crate::config::{CaretStyle, Config};
use crate::editor::{Buffer, Pos};
use crate::highlight::Span;
use crate::keys::Key;

mod editor_view;
mod find_view;
mod icons;
mod menus;
mod modals;
mod palette_view;
mod panel;
mod popups;
mod sidebar;
mod status;
mod tabs;
mod timers;

use crate::palette::Mode;
use icons::*;

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

/// The primary button went down on this widget this frame. Tabs, sidebar rows and the
/// menu bar react to the press (like VS Code); buttons keep acting on release.
/// Keep the mouse off what's under a popup or dialog. Called with its box (or the whole
/// window, for a dialog that dims it) before its own widgets: it takes the hover and
/// clicks there, so the editor or terminal below doesn't show its I-beam or react.
fn block(ui: &egui::Ui, r: Rect, what: &'static str) {
    ui.interact(r, Id::unique(("block", what)), Sense::click_and_drag());
}

fn pressed(ui: &egui::Ui, resp: &egui::Response) -> bool {
    resp.hovered() && ui.input(|i| i.pointer.primary_pressed())
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
    /// (buffer id, scroll line, scroll row) the editor last left the view at: if the app
    /// moves the view elsewhere, the smooth-scroll pixel offset is dropped.
    scroll_mark: HashMap<usize, (usize, usize)>,
    /// Sidebar: pixels the top row is scrolled past.
    side_px: f32,
    term_scroll_acc: f32,
    /// Per file (buffer id): the cursor and version the view last followed, so switching
    /// tabs keeps a file where it was scrolled to (only a moved cursor scrolls).
    last_cursor: HashMap<usize, (Pos, i32)>,
    /// Alt+drag column selection: where it started (line, display column).
    col_drag: Option<(usize, usize)>,
    /// Mouse selection in progress: the press's click count (1 characters, 2 words,
    /// 3 lines) and what the press selected, which dragging extends from.
    sel_drag: Option<(u32, Pos, Pos)>,
    /// The last press in an editor (when, where, click count): double and triple clicks
    /// are counted on the press (egui counts them on release, too late to drag by word).
    last_press: Option<(Instant, Pos2, u32)>,
    /// Where the pointer last was during a selection drag (it may be outside the window)
    /// and when the view may next scroll toward it.
    drag_pt: Option<Pos2>,
    autoscroll_at: Instant,
    /// Caret position/version and when it last changed: the blink phase restarts there so
    /// the caret is solid while typing or moving.
    caret_mark: (Option<(usize, Pos, i32)>, Instant),
    /// Where on the scrollbar thumb the drag started (pointer offset from the thumb's
    /// start), for the editor's vertical / horizontal scrollbars.
    vscroll_grab: Option<f32>,
    hscroll_grab: Option<f32>,
    /// Horizontal wheel scrolling not yet worth a whole column (trackpads send fractions).
    hscroll_rem: f32,
    /// Decoded icon-package images (None: failed to load, don't retry every frame).
    icon_tex: IconTextures,
    /// Buffer id → (version, widest line in columns, last full scan), for the horizontal
    /// scrollbar.
    widest_cache: HashMap<usize, (i32, usize, Instant)>,
    /// The focused group's editor rect (tests, popups).
    editor_rect: Rect,
    /// The focused editor's visible rows last frame: (line, start col, end col, y).
    last_rows: Vec<(usize, usize, usize, f32)>,
    /// (char width, text left edge, first line top, line height) of the last frame.
    text_geom: (f32, f32, f32, f32),
    last_tree_sel: Option<usize>,
    last_title: String,
    /// The editor font now loaded (`font_face`; None before the first frame).
    applied_font: Option<String>,
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
    /// Per group: how far its tab strip is scrolled (points), and the tab it last showed
    /// (the strip follows a newly shown tab; the wheel scrolls it freely otherwise).
    pub tab_scroll: HashMap<usize, (f32, Option<usize>)>,
    /// The buffer whose tab × the last press on a tab landed on.
    close_press: Option<usize>,
    /// This window's listing for `farsub FILE` (none in settings windows and tests), and
    /// whether the window had focus last frame.
    pub instance: Option<(crate::instance::Instance, std::sync::mpsc::Receiver<crate::instance::Request>)>,
    was_focused: bool,
    /// The tab (buffer id) being dragged to another place or group.
    tab_drag: Option<usize>,
    /// This frame's tabs (group, buffer id, rect) and groups (tab strip, column), where a
    /// dragged tab can be dropped.
    pub(crate) tab_hits: Vec<(usize, usize, Rect)>,
    pub(crate) group_cols: Vec<(Rect, Rect)>,
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
            scroll_mark: HashMap::new(),
            side_px: 0.0,
            term_scroll_acc: 0.0,
            last_cursor: HashMap::new(),
            col_drag: None,
            sel_drag: None,
            last_press: None,
            drag_pt: None,
            autoscroll_at: Instant::now(),
            caret_mark: (None, Instant::now()),
            vscroll_grab: None,
            hscroll_grab: None,
            hscroll_rem: 0.0,
            icon_tex: HashMap::new(),
            widest_cache: HashMap::new(),
            editor_rect: Rect::NOTHING,
            last_rows: vec![],
            text_geom: (0.0, 0.0, 0.0, 0.0),
            last_tree_sel: None,
            last_title: String::new(),
            applied_font: None,
            menu_titles: vec![],
            status_hits: vec![],
            hover_rect: None,
            hover_anchor_px: None,
            keyboard_spot: None,
            sidebar_sel_px: None,
            tab_scroll: HashMap::new(),
            close_press: None,
            instance: None,
            was_focused: false,
            tab_drag: None,
            tab_hits: Vec::new(),
            group_cols: Vec::new(),
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
    /// Screen point of a caret position on an unscrolled, unwrapped buffer (tests drive
    /// the mouse).
    pub fn pos_px_for_test(&self, line: usize, col: usize) -> egui::Pos2 {
        let (cw, left, top, lh) = self.text_geom;
        egui::Pos2::new(left + col as f32 * cw, top + line as f32 * lh + lh / 2.0)
    }

    #[cfg(test)]
    /// Icon images loaded so far: (path, decoded ok).
    pub fn icon_textures_for_test(&self) -> Vec<(PathBuf, bool)> {
        self.icon_tex
            .iter()
            .map(|(p, t)| (p.clone(), t.is_some()))
            .collect()
    }

    #[cfg(test)]
    /// The focused editor's visible rows: (line, first display column, end column, y).
    #[cfg(test)]
    pub fn rows_for_test(&self) -> Vec<(usize, usize, usize, f32)> {
        self.last_rows.clone()
    }

    /// The focused group's editor area last frame.
    #[cfg(test)]
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
        // Sidebar folders listed in the background.
        any |= self.app.tree.poll();
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
        self.remote_requests(&ctx);
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        self.app.track_recent(ctrl);
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
        // The editor font (`font_face`), loaded when it changes (also while previewing).
        let want = self.app.config.editor.font_face.clone();
        if self.applied_font.as_deref() != Some(want.as_str()) {
            self.apply_font(&ctx, &want);
            self.applied_font = Some(want);
        }
        self.pump();
        self.handle_input(&ctx);
        self.pump();
        self.app.reap_terminals();
        self.answer_file_request();

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
        // The find panel spans the window above the status bar.
        let find_h = self.find_panel_height();
        let find_panel = (find_h > 0.0).then(|| {
            let r = Rect::from_min_max(Pos2::new(rest.left(), rest.bottom() - find_h), rest.max);
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
        // Editor groups side by side (Layout → Columns), each with its own tabs.
        let n = self.app.layout.max(1);
        if self.app.group_split.len() != n {
            self.app.group_split = vec![1.0 / n as f32; n];
        }
        self.app.hl_pending = false;
        let mut editor = rest;
        let mut cursor_px = None;
        let mut x = rest.left();
        self.tab_hits.clear();
        self.group_cols.clear();
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
            self.group_cols.push((tabs, col));
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
        self.drop_tab(ui, &painter, &c);
        self.draw_status(ui, &painter, status, &c);
        if let Some(r) = panel {
            self.draw_prompt(&painter, r, &c);
        }
        if let Some(r) = find_panel {
            self.draw_find_panel(ui, &painter, r, &c);
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
            Some(Modal::Confirm(_)) => self.draw_confirm(ui, &over, full, &c),
            Some(Modal::Keymap(_)) => self.draw_keymap(ui, &over, full, &c),
            Some(Modal::Servers(_)) => self.draw_servers(ui, &over, full, &c),
            Some(Modal::LspInfo(_)) => self.draw_lsp_info(ui, &over, full, &c),
            Some(Modal::Timers(_)) => self.draw_timer_dialog(ui, &over, full, &c),
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
        // Check open files against the disk about once a second while the window is in use.
        if ctx.input(|i| i.focused) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        self.app.tick();
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
        // A font with broken metrics (NaN, zero) must not break the layout math.
        let sane = |v: f32, k: f32| if v.is_finite() && v > 0.0 { v } else { font.size * k };
        let (cw, rh) = (sane(cw, 0.6), sane(rh, 1.2));
        let lh = (rh * 1.35).round();
        (cw, lh, rh)
    }
}

impl Gui {
    /// Files sent by `farsub FILE` open here and bring the window to the front; focusing
    /// the window makes it the one they go to.
    fn remote_requests(&mut self, ctx: &egui::Context) {
        let Some((inst, rx)) = &self.instance else { return };
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            inst.focused();
        }
        self.was_focused = focused;
        let reqs: Vec<_> = rx.try_iter().collect();
        for r in &reqs {
            for (path, line, col) in &r.files {
                self.app.open_at(path, *line, *col);
            }
            if let Some(t) = &r.stdin {
                self.app.open_untitled(t);
            }
            self.app.focus = Focus::Editor;
        }
        if !reqs.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
/// The font family holding the Codicons icons.
fn icon_family() -> egui::FontFamily {
    egui::FontFamily::Name("codicon".into())
}

/// A completion item's kind as VS Code shows it: a Codicons symbol icon, colored by group
/// (purple: callables; blue: values and members; orange: types; grey: the rest).
fn kind_icon(p: &Painter, center: Pos2, kind: &str) {
    let purple = Color32::from_rgb(0xB1, 0x80, 0xD7);
    let blue = Color32::from_rgb(0x75, 0xBE, 0xFF);
    let orange = Color32::from_rgb(0xEE, 0x9D, 0x28);
    let grey = Color32::from_rgb(0xA0, 0xA0, 0xA8);
    let (icon, col) = kind_codicon(kind);
    let col = match col {
        KindGroup::Callable => purple,
        KindGroup::Value => blue,
        KindGroup::Type => orange,
        KindGroup::Other => grey,
    };
    p.text(
        center,
        Align2::CENTER_CENTER,
        icon,
        FontId::new(16.0, icon_family()),
        col,
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum KindGroup {
    Callable,
    Value,
    Type,
    Other,
}

/// The Codicons glyph (codepoints from the package's codicon.csv) for a completion kind,
/// following VS Code's choices.
pub(crate) fn kind_codicon(kind: &str) -> (&'static str, KindGroup) {
    use KindGroup::*;
    match kind {
        // Completion kinds, and the symbol kinds Goto Symbol shows ("method", "ns", …).
        "fn" | "meth" | "method" | "ctor" => ("\u{EA8C}", Callable), // symbol-method
        "var" => ("\u{EA88}", Value),                     // symbol-variable
        "field" => ("\u{EB5F}", Value),                   // symbol-field
        "prop" => ("\u{EB65}", Value),                    // symbol-property
        "const" => ("\u{EB5D}", Value),                   // symbol-constant
        "val" => ("\u{EA95}", Value),                     // symbol-enum (VS Code's "value")
        "unit" => ("\u{EA96}", Value),                    // symbol-ruler
        "variant" => ("\u{EB5E}", Value),                 // symbol-enum-member
        "iface" => ("\u{EB61}", Type),                    // symbol-interface
        "class" => ("\u{EB5B}", Type),                    // symbol-class
        "struct" => ("\u{EA91}", Type),                   // symbol-structure
        "enum" => ("\u{EA95}", Type),                     // symbol-enum
        "type" => ("\u{EA92}", Type),                     // symbol-parameter (type parameter)
        "event" => ("\u{EA86}", Type),                    // symbol-event
        "mod" | "ns" | "pkg" | "obj" => ("\u{EA8B}", Other), // symbol-namespace
        "num" => ("\u{EA90}", Value),                     // symbol-numeric
        "bool" => ("\u{EA8F}", Value),                    // symbol-boolean
        "arr" => ("\u{EA8A}", Value),                     // symbol-array
        "null" => ("\u{EB63}", Value),                    // symbol-misc
        "kw" => ("\u{EB62}", Other),                      // symbol-keyword
        "snip" => ("\u{EB66}", Other),                    // symbol-snippet
        "file" => ("\u{EA7B}", Other),                    // symbol-file
        "dir" => ("\u{EA83}", Other),                     // folder
        "ref" => ("\u{EB36}", Other),                     // references
        "op" => ("\u{EB64}", Other),                      // symbol-operator
        "color" => ("\u{EB5C}", Other),                   // symbol-color
        _ => ("\u{EA93}", Other),                         // symbol-key (VS Code's "text")
    }
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
/// The caret stays solid for a moment after moving, then animates.
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
            // ~30 fps: a one-second fade looks just as smooth, at half the idle redraws.
            (alpha, Some(Duration::from_millis(33)))
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

/// Draw a one-line input (find, prompts, palettes, console line, keymap search): its
/// selection, text and caret, left-aligned at `left` inside `field` and vertically
/// centred. Long text scrolls sideways so the caret stays visible. Shows `placeholder`
/// (dimmed) when empty.
#[allow(clippy::too_many_arguments)]
fn draw_line_edit(
    p: &Painter,
    field: Rect,
    left: f32,
    e: &crate::lineedit::LineEdit,
    font: FontId,
    c: &Palette,
    caret: bool,
    placeholder: &str,
) {
    let clip = p.with_clip_rect(field);
    let width = |s: &str| {
        p.ctx()
            .fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), c.fg).size().x)
    };
    // A password field draws dots, never its text.
    let text = e.display_text();
    let text = text.as_ref();
    let x_of = |col: usize| -> f32 {
        let b = text
            .char_indices()
            .nth(col)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        width(&text[..b])
    };
    let room = (field.right() - left - 6.0).max(10.0);
    let cx = x_of(e.cursor());
    let shift = (cx - room).max(0.0);
    let x0 = left - shift;
    let h = font.size + 6.0;
    let y = field.center().y;
    if let Some((a, b)) = e.selection() {
        clip.rect_filled(
            Rect::from_min_max(
                Pos2::new(x0 + x_of(a), y - h / 2.0),
                Pos2::new(x0 + x_of(b), y + h / 2.0),
            ),
            2.0,
            c.sel,
        );
    }
    if text.is_empty() {
        clip.text(
            Pos2::new(left, y),
            Align2::LEFT_CENTER,
            placeholder,
            font.clone(),
            c.dim,
        );
    } else {
        clip.text(
            Pos2::new(x0, y),
            Align2::LEFT_CENTER,
            text,
            font.clone(),
            c.fg,
        );
    }
    if caret {
        clip.rect_filled(
            Rect::from_min_size(
                Pos2::new(x0 + cx, y - h / 2.0 + 1.0),
                Vec2::new(2.0, h - 2.0),
            ),
            0.0,
            c.accent,
        );
    }
}

impl Gui {
    /// Use an installed font family for monospace text (the editor, inputs); "" = built-in.
    /// The built-in font stays behind it for characters the chosen one lacks.
    fn apply_font(&mut self, ctx: &egui::Context, name: &str) {
        let mut defs = egui::FontDefinitions::default();
        // Codicons (VS Code's icons, CC BY 4.0; credited in Help › Licenses): a family of
        // its own, for the completion-kind icons.
        defs.font_data.insert(
            "codicon".into(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                "../../runtime/fonts/codicon.ttf"
            ))),
        );
        defs.families
            .insert(icon_family(), vec!["codicon".into()]);
        if !name.is_empty() {
            match crate::fonts::load(name) {
                Some((bytes, index)) => {
                    let mut fd = egui::FontData::from_owned(bytes);
                    fd.index = index;
                    defs.font_data
                        .insert("farsub-editor".into(), std::sync::Arc::new(fd));
                    defs.families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .insert(0, "farsub-editor".into());
                }
                None => self.app.msg(format!(
                    "The font \"{name}\" isn't installed or can't be drawn; using the built-in one"
                )),
            }
        }
        ctx.set_fonts(defs);
    }

    /// A command asked for a file or folder: show the system's picker, then install.
    fn answer_file_request(&mut self) {
        use crate::app::FileRequest::*;
        let Some(req) = self.app.file_request.take() else {
            return;
        };
        let (start, name) = self.app.picker_start(req);
        #[cfg(test)]
        let picked = {
            // Tests can't click a native dialog: they set the answer beforehand.
            let _ = (&start, &name);
            self.app.test_pick.take()
        };
        #[cfg(not(test))]
        let picked = {
            let archives = ["zip", "gz", "tgz", "tar", "crate"];
            let dialog = || rfd::FileDialog::new().set_directory(&start);
            let project = ["farsub-project"];
            match req {
                PackageFile => dialog()
                    .set_title("Install a package (archive)")
                    .add_filter("Package archive", &archives)
                    .pick_file(),
                PackageFolder => dialog()
                    .set_title("Install a package (folder)")
                    .pick_folder(),
                GrammarFile => dialog()
                    .set_title("Install a tree-sitter grammar (source archive)")
                    .add_filter("Grammar source archive", &archives)
                    .pick_file(),
                GrammarFolder => dialog()
                    .set_title("Install a tree-sitter grammar (source folder)")
                    .pick_folder(),
                FindFolder => dialog().set_title("Search in folder").pick_folder(),
                OpenFile => dialog().set_title("Open File").pick_file(),
                OpenFolder => dialog().set_title("Open Folder").pick_folder(),
                OpenProject => dialog()
                    .set_title("Open Project")
                    .add_filter("farsub project", &project)
                    .pick_file(),
                AddFolder => dialog().set_title("Add Folder to Project").pick_folder(),
                SaveAs(_) => {
                    let d = dialog().set_title("Save As");
                    match &name {
                        Some(n) => d.set_file_name(n),
                        None => d,
                    }
                    .save_file()
                }
                SaveProjectAs => {
                    let d = dialog()
                        .set_title("Save Project As")
                        .add_filter("farsub project", &project);
                    match &name {
                        Some(n) => d.set_file_name(n),
                        None => d,
                    }
                    .save_file()
                }
            }
        };
        let Some(path) = picked else { return };
        match req {
            PackageFile | PackageFolder => self.app.install_package_from(path),
            GrammarFile | GrammarFolder => self.app.install_grammar_from(path),
            FindFolder => self.app.add_find_folder(path),
            _ => self.app.path_picked(req, &path),
        }
    }
}
