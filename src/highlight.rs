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
    states: Vec<(ParseState, HighlightState)>,
    pub lines: Vec<Vec<Span>>,
}

const MAX_HL_LINE: usize = 4000;

impl HlCache {
    pub fn new(syntax: String) -> Self {
        HlCache {
            syntax,
            states: vec![],
            lines: vec![],
        }
    }

    pub fn reset(&mut self, syntax: String) {
        self.syntax = syntax;
        self.states.clear();
        self.lines.clear();
    }

    pub fn invalidate(&mut self, line: usize) {
        self.states.truncate(line + 1);
        self.lines.truncate(line);
    }

    pub fn ensure(&mut self, syn: &Syntax, text: &[String], upto: usize) {
        let upto = upto.min(text.len());
        let hl = Highlighter::new(&syn.theme);
        if self.states.is_empty() {
            let sr = syn
                .ss
                .find_syntax_by_name(&self.syntax)
                .unwrap_or_else(|| syn.ss.find_syntax_plain_text());
            self.states.push((
                ParseState::new(sr),
                HighlightState::new(&hl, ScopeStack::new()),
            ));
            self.lines.clear();
        }
        while self.lines.len() < upto {
            let i = self.lines.len();
            let (mut ps, mut hs) = self.states[i].clone();
            let src = &text[i];
            let mut spans = Vec::new();
            if src.len() <= MAX_HL_LINE {
                let mut line = String::with_capacity(src.len() + 1);
                line.push_str(src);
                line.push('\n');
                let ops = ps.parse_line(&line, &syn.ss).unwrap_or_default();
                let mut off = 0;
                for (style, piece) in HighlightIterator::new(&mut hs, &ops, &line, &hl) {
                    let end = (off + piece.len()).min(src.len());
                    if end > off {
                        spans.push((off, end, to_style(style)));
                    }
                    off += piece.len();
                }
            }
            self.lines.push(spans);
            self.states.push((ps, hs));
        }
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
