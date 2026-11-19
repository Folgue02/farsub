//! App: console selection / console.

use super::*;

impl App {
    // ===================================================================== console selection

    /// The rendered text of a log line in the console.
    pub fn console_line(e: &crate::log::Entry) -> String {
        format!(
            "{} [{}] {}",
            &e.time[..8.min(e.time.len())],
            e.source,
            e.text
        )
    }

    pub fn console_entries(&self) -> Vec<crate::log::Entry> {
        let min = if self.console.show_debug {
            crate::log::Level::Debug
        } else {
            crate::log::Level::Info
        };
        crate::log::entries(min, &self.console.filter)
    }

    pub fn console_select_all(&mut self) {
        let es = self.console_entries();
        if let (Some(a), Some(b)) = (es.first(), es.last()) {
            self.console.sel = Some(((a.id, 0), (b.id, Self::console_line(b).chars().count())));
        }
    }

    pub fn console_selection_text(&self) -> Option<String> {
        let (a, b) = self.console.sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = vec![];
        for e in self
            .console_entries()
            .iter()
            .filter(|e| e.id >= a.0 && e.id <= b.0)
        {
            let line: Vec<char> = Self::console_line(e).chars().collect();
            let s = if e.id == a.0 { a.1.min(line.len()) } else { 0 };
            let t = if e.id == b.0 {
                b.1.min(line.len())
            } else {
                line.len()
            };
            out.push(line[s..t.max(s)].iter().collect::<String>());
        }
        let text = out.join("\n");
        (!text.is_empty()).then_some(text)
    }

    // ===================================================================== console

    pub(super) fn console_key(&mut self, k: Key) {
        let c = &mut self.console;
        match k.code {
            KeyCode::Esc => {
                if c.sel.take().is_none() {
                    self.panel.visible = false;
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Enter => {
                let line = c.input.take();
                c.hist_pos = None;
                if !line.trim().is_empty() && c.history.last() != Some(&line) {
                    c.history.push(line.clone());
                }
                self.console_exec(line.trim());
            }
            KeyCode::Up | KeyCode::Down if !c.history.is_empty() => {
                let n = c.history.len();
                let pos = match (c.hist_pos, k.code) {
                    (None, KeyCode::Up) => Some(n - 1),
                    (Some(p), KeyCode::Up) => Some(p.saturating_sub(1)),
                    (Some(p), _) if p + 1 < n => Some(p + 1),
                    _ => None,
                };
                c.hist_pos = pos;
                c.input = LineEdit::new(pos.map(|p| c.history[p].as_str()).unwrap_or(""));
            }
            KeyCode::PageUp => c.scroll += 10,
            KeyCode::PageDown => c.scroll = c.scroll.saturating_sub(10),
            KeyCode::Char('l') if k.ctrl => crate::log::clear(),
            _ => {}
        }
    }

    /// Run a console line: a few built-ins, otherwise a command id.
    pub fn console_exec(&mut self, line: &str) {
        use crate::log;
        if line.is_empty() {
            return;
        }
        log::info("console", format!("> {line}"));
        let (word, rest) = line
            .split_once(' ')
            .map(|(a, b)| (a, b.trim()))
            .unwrap_or((line, ""));
        match word {
            "help" => log::info(
                "console",
                "help · clear · debug (toggle server output) · filter TEXT · wrap (toggle line wrap) · log (open log file) · lsp (LSP info) · \
                 <command id> runs a command, e.g. restart_lsp, goto_file, toggle_sidebar (see Help › Keyboard Shortcuts)",
            ),
            "clear" => log::clear(),
            "wrap" => {
                self.console.wrap = !self.console.wrap;
                self.console.hscroll = 0.0;
                log::info(
                    "console",
                    format!(
                        "line wrapping {}",
                        if self.console.wrap { "on" } else { "off" }
                    ),
                );
            }
            "debug" => {
                self.console.show_debug = !self.console.show_debug;
                log::info(
                    "console",
                    format!(
                        "debug output {}",
                        if self.console.show_debug {
                            "shown"
                        } else {
                            "hidden"
                        }
                    ),
                );
            }
            "filter" => {
                self.console.filter = rest.to_string();
                log::info(
                    "console",
                    if rest.is_empty() {
                        "filter cleared".into()
                    } else {
                        format!("showing lines matching \"{rest}\"")
                    },
                );
            }
            "log" => match self.config.data_dir.clone() {
                Some(d) => {
                    let f = d.join("logs").join("farsub.log");
                    self.open_file(&f, true);
                }
                None => log::warn("console", "no data folder"),
            },
            "lsp" => {
                for (_, l) in self.lsp_info() {
                    log::info("console", if l.is_empty() { " ".into() } else { l });
                }
            }
            id => match crate::commands::from_id(id) {
                Some(cmd) => {
                    self.focus = Focus::Editor;
                    self.run(cmd);
                    if self.focus == Focus::Editor && self.modal.is_none() {
                        self.focus = Focus::Console;
                    }
                }
                None => log::error("console", format!("unknown command `{id}` (type help)")),
            },
        }
    }
}
