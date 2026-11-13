//! Sublime-style fuzzy palette. The query prefix picks the mode, like Goto Anything:
//! `>` commands, `@` symbols, `:` line, `#` project symbols, nothing = files.

use std::path::PathBuf;

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use crate::commands::Command;
use crate::editor::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Files,
    Commands,
    Symbols,
    Line,
    WorkspaceSymbols,
    Buffers,
    Locations,
}

#[derive(Clone, Debug)]
pub enum Action {
    Open(PathBuf, Option<Pos>),
    Command(Command),
    Goto(Pos),
    Buffer(usize),
    #[allow(dead_code)]
    None,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub action: Action,
}

pub struct Palette {
    pub mode: Mode,
    /// Modes that cannot be switched by typing a prefix (buffers, references list).
    pub fixed: bool,
    pub title: String,
    pub query: String,
    pub items: Vec<Item>,
    pub shown: Vec<(usize, Vec<usize>)>,
    pub sel: usize,
    pub scroll: usize,
    /// (buffer index, cursor, scroll_y) to restore when a previewing palette is cancelled.
    pub origin: Option<(usize, Pos, usize)>,
    pub loading: bool,
    pub ws_request: Option<i64>,
    matcher: SkimMatcherV2,
}

const MAX_SHOWN: usize = 500;

impl Palette {
    pub fn new(mode: Mode, query: &str) -> Self {
        Palette {
            mode,
            fixed: matches!(mode, Mode::Buffers | Mode::Locations),
            title: String::new(),
            query: query.to_string(),
            items: vec![],
            shown: vec![],
            sel: 0,
            scroll: 0,
            origin: None,
            loading: false,
            ws_request: None,
            matcher: SkimMatcherV2::default().smart_case(),
        }
    }

    pub fn prefix(mode: Mode) -> &'static str {
        match mode {
            Mode::Commands => ">",
            Mode::Symbols => "@",
            Mode::Line => ":",
            Mode::WorkspaceSymbols => "#",
            _ => "",
        }
    }

    pub fn mode_for_query(&self) -> Mode {
        if self.fixed {
            return self.mode;
        }
        match self.query.chars().next() {
            Some('>') => Mode::Commands,
            Some('@') => Mode::Symbols,
            Some(':') => Mode::Line,
            Some('#') => Mode::WorkspaceSymbols,
            _ => Mode::Files,
        }
    }

    /// Query without the mode prefix (and, for files, without a trailing `:line`).
    pub fn needle(&self) -> &str {
        let p = Self::prefix(self.mode);
        let q = if !self.fixed && !p.is_empty() {
            self.query.strip_prefix(p).unwrap_or(&self.query)
        } else {
            &self.query
        };
        let q = q.trim_start();
        if self.mode == Mode::Files
            && let Some((f, l)) = q.rsplit_once(':')
            && l.chars().all(|c| c.is_ascii_digit())
        {
            return f;
        }
        q
    }

    /// For `file.rs:42` queries in file mode.
    pub fn file_line(&self) -> Option<usize> {
        if self.mode != Mode::Files {
            return None;
        }
        let (_, l) = self.query.rsplit_once(':')?;
        l.parse::<usize>().ok().filter(|n| *n > 0)
    }

    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let needle = self.needle().to_string();
        if needle.is_empty() {
            self.shown = (0..self.items.len().min(5000))
                .map(|i| (i, vec![]))
                .collect();
        } else {
            let mut scored: Vec<(i64, usize, Vec<usize>)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, it)| {
                    self.matcher
                        .fuzzy_indices(&it.label, &needle)
                        .map(|(s, idx)| {
                            // Prefer matches in the file name part / shorter labels.
                            let bonus = if self.mode == Mode::Files {
                                let base = it.label.rfind('/').map(|p| p + 1).unwrap_or(0);
                                idx.iter().filter(|&&c| c >= base).count() as i64 * 4
                            } else {
                                0
                            };
                            (s + bonus - it.label.len() as i64 / 8, i, idx)
                        })
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0));
            scored.truncate(MAX_SHOWN);
            self.shown = scored.into_iter().map(|(_, i, idx)| (i, idx)).collect();
        }
        self.sel = 0;
        self.scroll = 0;
    }

    pub fn selected(&self) -> Option<&Item> {
        self.shown
            .get(self.sel)
            .and_then(|(i, _)| self.items.get(*i))
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.shown.is_empty() {
            return;
        }
        let n = self.shown.len() as isize;
        let mut s = self.sel as isize + delta;
        if delta.abs() == 1 {
            s = s.rem_euclid(n);
        } else {
            s = s.clamp(0, n - 1);
        }
        self.sel = s as usize;
    }
}
