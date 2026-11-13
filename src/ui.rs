//! Rendering: Far-style blue panels, a syntax-highlighted editor, and Sublime-style overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthChar;

use crate::app::{App, Modal, View};
use crate::editor::{Buffer, byte_idx};
use crate::palette::Mode;
use crate::panels::{fmt_time, human_size};

const FAR_BG: Color = Color::Rgb(0, 0, 128);
const FAR_FG: Color = Color::Rgb(0, 255, 255);
const FAR_DIR: Color = Color::Rgb(255, 255, 255);
const FAR_SEL_BG: Color = Color::Rgb(0, 128, 128);
const FAR_MARK: Color = Color::Rgb(255, 255, 0);
const FAR_EXEC: Color = Color::Rgb(0, 255, 0);
const BAR_BG: Color = Color::Rgb(0, 128, 128);
const POP_BG: Color = Color::Rgb(37, 37, 48);
const POP_SEL: Color = Color::Rgb(60, 70, 110);
const DIM: Color = Color::Rgb(140, 140, 150);
const ACCENT: Color = Color::Rgb(255, 200, 60);
const TAB: usize = 4;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.hl_pending = false;
    match app.view {
        View::Panels => draw_panels(f, app, area),
        View::Editor => draw_editor_view(f, app, area),
    }
    match &app.modal {
        Some(Modal::Palette(_)) => draw_palette(f, app, area),
        Some(Modal::Prompt(_)) => draw_prompt(f, app, area),
        Some(Modal::Confirm(_)) => draw_confirm(f, app, area),
        None => {}
    }
}

// ============================================================================ panels

fn draw_panels(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    for i in 0..2 {
        draw_panel(f, app, i, cols[i]);
    }

    // Command line (Far style): "path>" prompt.
    let dir = app.panels[app.active].dir.display().to_string();
    let status = app.status.as_ref().map(|s| s.0.clone());
    let line = if let (Some(s), true) = (status, app.cmdline.is_empty()) {
        Line::from(vec![
            Span::styled(format!("{dir}> "), Style::default().fg(Color::Gray)),
            Span::styled(s, Style::default().fg(ACCENT)),
        ])
    } else {
        Line::from(vec![
            Span::styled(format!("{dir}> "), Style::default().fg(Color::Gray)),
            Span::raw(app.cmdline.clone()),
        ])
    };
    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black)),
        rows[1],
    );
    if app.modal.is_none() {
        let x = rows[1].x + (dir.chars().count() + 2 + app.cmdline.chars().count()) as u16;
        f.set_cursor_position((x.min(rows[1].right().saturating_sub(1)), rows[1].y));
    }

    draw_hints(f, app, rows[2]);
}

