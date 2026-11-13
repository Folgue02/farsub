//! Incremental syntect highlighting. Parse state is cached at the start of every line so an
//! edit only re-highlights from the changed line down to the bottom of the viewport.

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

pub struct Syntax {
    pub ss: SyntaxSet,
    pub theme: Theme,
}

pub fn rgb(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

impl Syntax {
    pub fn load() -> Self {
        let ss = two_face::syntax::extra_newlines();
        let themes = two_face::theme::extra();
        let theme = themes
            .get(two_face::theme::EmbeddedThemeName::MonokaiExtended)
            .clone();
        Syntax { ss, theme }
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

    pub fn bg(&self) -> Color {
        self.theme
            .settings
            .background
            .map(rgb)
            .unwrap_or(Color::Rgb(39, 40, 34))
    }
    pub fn fg(&self) -> Color {
        self.theme
            .settings
            .foreground
            .map(rgb)
            .unwrap_or(Color::Rgb(248, 248, 242))
    }
    pub fn selection(&self) -> Color {
        self.theme
            .settings
            .selection
            .map(rgb)
            .unwrap_or(Color::Rgb(73, 72, 62))
    }
    pub fn line_highlight(&self) -> Color {
        self.theme
            .settings
            .line_highlight
            .map(rgb)
            .unwrap_or(Color::Rgb(50, 51, 44))
    }
    pub fn gutter(&self) -> Color {
        self.theme
            .settings
            .gutter_foreground
            .map(rgb)
            .unwrap_or(Color::Rgb(117, 113, 94))
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
    let mut st = Style::default().fg(rgb(s.foreground));
    if s.font_style.contains(FontStyle::BOLD) {
        st = st.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        st = st.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(FontStyle::UNDERLINE) {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    st
}
