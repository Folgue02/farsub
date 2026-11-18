//! GUI: the alarm, pomodoro and clock at the right end of the tab strip, and their
//! setup dialog.
//!
//! The widgets are slanted segments, left to right: alarm, pomodoro, clock. One that
//! isn't in use is a thin empty stripe; in use, it shows its time. Hover highlights a
//! segment; a click opens its dialog (or stops it flashing after an alert).
//!
//! Theme `[editor]` colors: `timer_background`, `clock`, `pomodoro`, `pomodoro_break`,
//! `alarm`, `timer_alert`.

use super::*;
use crate::app::{FieldKind, TimerKind};
use crate::timers::{Phase, Run};

/// Unix time in milliseconds (the clock's second/minute boundaries).
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A widget: which one, its text and color (None: an empty stripe), and whether it's
/// flashing.
type Segment = (TimerKind, Option<(String, Color32)>, bool);

struct TimerColors {
    bg: Color32,
    clock: Color32,
    work: Color32,
    rest: Color32,
    alarm: Color32,
    alert: Color32,
}

impl Gui {
    fn timer_colors(&self, c: &Palette) -> TimerColors {
        let key = |k: &str| self.app.syn.editor_color(k);
        TimerColors {
            bg: key("timer_background").unwrap_or(mix(c.chrome, c.fg, 0.06)),
            clock: key("clock").unwrap_or(c.fg),
            work: key("pomodoro").unwrap_or(c.accent),
            rest: key("pomodoro_break").unwrap_or(Color32::from_rgb(0x7F, 0xD1, 0x7A)),
            alarm: key("alarm").unwrap_or(c.info),
            alert: key("timer_alert").unwrap_or(c.err),
        }
    }