fn draw_panel(f: &mut Frame, app: &mut App, idx: usize, area: Rect) {
    let active = idx == app.active;
    let p = &mut app.panels[idx];
    let mut title = p.dir.display().to_string();
    let maxw = area.width.saturating_sub(4) as usize;
    if title.chars().count() > maxw {
        let skip = title.chars().count() - maxw + 1;
        title = format!("…{}", title.chars().skip(skip).collect::<String>());
    }
    let title_style = if active {
        Style::default().fg(Color::Black).bg(FAR_SEL_BG)
    } else {
        Style::default().fg(FAR_FG).bg(FAR_BG)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(FAR_FG).bg(FAR_BG))
        .title(Span::styled(format!(" {title} "), title_style))
        .title_alignment(ratatui::layout::Alignment::Center)
        .style(Style::default().bg(FAR_BG));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let list_h = inner.height as usize - 2;
    app.panel_page = list_h.saturating_sub(1);
    let p = &mut app.panels[idx];
    if p.sel < p.scroll {
        p.scroll = p.sel;
    } else if p.sel >= p.scroll + list_h {
        p.scroll = p.sel + 1 - list_h;
    }
    let w = inner.width as usize;
    let show_date = w >= 44;
    let size_w = 9;
    let date_w = if show_date { 17 } else { 0 };
    let name_w = w.saturating_sub(size_w + date_w + 1);

    let header = format!(
        "{:<name_w$}{:>size_w$}{}",
        "Name",
        "Size",
        if show_date {
            format!("{:>date_w$}", "Modified")
        } else {
            String::new()
        }
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            header,
            Style::default().fg(FAR_MARK).add_modifier(Modifier::BOLD),
        )),
        Rect { height: 1, ..inner },
    );

    let mut lines = vec![];
    for (i, e) in p
        .entries
        .iter()
        .enumerate()
        .skip(p.scroll)
        .take(list_h.saturating_sub(1))
    {
        let marked = p.marked.contains(&e.path);
        let mut fg = if e.is_dir {
            FAR_DIR
        } else if e.exec {
            FAR_EXEC
        } else {
            FAR_FG
        };
        if marked {
            fg = FAR_MARK;
        }
        let mut st = Style::default().fg(fg);
        if e.is_dir {
            st = st.add_modifier(Modifier::BOLD);
        }
        if i == p.sel && active {
            st = st
                .bg(FAR_SEL_BG)
                .fg(if marked { FAR_MARK } else { Color::Black });
        }
        let mut name = e.name.clone();
        if e.is_link {
            name.push('@');
        }
        let name: String = if name.chars().count() > name_w.saturating_sub(1) {
            let mut s: String = name.chars().take(name_w.saturating_sub(2)).collect();
            s.push('…');
            s
        } else {
            name
        };
        let size = if e.name == ".." {
            "Up".to_string()
        } else if e.is_dir {
            "<DIR>".to_string()
        } else {
            human_size(e.size)
        };
        let date = if show_date {
            format!("{:>date_w$}", fmt_time(e.mtime))
        } else {
            String::new()
        };
        let mark = if marked { "√" } else { " " };
        lines.push(Line::from(Span::styled(
            format!(
                "{mark}{:<nw$}{:>size_w$}{date}",
                name,
                size,
                nw = name_w.saturating_sub(1)
            ),
            st,
        )));
    }
    if let Some(err) = &p.error {
        lines.push(Line::from(Span::styled(
            format!(" {err}"),
            Style::default().fg(Color::LightRed),
        )));
    }
    f.render_widget(
        Paragraph::new(lines),
        Rect {
            y: inner.y + 1,
            height: list_h.saturating_sub(1) as u16,
            ..inner
        },
    );

    // Footer: separator + info about current entry.
    let sep_y = inner.y + inner.height - 2;
    f.render_widget(
        Paragraph::new("─".repeat(w)).style(Style::default().fg(FAR_FG)),
        Rect {
            y: sep_y,
            height: 1,
            ..inner
        },
    );
    let info = if !p.marked.is_empty() {
        let total: u64 = p
            .entries
            .iter()
            .filter(|e| p.marked.contains(&e.path))
            .map(|e| e.size)
            .sum();
        format!(" {} marked, {}", p.marked.len(), human_size(total))
    } else if let Some(e) = p.current() {
        let n = p.entries.iter().filter(|e| e.name != "..").count();
        format!(" {}   ({} items)", e.name, n)
    } else {
        String::new()
    };
    f.render_widget(
        Paragraph::new(Span::styled(info, Style::default().fg(FAR_FG))),
        Rect {
            y: sep_y + 1,
            height: 1,
            ..inner
        },
    );
}

// ============================================================================ editor

pub fn display_col(line: &str, col: usize) -> usize {
    let mut w = 0;
    for c in line.chars().take(col) {
        w += char_width(c, w);
    }
    w
}

fn char_width(c: char, at: usize) -> usize {
    if c == '\t' {
        TAB - at % TAB
    } else {
        c.width().unwrap_or(1).max(1)
    }
}

