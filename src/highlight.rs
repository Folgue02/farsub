//! Incremental syntect highlighting. Parse state is cached at the start of every line so an
//! edit only re-highlights from the changed line down to the bottom of the viewport.

use std::path::Path;

use eframe::egui::Color32;
use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

pub struct Syntax {
    pub ss: SyntaxSet,
    pub theme: Theme,
    pub themes: two_face::theme::LazyThemeSet,
    /// Themes from Sublime packages (.tmTheme), by name.
    pub user_themes: Vec<(String, Theme)>,
}

/// Color + font flags for a highlighted span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fg: Color32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

pub fn rgb(c: syntect::highlighting::Color) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

impl Syntax {
    pub fn load(theme: &str) -> Self {
        let ss = two_face::syntax::extra_newlines();
        let themes = two_face::theme::LazyThemeSet::from(two_face::theme::extra());
        let mut s = Syntax {
            ss,
            theme: Theme::default(),
            themes,
            user_themes: vec![],
        };
        s.set_theme(theme);
        s
    }

    /// Switch theme by name; falls back to Monokai Extended. Returns false if unknown.
    pub fn set_theme(&mut self, name: &str) -> bool {
        if let Some((_, t)) = self.user_themes.iter().find(|(n, _)| n == name) {
            self.theme = t.clone();
            return true;
        }
        match self.themes.get(name) {
            Some(t) => {
                self.theme = t.clone();
                true
            }
            None => {
                self.theme = self
                    .themes
                    .get("Monokai Extended")
                    .cloned()
                    .unwrap_or_default();
                false
            }
        }
    }

    pub fn theme_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.themes.theme_names().map(String::from).collect();
        v.extend(self.user_themes.iter().map(|(n, _)| n.clone()));
        v.sort();
        v
    }

    pub fn find_by_name(&self, name: &str) -> Option<String> {
        self.ss
            .syntaxes()
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(|s| s.name.clone())
    }

    pub fn detect(&self, path: Option<&Path>, first_line: &str) -> String {
        let by_path = path.and_then(|p| {
            let ext = p.extension().and_then(|e| e.to_str());
            let name = p.file_name().and_then(|e| e.to_str());
            ext.and_then(|e| self.ss.find_syntax_by_extension(e))
                .or_else(|| name.and_then(|n| self.ss.find_syntax_by_extension(n)))
        });
        by_path
            .or_else(|| self.ss.find_syntax_by_first_line(first_line))
            .unwrap_or_else(|| self.ss.find_syntax_plain_text())
            .name
            .clone()
    }

    pub fn bg(&self) -> Color32 {
        self.theme
            .settings
            .background
            .map(rgb)
            .unwrap_or(Color32::from_rgb(39, 40, 34))
    }
    pub fn fg(&self) -> Color32 {
        self.theme
            .settings
            .foreground
            .map(rgb)
            .unwrap_or(Color32::from_rgb(248, 248, 242))
    }
    pub fn selection(&self) -> Color32 {
        self.theme
            .settings
            .selection
            .map(rgb)
            .unwrap_or(Color32::from_rgb(73, 72, 62))
    }
    pub fn line_highlight(&self) -> Color32 {
        self.theme
            .settings
            .line_highlight
            .map(rgb)
            .unwrap_or(Color32::from_rgb(50, 51, 44))
    }
    pub fn gutter(&self) -> Color32 {
        self.theme
            .settings
            .gutter_foreground
            .map(rgb)
            .unwrap_or(Color32::from_rgb(117, 113, 94))
    }
}

pub type Span = (usize, usize, Style);

pub struct HlCache {
    pub syntax: String,
    /// Parser state at the start of every CHECKPOINT-th line.
    checkpoints: Vec<(ParseState, HighlightState)>,
    /// Running state at the start of line `lines.len()`.
    frontier: Option<(ParseState, HighlightState)>,
    pub lines: Vec<Vec<Span>>,
}

const MAX_HL_LINE: usize = 4000;
const CHECKPOINT: usize = 64;

impl HlCache {
    pub fn new(syntax: String) -> Self {
        HlCache {
            syntax,
            checkpoints: vec![],
            frontier: None,
            lines: vec![],
        }
    }

    pub fn reset(&mut self, syntax: String) {
        self.syntax = syntax;
        self.checkpoints.clear();
        self.frontier = None;
        self.lines.clear();
    }

    pub fn invalidate(&mut self, line: usize) {
        if line >= self.lines.len() {
            return;
        }
        let k = line / CHECKPOINT;
        self.checkpoints.truncate(k + 1);
        self.lines.truncate(k * CHECKPOINT);
        self.frontier = None;
    }

    /// Highlight lines up to `upto`, stopping at `deadline`. Returns true when done, so a
    /// jump deep into a huge file never freezes the UI; the rest fills in over later frames.
    pub fn ensure(
        &mut self,
        syn: &Syntax,
        text: &[String],
        upto: usize,
        deadline: std::time::Instant,
    ) -> bool {
        let upto = upto.min(text.len());
        if self.lines.len() >= upto {
            return true;
        }
        let hl = Highlighter::new(&syn.theme);
        if self.frontier.is_none() {
            self.frontier = Some(match self.checkpoints.get(self.lines.len() / CHECKPOINT) {
                Some(st) => st.clone(),
                None => {
                    let sr = syn
                        .ss
                        .find_syntax_by_name(&self.syntax)
                        .unwrap_or_else(|| syn.ss.find_syntax_plain_text());
                    self.lines.clear();
                    self.checkpoints.clear();
                    (
                        ParseState::new(sr),
                        HighlightState::new(&hl, ScopeStack::new()),
                    )
                }
            });
        }
        let (ps, hs) = self.frontier.as_mut().unwrap();
        let mut line = String::new();
        while self.lines.len() < upto {
            let i = self.lines.len();
            if i % CHECKPOINT == 0 && self.checkpoints.len() == i / CHECKPOINT {
                self.checkpoints.push((ps.clone(), hs.clone()));
            }
            let src = &text[i];
            let mut spans = Vec::new();
            if src.len() <= MAX_HL_LINE {
                line.clear();
                line.push_str(src);
                line.push('\n');
                let ops = ps.parse_line(&line, &syn.ss).unwrap_or_default();
                let mut off = 0;
                for (style, piece) in HighlightIterator::new(hs, &ops, &line, &hl) {
                    let end = (off + piece.len()).min(src.len());
                    if end > off {
                        spans.push((off, end, to_style(style)));
                    }
                    off += piece.len();
                }
            }
            self.lines.push(spans);
            if i % 32 == 31 && std::time::Instant::now() > deadline {
                return self.lines.len() >= upto;
            }
        }
        true
    }
}

fn to_style(s: syntect::highlighting::Style) -> Style {
    Style {
        fg: rgb(s.foreground),
        bold: s.font_style.contains(FontStyle::BOLD),
        italic: s.font_style.contains(FontStyle::ITALIC),
        underline: s.font_style.contains(FontStyle::UNDERLINE),
    }
}
