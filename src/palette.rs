//! Fuzzy palette. The query prefix picks the mode, like Goto Anything:
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
    Themes,
    Packages,
    Projects,
    Syntaxes,
    Indentation,
    /// Grammars that can be installed (and reinstalled).
    Grammars,
    /// Installed grammars, to remove.
    GrammarsRemove,
    /// Installed fonts (Preferences › Select Font).
    Fonts,
    /// The language server's code actions at the cursor.
    CodeActions,
}

/// Items of the Indentation menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentAction {
    Spaces,
    Tabs,
    Width(usize),
    Guess,
    ToSpaces,
    ToTabs,
}

#[derive(Clone, Debug)]
pub enum Action {
    Open(PathBuf, Option<Pos>),
    Command(Command),
    Goto(Pos),
    Buffer(usize),
    Theme(String),
    Font(String),
    /// Index into the offered code actions.
    CodeAction(usize),
    Package(String),
    Project(crate::project::Last),
    Syntax(String),
    Indent(IndentAction),
    InstallGrammar(String),
    RemoveGrammar(String),
    #[allow(dead_code)]
    None,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub detail: String,
    pub action: Action,
    /// A symbol kind ("fn", "class", …) drawn as its Codicon before the label.
    pub icon: Option<&'static str>,
}

pub struct Palette {
    pub mode: Mode,
    /// Modes that cannot be switched by typing a prefix (buffers, references list).
    pub fixed: bool,
    pub title: String,
    pub query: crate::lineedit::LineEdit,
    pub items: Vec<Item>,
    pub shown: Vec<(usize, Vec<usize>)>,
    pub sel: usize,
    pub scroll: usize,
    /// (buffer index, cursor, scroll_y) to restore when a previewing palette is cancelled.
    pub origin: Option<(usize, Pos, usize)>,
    pub loading: bool,
    pub ws_request: Option<i64>,
    /// (needle, indices of items that matched it) for incremental narrowing.
    narrow: Option<(String, Vec<usize>)>,
    matcher: SkimMatcherV2,
}

const MAX_SHOWN: usize = 500;

impl Palette {
    pub fn new(mode: Mode, query: &str) -> Self {
        Palette {
            mode,
            fixed: matches!(
                mode,
                Mode::Buffers
                    | Mode::Locations
                    | Mode::Themes
                    | Mode::Packages
                    | Mode::Projects
                    | Mode::Syntaxes
                    | Mode::Indentation
                    | Mode::Grammars
                    | Mode::GrammarsRemove
                    | Mode::Fonts
                    | Mode::CodeActions
            ),
            title: String::new(),
            query: crate::lineedit::LineEdit::new(query),
            items: vec![],
            shown: vec![],
            sel: 0,
            scroll: 0,
            origin: None,
            loading: false,
            ws_request: None,
            narrow: None,
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
        match self.query.text().chars().next() {
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
            self.query
                .text()
                .strip_prefix(p)
                .unwrap_or(self.query.text())
        } else {
            self.query.text()
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
        let (_, l) = self.query.text().rsplit_once(':')?;
        l.parse::<usize>().ok().filter(|n| *n > 0)
    }

    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.narrow = None;
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let needle = self.needle().to_string();
        self.sel = 0;
        self.scroll = 0;
        if needle.is_empty() {
            self.narrow = None;
            self.shown = (0..self.items.len().min(5000))
                .map(|i| (i, vec![]))
                .collect();
            return;
        }
        // Typing more characters can only shrink the match set, so only re-score the
        // previous matches instead of the whole list.
        let candidates: Vec<usize> = match self.narrow.take() {
            Some((prev, hits)) if needle.starts_with(&prev) => hits,
            _ => (0..self.items.len()).collect(),
        };
        // Pass 1: cheap score-only match, in parallel for big lists.
        let items = &self.items;
        let score_chunk = |chunk: &[usize]| -> Vec<(i64, usize)> {
            let m = SkimMatcherV2::default().smart_case();
            chunk
                .iter()
                .filter_map(|&i| {
                    let l = &items[i].label;
                    m.fuzzy_match(l, &needle)
                        .map(|s| (s - l.len() as i64 / 8, i))
                })
                .collect()
        };
        let mut scored: Vec<(i64, usize)> = if candidates.len() > 20_000 {
            let n = std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(8);
            let size = candidates.len().div_ceil(n);
            std::thread::scope(|sc| {
                let hs: Vec<_> = candidates
                    .chunks(size)
                    .map(|c| sc.spawn(move || score_chunk(c)))
                    .collect();
                hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
            })
        } else {
            score_chunk(&candidates)
        };
        let mut hits: Vec<usize> = scored.iter().map(|s| s.1).collect();
        hits.sort_unstable();
        self.narrow = Some((needle.clone(), hits));
        // Pass 2: match positions (for highlighting + file-name bonus) on the best few only.
        scored.sort_unstable_by_key(|s| std::cmp::Reverse(s.0));
        scored.truncate(MAX_SHOWN * 4);
        let mut ranked: Vec<(i64, usize, Vec<usize>)> = scored
            .into_iter()
            .filter_map(|(base, i)| {
                let label = &self.items[i].label;
                let (_, idx) = self.matcher.fuzzy_indices(label, &needle)?;
                let bonus = if self.mode == Mode::Files {
                    let start = label.rfind('/').map(|p| p + 1).unwrap_or(0);
                    let start = label[..start].chars().count();
                    idx.iter().filter(|&&c| c >= start).count() as i64 * 4
                } else {
                    0
                };
                Some((base + bonus, i, idx))
            })
            .collect();
        ranked.sort_by_key(|r| std::cmp::Reverse(r.0));
        ranked.truncate(MAX_SHOWN);
        self.shown = ranked.into_iter().map(|(_, i, idx)| (i, idx)).collect();
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