fn draw_editor_view(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let (mut top, mut body) = (rows[0], rows[1]);
    if app.tree_visible && area.width >= 40 {
        let sw = (area.width / 4).clamp(18, 36);
        let split = |r: Rect| {
            let c = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(sw), Constraint::Min(1)])
                .split(r);
            (c[0], c[1])
        };
        let (st, mt) = split(rows[0]);
        let (sb, mb) = split(rows[1]);
        draw_sidebar(f, app, st, sb);
        top = mt;
        body = mb;
    }
    draw_tabs(f, app, top);
    if app.buffers.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from("  No open files."),
            Line::from(""),
            Line::from("  Ctrl/Cmd+P        open file (fuzzy)"),
            Line::from("  Ctrl/Cmd+Shift+P  command palette (also F1)"),
            Line::from("  Ctrl/Cmd+B        file tree sidebar"),
            Line::from("  Ctrl/Cmd+O        back to file panels"),
        ])
        .style(Style::default().bg(app.syn.bg()).fg(app.syn.fg()));
        f.render_widget(p, body);
        f.render_widget(
            Paragraph::new("").style(Style::default().bg(STATUS_BG)),
            rows[2],
        );
    } else {
        draw_editor(f, app, body);
        draw_status(f, app, rows[2]);
    }
    draw_hints(f, app, rows[3]);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    const TAB_BG: Color = Color::Rgb(25, 25, 25);
    let clock = format!(" {} ", chrono::Local::now().format("%H:%M"));
    let clock_w = clock.chars().count();
    let avail = (area.width as usize).saturating_sub(clock_w);
    let tabs: Vec<(String, Style)> = app
        .buffers
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let name = format!(" {}{} ", b.name(), if b.modified { " ●" } else { "" });
            let st = if i == app.cur {
                Style::default()
                    .fg(Color::White)
                    .bg(app.syn.bg())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM).bg(TAB_BG)
            };
            (name, st)
        })
        .collect();
    // Drop tabs from the left until the current one fits.
    let width = |t: &[(String, Style)]| t.iter().map(|x| x.0.chars().count() + 1).sum::<usize>();
    let mut first = 0;
    while first < app.cur && width(&tabs[first..=app.cur]) > avail {
        first += 1;
    }
    let mut spans = vec![];
    let mut used = 0;
    if first > 0 {
        spans.push(Span::styled("‹", Style::default().fg(DIM).bg(TAB_BG)));
        used += 1;
    }
    for (name, st) in &tabs[first..] {
        let w = name.chars().count() + 1;
        if used + w > avail {
            break;
        }
        spans.push(Span::styled(name.clone(), *st));
        spans.push(Span::styled(
            "│",
            Style::default().fg(Color::Rgb(60, 60, 60)).bg(TAB_BG),
        ));
        used += w;
    }
    spans.push(Span::styled(
        " ".repeat(avail.saturating_sub(used)),
        Style::default().bg(TAB_BG),
    ));
    spans.push(Span::styled(
        clock,
        Style::default()
            .fg(Color::Black)
            .bg(BAR_BG)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(TAB_BG)),
        area,
    );
}

const SIDEBAR_BG: Color = Color::Rgb(30, 31, 36);
const STATUS_BG: Color = Color::Rgb(0, 95, 135);

fn draw_sidebar(f: &mut Frame, app: &mut App, top: Rect, body: Rect) {
    let focused = app.focus_tree && app.modal.is_none();
    let current = app.buf().and_then(|b| b.path.clone());
    let t = &mut app.tree;
    let root = t
        .root
        .file_name()
        .map(|s| s.to_string_lossy().to_uppercase())
        .unwrap_or_default();
    let w = top.width as usize;
    let head = format!(" ▾ {root}");
    let head: String = head.chars().take(w.saturating_sub(1)).collect();
    let pad = w.saturating_sub(head.chars().count() + 1);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                head,
                Style::default()
                    .fg(if focused { ACCENT } else { DIM })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled("│", Style::default().fg(Color::Rgb(55, 55, 62))),
        ]))
        .style(Style::default().bg(SIDEBAR_BG)),
        top,
    );
    let h = body.height as usize;
    if t.sel < t.scroll {
        t.scroll = t.sel;
    } else if t.sel >= t.scroll + h {
        t.scroll = t.sel + 1 - h;
    }
    let inner_w = w.saturating_sub(1);
    let mut lines = vec![];
    for row in 0..h {
        let i = t.scroll + row;
        let mut spans = vec![];
        let mut text_w = 0;
        if let Some(n) = t.nodes.get(i) {
            let marker = if n.is_dir {
                if t.expanded.contains(&n.path) {
                    "▾ "
                } else {
                    "▸ "
                }
            } else {
                "  "
            };
            let label = format!(" {}{}{}", "  ".repeat(n.depth), marker, n.name);
            let mut label: String = label.chars().take(inner_w).collect();
            if label.chars().count() == inner_w && n.name.chars().count() > 1 {
                label.pop();
                label.push('…');
            }
            text_w = label.chars().count();
            let is_cur = current.as_deref() == Some(n.path.as_path());
            let mut st = Style::default().fg(if n.is_dir {
                Color::Rgb(205, 205, 215)
            } else {
                Color::Rgb(170, 170, 175)
            });
            if n.is_dir {
                st = st.add_modifier(Modifier::BOLD);
            }
            if is_cur {
                st = st.fg(ACCENT);
            }
            let bg = if i == t.sel {
                if focused {
                    POP_SEL
                } else {
                    Color::Rgb(48, 50, 58)
                }
            } else {
                SIDEBAR_BG
            };
            spans.push(Span::styled(label, st.bg(bg)));
            spans.push(Span::styled(
                " ".repeat(inner_w - text_w),
                Style::default().bg(bg),
            ));
            text_w = inner_w;
        }
        if text_w < inner_w {
            spans.push(Span::raw(" ".repeat(inner_w - text_w)));
        }
        spans.push(Span::styled(
            "│",
            Style::default().fg(Color::Rgb(55, 55, 62)),
        ));
        lines.push(Line::from(spans));
    }
    if t.nodes.is_empty() && h > 0 {
        lines[0] = Line::from(Span::styled(" (empty)", Style::default().fg(DIM)));
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(SIDEBAR_BG)),
        body,
    );
}