    /// The widgets at the right of `r` (a tab strip). Called twice a frame: with
    /// `paint` false before the tabs, to learn where they start (the x of the slanted
    /// edge at the top, and the slant), then with `paint` true after the tabs, so they
    /// cover the tabs that slide under them and take the clicks there.
    pub(super) fn draw_timers(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        r: Rect,
        c: &Palette,
        paint: bool,
    ) -> Option<(f32, f32)> {
        if r.width() < 320.0 {
            return None;
        }
        let tc = self.timer_colors(c);
        let ms = now_ms();
        let now = ms.div_euclid(1000);
        let t = &self.app.timers;
        let font = FontId::monospace(13.0);
        let blink = (ms / 500) % 2 == 0;
        // (kind, text, color, flashing)
        let pomo = t.pomodoro.remaining(now).map(|left| {
            let col = if t.pomodoro.phase == Phase::Work {
                tc.work
            } else {
                tc.rest
            };
            let col = if matches!(t.pomodoro.run, Run::Running { .. }) {
                col
            } else {
                mix(col, tc.bg, 0.45)
            };
            (crate::timers::countdown(left), col)
        });
        let alarm = t.alarm.at.map(|_| (t.alarm.time.clone(), tc.alarm));
        let segs: [Segment; 3] = [
            (TimerKind::Alarm, alarm, t.flash.1),
            (TimerKind::Pomodoro, pomo, t.flash.0),
            (
                TimerKind::Clock,
                Some((t.clock.format(now), tc.clock)),
                false,
            ),
        ];
        let h = r.height();
        let s = (h * 0.34).round(); // the slant
        let pad = 11.0;
        let gap = 3.0;
        let stripe = 7.0;
        let text_w = |txt: &str| {
            ui.ctx()
                .fonts_mut(|f| f.layout_no_wrap(txt.to_string(), font.clone(), c.fg).size().x)
        };
        // Lay out from the right: (kind, top-left x, top width (clock: to the edge)).
        let mut lay: Vec<(usize, f32, f32)> = vec![];
        let mut right = r.right();
        for (i, (kind, content, _)) in segs.iter().enumerate().rev() {
            let w = match content {
                Some((txt, _)) => text_w(txt) + 2.0 * pad,
                None => stripe,
            };
            let left = if *kind == TimerKind::Clock {
                right - w - s / 2.0
            } else {
                right - w
            };
            lay.push((i, left, right - left));
            right = left - gap;
        }
        if !paint {
            return Some((right, s));
        }
        // The strip's background behind the widgets, with the same slant on its left
        // edge: tabs slide under it along the diagonal (not a straight cut).
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(right, r.top()),
                Pos2::new(r.right(), r.top()),
                r.right_bottom(),
                Pos2::new(right + s, r.bottom()),
            ],
            c.chrome,
            Stroke::NONE,
        ));
        // They take the mouse from the middle of the slant, where the tabs stop.
        let area = Rect::from_min_max(Pos2::new(right + s / 2.0, r.top()), r.max);
        let resp = ui.interact(area, Id::unique("timer-widgets"), Sense::click());
        let hover = resp.hover_pos();
        let inside = |left: f32, w: f32, clock: bool, pt: Pos2| {
            let dx = s * (pt.y - r.top()) / h;
            pt.y >= r.top()
                && pt.y <= r.bottom()
                && pt.x >= left + dx
                && (clock || pt.x <= left + w + dx)
        };
        let clip = p.with_clip_rect(r);
        let mut clicked = None;
        for &(i, left, w) in &lay {
            let (kind, content, flashing) = &segs[i];
            let clock = *kind == TimerKind::Clock;
            let right_top = if clock { r.right() + s } else { left + w };
            let pts = vec![
                Pos2::new(left, r.top()),
                Pos2::new(right_top, r.top()),
                Pos2::new(right_top + s, r.bottom()),
                Pos2::new(left + s, r.bottom()),
            ];
            let hovered = hover.is_some_and(|pt| inside(left, w, clock, pt));
            let mut fill = if hovered {
                mix(tc.bg, c.fg, 0.10)
            } else {
                tc.bg
            };
            if *flashing && blink {
                fill = mix(tc.bg, tc.alert, 0.55);
            }
            clip.add(egui::Shape::convex_polygon(
                pts,
                fill,
                Stroke::new(1.0, mix(tc.bg, c.fg, if hovered { 0.30 } else { 0.16 })),
            ));
            if let Some((txt, col)) = content {
                let mid = if clock {
                    (left + s / 2.0 + r.right()) / 2.0
                } else {
                    left + w / 2.0 + s / 2.0
                };
                let col = if *flashing && blink { c.fg } else { *col };
                clip.text(
                    Pos2::new(mid, r.center().y),
                    Align2::CENTER_CENTER,
                    txt,
                    font.clone(),
                    col,
                );
            }
            if hovered {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    clicked = Some(*kind);
                }
            }
        }
        let hovered_kind = hover.and_then(|pt| {
            lay.iter()
                .find(|&&(i, l, w)| inside(l, w, segs[i].0 == TimerKind::Clock, pt))
                .map(|&(i, _, _)| segs[i].0)
        });
        if let Some(k) = hovered_kind {
            let tip = format!(
                "{}: {}",
                match k {
                    TimerKind::Clock => "Clock",
                    TimerKind::Pomodoro => "Pomodoro",
                    TimerKind::Alarm => "Alarm",
                },
                self.app.timer_summary(k)
            );
            resp.on_hover_text_at_pointer(tip);
        }
        if let Some(k) = clicked {
            self.app.timer_widget_clicked(k);
        }
        ui.ctx()
            .request_repaint_after(self.app.timers.redraw_in(now_ms()));
        Some((right, s))
    }

    pub(super) fn draw_timer_dialog(
        &mut self,
        ui: &mut egui::Ui,
        p: &Painter,
        full: Rect,
        c: &Palette,
    ) {
        let Some(Modal::Timers(d)) = &self.app.modal else {
            return;
        };
        let kind = d.kind;
        let summary = self.app.timer_summary(kind);
        let row_h = 32.0;
        let n = d.fields.len();
        let w = 460.0_f32.min(full.width() - 40.0);
        let h = 86.0 + n as f32 * row_h + if d.error.is_some() { 26.0 } else { 0.0 } + 34.0;
        let r = Rect::from_center_size(
            full.center() - Vec2::new(0.0, full.height() * 0.08),
            Vec2::new(w, h),
        );
        block(ui, full, "timer-dialog");
        p.rect_filled(full, 0.0, Color32::from_black_alpha(80));
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
            d.title(),
            FontId::proportional(16.0),
            c.fg,
        );
        p.text(
            Pos2::new(r.left() + 18.0, r.top() + 50.0),
            Align2::LEFT_CENTER,
            summary,
            FontId::proportional(12.5),
            c.dim,
        );
        let font = FontId::proportional(UI_SIZE);
        let top = r.top() + 70.0;
        let mut clicked = None;
        for (i, f) in d.fields.iter().enumerate() {
            let rr = Rect::from_min_size(
                Pos2::new(r.left() + 8.0, top + i as f32 * row_h),
                Vec2::new(r.width() - 16.0, row_h - 2.0),
            );
            let resp = ui.interact(rr, Id::unique(("timer-row", i)), Sense::click());
            let sel = i == d.sel;
            if resp.clicked() {
                clicked = Some(i);
            }
            match &f.kind {
                FieldKind::Button(_) => {
                    let b = rr.shrink2(Vec2::new(90.0, 2.0));
                    let fill = if sel {
                        c.pop_sel
                    } else if resp.hovered() {
                        mix(c.pop, c.fg, 0.10)
                    } else {
                        mix(c.pop, c.fg, 0.05)
                    };
                    p.rect_filled(b, 4.0, fill);
                    p.text(b.center(), Align2::CENTER_CENTER, &f.label, font.clone(), c.fg);
                }
                kind => {
                    if sel {
                        p.rect_filled(rr, 4.0, c.pop_sel);
                    } else if resp.hovered() {
                        p.rect_filled(rr, 4.0, mix(c.pop, c.fg, 0.05));
                    }
                    p.text(
                        Pos2::new(rr.left() + 10.0, rr.center().y),
                        Align2::LEFT_CENTER,
                        &f.label,
                        font.clone(),
                        c.fg,
                    );
                    match kind {
                        FieldKind::Text(e) => {
                            let field = Rect::from_min_max(
                                Pos2::new(rr.right() - 170.0, rr.top() + 3.0),
                                Pos2::new(rr.right() - 6.0, rr.bottom() - 3.0),
                            );
                            p.rect_filled(field, 3.0, c.bg);
                            draw_line_edit(
                                p,
                                field,
                                field.left() + 8.0,
                                e,
                                FontId::monospace(13.0),
                                c,
                                sel,
                                "",
                            );
                        }
                        FieldKind::Toggle(on) => {
                            let sw = Rect::from_center_size(
                                Pos2::new(rr.right() - 26.0, rr.center().y),
                                Vec2::new(30.0, 16.0),
                            );
                            p.rect_filled(
                                sw,
                                8.0,
                                if *on { c.accent } else { mix(c.pop, c.fg, 0.2) },
                            );
                            let kx = if *on { sw.right() - 8.0 } else { sw.left() + 8.0 };
                            p.circle_filled(Pos2::new(kx, sw.center().y), 6.0, c.fg);
                        }
                        FieldKind::Button(_) => {}
                    }
                }
            }
        }
        let mut y = top + n as f32 * row_h + 8.0;
        if let Some(e) = &d.error {
            p.text(
                Pos2::new(r.left() + 18.0, y + 6.0),
                Align2::LEFT_CENTER,
                e,
                FontId::proportional(12.5),
                c.err,
            );
            y += 26.0;
        }
        p.text(
            Pos2::new(r.center().x, y + 10.0),
            Align2::CENTER_CENTER,
            "Up/Down move · Space toggle/press · Enter apply · Esc close",
            FontId::proportional(11.5),
            c.dim,
        );
        if let Some(i) = clicked {
            self.app.timer_row_clicked(i);
        }
    }
}