fn draw_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let syn = &app.syn;
    let (bg, fg, sel_bg, cur_bg, gut) = (
        syn.bg(),
        syn.fg(),
        syn.selection(),
        syn.line_highlight(),
        syn.gutter(),
    );
    let h = area.height as usize;
    app.page = h.saturating_sub(2).max(1);
    let search = app.search.clone().filter(|s| !s.is_empty());
    let path = app.buffers[app.cur].path.clone();
    let empty = Vec::new();
    let diags: &Vec<crate::app::Diag> = path
        .as_ref()
        .and_then(|p| app.diagnostics.get(p))
        .unwrap_or(&empty);
    let b: &mut Buffer = &mut app.buffers[app.cur];

    let digits = b.lines.len().to_string().len().max(3);
    let gutter_w = digits + 3;
    let text_w = (area.width as usize).saturating_sub(gutter_w).max(1);

    // Keep cursor visible.
    if b.cursor.line < b.scroll_y {
        b.scroll_y = b.cursor.line;
    } else if b.cursor.line >= b.scroll_y + h {
        b.scroll_y = b.cursor.line + 1 - h;
    }
    b.scroll_y = b.scroll_y.min(b.lines.len().saturating_sub(1));
    let cur_dcol = display_col(&b.lines[b.cursor.line], b.cursor.col);
    if cur_dcol < b.scroll_x {
        b.scroll_x = cur_dcol;
    } else if cur_dcol >= b.scroll_x + text_w {
        b.scroll_x = cur_dcol + 1 - text_w;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(12);
    let hl_done = b.hl.ensure(&app.syn, &b.lines, b.scroll_y + h, deadline);

    let sel = b.selection();
    let mut out: Vec<Line> = Vec::with_capacity(h);
    for row in 0..h {
        let ln = b.scroll_y + row;
        if ln >= b.lines.len() {
            out.push(Line::from(Span::styled(
                " ".repeat(area.width as usize),
                Style::default().bg(bg),
            )));
            continue;
        }
        let is_cur = ln == b.cursor.line;
        let line_bg = if is_cur { cur_bg } else { bg };
        let line = &b.lines[ln];

        // Gutter: diagnostic marker + line number.
        let worst = diags
            .iter()
            .filter(|d| d.start.line <= ln && d.end.line >= ln)
            .map(|d| d.severity)
            .min();
        let (mark, mark_st) = match worst {
            Some(1) => ("●", Style::default().fg(Color::Rgb(240, 80, 80))),
            Some(2) => ("●", Style::default().fg(Color::Rgb(230, 190, 60))),
            Some(_) => ("·", Style::default().fg(Color::Rgb(100, 160, 230))),
            None => (" ", Style::default()),
        };
        let mut spans: Vec<Span> = vec![
            Span::styled(mark, mark_st.bg(bg)),
            Span::styled(
                format!("{:>digits$}  ", ln + 1),
                Style::default().fg(if is_cur { fg } else { gut }).bg(bg),
            ),
        ];

        let hl = b.hl.lines.get(ln);
        let matches = search
            .as_ref()
            .map(|s| Buffer::matches_in_line(line, s))
            .unwrap_or_default();
        let line_diags: Vec<(usize, usize, u8)> = diags
            .iter()
            .filter(|d| d.start.line <= ln && d.end.line >= ln)
            .map(|d| {
                let s = if d.start.line == ln {
                    byte_idx(line, d.start.col)
                } else {
                    0
                };
                let mut e = if d.end.line == ln {
                    byte_idx(line, d.end.col)
                } else {
                    line.len()
                };
                if e <= s {
                    e = (s + 1).min(line.len().max(s + 1));
                }
                (s, e, d.severity)
            })
            .collect();
        let (sel_s, sel_e) = match sel {
            Some((a, z)) if a.line <= ln && z.line >= ln => (
                if a.line == ln {
                    byte_idx(line, a.col)
                } else {
                    0
                },
                if z.line == ln {
                    byte_idx(line, z.col)
                } else {
                    usize::MAX
                },
            ),
            _ => (usize::MAX, usize::MAX),
        };

        let mut dcol = 0usize;
        let mut span_i = 0usize;
        let mut cur_text = String::new();
        let mut cur_style: Option<Style> = None;
        let flush = |text: &mut String, st: &mut Option<Style>, spans: &mut Vec<Span>| {
            if let Some(s) = st.take()
                && !text.is_empty()
            {
                spans.push(Span::styled(std::mem::take(text), s));
            }
        };
        for (bi, c) in line.char_indices() {
            let w = char_width(c, dcol);
            let start = dcol;
            dcol += w;
            if dcol <= b.scroll_x {
                continue;
            }
            if start >= b.scroll_x + text_w {
                break;
            }
            let mut st = Style::default().fg(fg).bg(line_bg);
            if let Some(spans_hl) = hl {
                while span_i < spans_hl.len() && spans_hl[span_i].1 <= bi {
                    span_i += 1;
                }
                if let Some(&(s, e, hst)) = spans_hl.get(span_i)
                    && s <= bi
                    && bi < e
                {
                    st = st.patch(hst);
                }
            }
            if matches.iter().any(|&(s, e)| s <= bi && bi < e) {
                st = st.bg(Color::Rgb(90, 80, 30));
            }
            if bi >= sel_s && bi < sel_e {
                st = st.bg(sel_bg);
            }
            if let Some(&(_, _, sev)) = line_diags.iter().find(|&&(s, e, _)| s <= bi && bi < e) {
                let col = match sev {
                    1 => Color::Rgb(240, 80, 80),
                    2 => Color::Rgb(230, 190, 60),
                    _ => Color::Rgb(100, 160, 230),
                };
                st = st.add_modifier(Modifier::UNDERLINED).underline_color(col);
            }
            if cur_style != Some(st) {
                flush(&mut cur_text, &mut cur_style, &mut spans);
                cur_style = Some(st);
            }
            // Partially-visible wide char / tab at the left edge.
            let visible_w = dcol - start.max(b.scroll_x);
            if c == '\t' || start < b.scroll_x {
                cur_text.push_str(&" ".repeat(visible_w));
            } else if c.is_control() {
                cur_text.push('?');
            } else {
                cur_text.push(c);
            }
        }
        flush(&mut cur_text, &mut cur_style, &mut spans);
        let used = dcol.saturating_sub(b.scroll_x).min(text_w);
        let mut rest = text_w - used;
        // Show selected newline as one highlighted cell.
        if sel_e == usize::MAX && sel_s != usize::MAX && rest > 0 {
            spans.push(Span::styled(" ", Style::default().bg(sel_bg)));
            rest -= 1;
        }
        spans.push(Span::styled(" ".repeat(rest), Style::default().bg(line_bg)));
        out.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(out).style(Style::default().bg(bg)), area);

    let cx = area.x + (gutter_w + cur_dcol - b.scroll_x) as u16;
    let cy = area.y + (b.cursor.line - b.scroll_y) as u16;
    if app.modal.is_none() && !(app.focus_tree && app.tree_visible) {
        f.set_cursor_position((cx, cy));
    }
    let cursor_screen = (cx, cy);
    app.hl_pending = !hl_done;
    draw_completion(f, app, area, cursor_screen);
    draw_hover(f, app, area, cursor_screen);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let Some(b) = app.buf() else { return };
    let diags = b.path.as_ref().and_then(|p| app.diagnostics.get(p));
    let (errs, warns) = diags
        .map(|d| {
            (
                d.iter().filter(|x| x.severity == 1).count(),
                d.iter().filter(|x| x.severity == 2).count(),
            )
        })
        .unwrap_or((0, 0));
    let msg = app.status.as_ref().map(|s| s.0.clone()).or_else(|| {
        diags.and_then(|d| {
            d.iter()
                .filter(|x| x.start.line <= b.cursor.line && x.end.line >= b.cursor.line)
                .min_by_key(|x| x.severity)
                .map(|x| x.message.lines().next().unwrap_or("").to_string())
        })
    });
    let sel = b
        .selection()
        .map(|(a, z)| format!(" ({} sel)", b.text_range(a, z).chars().count()))
        .unwrap_or_default();
    let lsp = app.lsp_status();
    let right = format!(
        "{}  E:{errs} W:{warns}  {}  {} ",
        lsp,
        b.hl.syntax,
        if b.indent_unit == "\t" {
            "Tabs".to_string()
        } else {
            format!("Spaces: {}", b.indent_unit.len())
        }
    );
    let tail = format!(
        "{}{}  Ln {}, Col {}{}",
        if b.modified { " ●" } else { "" },
        if b.read_only { " [view]" } else { "" },
        b.cursor.line + 1,
        b.cursor.col + 1,
        sel
    );
    let mut path = b
        .path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "untitled".into());
    if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned())
        && let Some(rest) = path.strip_prefix(&home)
    {
        path = format!("~{rest}");
    }
    let max_path = (area.width as usize)
        .saturating_sub(right.chars().count() + tail.chars().count() + 2)
        .max(8);
    let n = path.chars().count();
    if n > max_path {
        path = format!(
            "…{}",
            path.chars().skip(n + 1 - max_path).collect::<String>()
        );
    }
    let left = format!(" {path}{tail}");
    let mut spans = vec![Span::styled(
        left.clone(),
        Style::default().fg(Color::White),
    )];
    let used = left.chars().count() + right.chars().count();
    let room = (area.width as usize).saturating_sub(used);
    if let Some(m) = msg {
        let m: String = format!("   {m}").chars().take(room).collect();
        let n = m.chars().count();
        spans.push(Span::styled(m, Style::default().fg(ACCENT)));
        spans.push(Span::raw(" ".repeat(room - n)));
    } else {
        spans.push(Span::raw(" ".repeat(room)));
    }
    spans.push(Span::styled(
        right,
        Style::default().fg(Color::Rgb(220, 220, 220)),
    ));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(STATUS_BG)),
        area,
    );
}

// ============================================================================ popups

fn popup_rect(area: Rect, cursor: (u16, u16), w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let below = area.bottom().saturating_sub(cursor.1 + 1);
    let above = cursor.1.saturating_sub(area.y);
    let (y, h) = if below >= h || below >= above {
        (cursor.1 + 1, h.min(below))
    } else {
        let h = h.min(above);
        (cursor.1 - h, h)
    };
    let x = cursor.0.min(area.right().saturating_sub(w));
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn draw_completion(f: &mut Frame, app: &App, area: Rect, cursor: (u16, u16)) {
    let Some(c) = &app.completion else { return };
    if c.shown.is_empty() {
        return;
    }
    let h = c.shown.len().min(10) as u16;
    let label_w = c
        .shown
        .iter()
        .map(|&i| c.items[i].label.chars().count())
        .max()
        .unwrap_or(10)
        .min(40);
    let w = (label_w + 34).min(70) as u16;
    let start_x = cursor
        .0
        .saturating_sub((app.buf().map(|b| b.cursor.col - c.start.col).unwrap_or(0)) as u16 + 1);
    let r = popup_rect(area, (start_x, cursor.1), w, h);
    if r.height == 0 {
        return;
    }
    let first = c.sel.saturating_sub(r.height as usize - 1);
    let lines: Vec<Line> = c
        .shown
        .iter()
        .enumerate()
        .skip(first)
        .take(r.height as usize)
        .map(|(i, &idx)| {
            let it = &c.items[idx];
            let bg = if i == c.sel { POP_SEL } else { POP_BG };
            let label: String = it.label.chars().take(label_w).collect();
            let detail_w = (w as usize).saturating_sub(label_w + 8);
            let detail: String = it.detail.chars().take(detail_w).collect();
            Line::from(vec![
                Span::styled(
                    format!(" {:<4} ", it.kind),
                    Style::default().fg(Color::Rgb(120, 180, 250)).bg(bg),
                ),
                Span::styled(
                    format!("{label:<label_w$} "),
                    Style::default().fg(Color::White).bg(bg),
                ),
                Span::styled(
                    format!("{detail:<detail_w$}"),
                    Style::default().fg(DIM).bg(bg),
                ),
            ])
        })
        .collect();
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).style(Style::default().bg(POP_BG)), r);
}

fn draw_hover(f: &mut Frame, app: &App, area: Rect, cursor: (u16, u16)) {
    let Some(hv) = &app.hover else { return };
    let w = hv
        .lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(20)
        .clamp(20, 90) as u16
        + 4;
    let h = (hv.lines.len() as u16 + 2).min(18);
    let r = popup_rect(area, cursor, w, h);
    if r.height < 3 {
        return;
    }
    let lines: Vec<Line> = hv.lines.iter().map(|l| Line::from(l.clone())).collect();
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(DIM)),
            )
            .style(Style::default().bg(POP_BG).fg(Color::Rgb(220, 220, 220))),
        r,
    );
}

fn centered(area: Rect, w: u16, h: u16, top: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(top));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + top,
        width: w,
        height: h,
    }
}

fn draw_palette(f: &mut Frame, app: &App, area: Rect) {
    let Some(Modal::Palette(p)) = &app.modal else {
        return;
    };
    let w = (area.width * 2 / 3).clamp(40, 100);
    let list_h = (area.height as usize).saturating_sub(8).min(16);
    let r = centered(area, w, list_h as u16 + 4, 1);
    let title = match p.mode {
        Mode::Files => "Goto Anything",
        Mode::Commands => "Command Palette",
        Mode::Symbols => "Goto Symbol",
        Mode::Line => "Goto Line",
        Mode::WorkspaceSymbols => "Goto Symbol in Project",
        Mode::Buffers => "Open Files",
        Mode::Locations => "Locations",
    };
    let sub = if p.loading {
        format!("{} — loading…", p.title)
    } else {
        p.title.clone()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(90, 100, 140)))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            format!(" {} ", sub.chars().take(w as usize - 4).collect::<String>()),
            Style::default().fg(DIM),
        ))
        .style(Style::default().bg(POP_BG));
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);

    let input = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(ACCENT)),
        Span::styled(p.query.clone(), Style::default().fg(Color::White)),
    ]);
    f.render_widget(Paragraph::new(input), Rect { height: 1, ..inner });
    f.set_cursor_position((inner.x + 2 + p.query.chars().count() as u16, inner.y));
    f.render_widget(
        Paragraph::new("─".repeat(inner.width as usize))
            .style(Style::default().fg(Color::Rgb(60, 60, 80))),
        Rect {
            y: inner.y + 1,
            height: 1,
            ..inner
        },
    );

    let list = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let n = list.height as usize;
    let first = p.sel.saturating_sub(n.saturating_sub(1));
    let iw = inner.width as usize;
    let mut lines = vec![];
    if p.mode == Mode::Line {
        let total = app.buf().map(|b| b.lines.len()).unwrap_or(0);
        lines.push(Line::from(Span::styled(
            format!(" Type a line number between 1 and {total}, then Enter"),
            Style::default().fg(DIM),
        )));
    } else if p.shown.is_empty() && !p.loading {
        lines.push(Line::from(Span::styled(
            " No matches",
            Style::default().fg(DIM),
        )));
    }
    for (i, (idx, hits)) in p.shown.iter().enumerate().skip(first).take(n) {
        let it = &p.items[*idx];
        let bg = if i == p.sel { POP_SEL } else { POP_BG };
        let detail_w = it.detail.chars().count().min(iw / 2);
        let label_w = iw.saturating_sub(detail_w + 3);
        let mut spans = vec![Span::styled(" ", Style::default().bg(bg))];
        let mut used = 1;
        for (ci, ch) in it.label.chars().enumerate() {
            if used >= label_w {
                break;
            }
            let st = if hits.contains(&ci) {
                Style::default()
                    .fg(ACCENT)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White).bg(bg)
            };
            spans.push(Span::styled(ch.to_string(), st));
            used += 1;
        }
        let pad = iw.saturating_sub(used + detail_w + 1);
        spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
        let detail: String = it.detail.chars().take(detail_w).collect();
        spans.push(Span::styled(
            format!("{detail} "),
            Style::default().fg(DIM).bg(bg),
        ));
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), list);
}

fn draw_prompt(f: &mut Frame, app: &App, area: Rect) {
    let Some(Modal::Prompt(p)) = &app.modal else {
        return;
    };
    let r = centered(area, 70, 3, area.height / 3);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::White))
        .title(Span::styled(
            format!(" {} ", p.title),
            Style::default().fg(Color::White),
        ))
        .style(Style::default().bg(FAR_SEL_BG));
    let inner = block.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(block, r);
    let iw = inner.width as usize;
    let skip = p.cursor.saturating_sub(iw.saturating_sub(2));
    let shown: String = p.input.chars().skip(skip).take(iw).collect();
    f.render_widget(
        Paragraph::new(shown).style(Style::default().fg(Color::Black).bg(FAR_FG)),
        inner,
    );
    f.set_cursor_position((inner.x + (p.cursor - skip) as u16, inner.y));
}

fn draw_confirm(f: &mut Frame, app: &App, area: Rect) {
    let Some(Modal::Confirm(c)) = &app.modal else {
        return;
    };
    let w = (c.message.chars().count() as u16 + 6).clamp(30, area.width.saturating_sub(4));
    let r = centered(area, w, 5, area.height / 3);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::White))
        .title(" Confirm ")
        .style(Style::default().bg(Color::Rgb(128, 0, 0)).fg(Color::White));
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(c.message.clone()),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "[Y]",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw("es    "),
                Span::styled(
                    "[N]",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw("o"),
            ]),
        ])
        .alignment(ratatui::layout::Alignment::Center)
        .block(block),
        r,
    );
}

// ============================================================================ hint bar

fn hints(app: &App) -> Vec<(&'static str, &'static str)> {
    match &app.modal {
        Some(Modal::Palette(p)) => match p.mode {
            Mode::Files => vec![
                ("Enter", "Open"),
                ("↑↓", "Move"),
                ("Esc", "Cancel"),
                (">", "Commands"),
                ("@", "Symbols"),
                (":", "Line"),
                ("#", "Project symbols"),
                ("name:42", "Open at line"),
            ],
            Mode::Commands => vec![
                ("Enter", "Run"),
                ("↑↓", "Move"),
                ("Esc", "Cancel"),
                ("⌫", "Back to files"),
            ],
            Mode::Symbols => vec![
                ("Enter", "Go"),
                ("↑↓", "Preview"),
                ("Esc", "Back"),
                (">", "Commands"),
                (":", "Line"),
            ],
            Mode::Line => vec![("Enter", "Go"), ("Esc", "Back"), ("@", "Symbols")],
            Mode::WorkspaceSymbols => vec![("Enter", "Open"), ("↑↓", "Move"), ("Esc", "Cancel")],
            Mode::Buffers | Mode::Locations => {
                vec![("Enter", "Open"), ("↑↓", "Move"), ("Esc", "Cancel")]
            }
        },
        Some(Modal::Prompt(p)) => match p.kind {
            crate::app::PromptKind::Find { .. } => {
                vec![
                    ("Enter", "Done"),
                    ("F3 ↓", "Next"),
                    ("↑", "Previous"),
                    ("Esc", "Cancel"),
                ]
            }
            _ => vec![("Enter", "OK"), ("Esc", "Cancel"), ("^U", "Clear")],
        },
        Some(Modal::Confirm(_)) => vec![("Y", "Yes"), ("N", "No"), ("Esc", "Cancel")],
        None => match app.view {
            View::Panels => vec![
                ("1", "Cmds"),
                ("3", "View"),
                ("4", "Edit"),
                ("5", "Copy"),
                ("6", "RenMov"),
                ("7", "MkFold"),
                ("8", "Delete"),
                ("10", "Quit"),
                ("^P", "Find file"),
                ("^O", "Editor"),
                ("^B", "Tree"),
                ("Ins", "Mark"),
                ("^H", "Hidden"),
                ("^U", "Swap"),
            ],
            View::Editor if app.completion.is_some() => {
                vec![
                    ("Enter", "Accept"),
                    ("Tab", "Accept"),
                    ("↑↓", "Select"),
                    ("Esc", "Close"),
                ]
            }
            View::Editor if app.tree_visible && app.focus_tree => vec![
                ("↑↓", "Move"),
                ("←→", "Fold"),
                ("Enter", "Open"),
                ("Space", "Preview"),
                ("Esc", "Editor"),
                ("^B", "Hide"),
                ("^H", "Hidden"),
                ("a-z", "Jump"),
            ],
            View::Editor => {
                let lsp = app.buf().is_some_and(|b| b.lsp_client.is_some());
                let mut v = vec![
                    ("^P", "Files"),
                    ("^⇧P", "Cmds"),
                    ("^R", "Symbol"),
                    ("^S", "Save"),
                    ("^F", "Find"),
                    ("^D", "Next match"),
                    ("^/", "Comment"),
                ];
                if lsp {
                    v.extend([
                        ("^Space", "Complete"),
                        ("F12", "Definition"),
                        ("^K", "Hover"),
                        ("F2", "Rename"),
                    ]);
                }
                v.extend([
                    ("^B", "Tree"),
                    ("^E", "Open files"),
                    ("^W", "Close"),
                    ("^O", "Panels"),
                ]);
                v
            }
        },
    }
}

fn draw_hints(f: &mut Frame, app: &App, area: Rect) {
    let key_st = Style::default()
        .fg(Color::White)
        .bg(Color::Black)
        .add_modifier(Modifier::BOLD);
    let label_st = Style::default().fg(Color::Black).bg(BAR_BG);
    let width = area.width as usize;
    let mut spans = vec![];
    let mut used = 0;
    for (k, l) in hints(app) {
        let w = k.chars().count() + l.chars().count() + 2;
        if used + w > width {
            break;
        }
        spans.push(Span::styled(k, key_st));
        spans.push(Span::styled(format!("{l} "), label_st));
        spans.push(Span::styled(" ", Style::default().bg(Color::Black)));
        used += w;
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Black)),
        area,
    );
}
