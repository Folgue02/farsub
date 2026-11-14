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

/// Every grammar that ships with farsub: two-face's 220 (bat's set) plus the ones in
/// runtime/syntaxes, precompiled by build.rs.
pub fn builtin_syntaxes() -> SyntaxSet {
    syntect::dumps::from_uncompressed_data(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/syntaxes.packdump"
    )))
    .expect("bundled syntax dump")
}

impl Syntax {
    pub fn load(theme: &str) -> Self {
        let ss = builtin_syntaxes();
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
    pub fn guide(&self) -> Option<Color32> {
        self.theme.settings.guide.map(rgb)
    }
    pub fn active_guide(&self) -> Option<Color32> {
        self.theme.settings.active_guide.map(rgb)
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

#[cfg(test)]
mod bundled_tests {
    use super::*;
    use std::collections::HashMap;

    /// Scopes the parser assigns to `text`, as "scope: token" strings.
    fn scopes(ss: &SyntaxSet, syntax: &str, text: &str) -> Vec<String> {
        let syn = ss
            .find_syntax_by_name(syntax)
            .unwrap_or_else(|| panic!("{syntax}"));
        let mut state = ParseState::new(syn);
        let mut stack = ScopeStack::new();
        let mut out = vec![];
        for line in text.lines() {
            let line = format!("{line}\n");
            let ops = state.parse_line(&line, ss).expect("parse");
            let mut last = 0;
            for (i, op) in ops {
                if i > last {
                    let tok = line[last..i].trim();
                    if !tok.is_empty()
                        && let Some(top) = stack.as_slice().last()
                    {
                        out.push(format!("{top}: {tok}"));
                    }
                }
                last = i;
                stack.apply(&op).unwrap();
            }
        }
        out
    }

    fn has(v: &[String], scope_prefix: &str, tok: &str) -> bool {
        v.iter()
            .any(|s| s.starts_with(scope_prefix) && s.ends_with(&format!(": {tok}")))
    }

    #[test]
    fn bundled_grammars_load_and_highlight() {
        let ss = builtin_syntaxes();
        assert!(ss.syntaxes().len() >= 225, "{}", ss.syntaxes().len());
        let g = scopes(
            &ss,
            "Gleam",
            "// hi\npub fn add(a: Int) -> Int {\n  a + 1 |> io.debug\n}\nconst s = \"x\\n\"",
        );
        assert!(has(&g, "comment", "// hi"));
        assert!(has(&g, "entity.name.function", "add"));
        assert!(has(&g, "storage.type", "Int"));
        assert!(has(&g, "keyword.operator", "|>"));
        let j = scopes(
            &ss,
            "Just",
            "# build it\nset shell := [\"bash\", \"-c\"]\nversion := `git describe`\nbuild target='x': deps\n    cargo build {{target}}",
        );
        assert!(has(&j, "comment", "# build it"));
        assert!(has(&j, "keyword.control", "set"));
        assert!(has(&j, "entity.name.function", "build"));
        assert!(has(&j, "entity.name.function", "deps"));
        assert!(has(&j, "variable.other", "target"));
        let k = scopes(
            &ss,
            "KDL",
            "// cfg\npackage name=\"farsub\" {\n  version 1.2\n  dev #true\n}",
        );
        assert!(has(&k, "entity.name.tag", "package"));
        assert!(has(&k, "entity.other.attribute-name", "name"));
        assert!(has(&k, "constant.numeric", "1.2"));
        let p = scopes(
            &ss,
            "Prisma",
            "model User {\n  id    Int    @id @default(autoincrement())\n  email String @unique\n}",
        );
        assert!(has(&p, "entity.name.type", "User"));
        assert!(has(&p, "support.type", "Int"));
        assert!(has(&p, "entity.other.attribute-name", "@id"));
        let m = scopes(
            &ss,
            "Meson",
            "project('farsub', 'c', version: '1.0')\nif get_option('x')\nendif",
        );
        assert!(has(&m, "support.function", "project"));
        assert!(has(&m, "keyword.control", "endif"));
        assert!(has(&m, "variable.parameter", "version"));
    }

    #[test]
    fn every_language_resolves_to_a_grammar_and_known_servers() {
        let cfg = crate::config::Config::builtin();
        let syn = Syntax::load("Monokai Extended");
        assert!(cfg.languages.len() >= 70, "{}", cfg.languages.len());
        let mut owner: HashMap<String, String> = HashMap::new();
        let mut no_grammar = vec![];
        for l in &cfg.languages {
            if let Some(s) = &l.syntax {
                assert!(syn.find_by_name(s).is_some(), "{}: no syntax {s:?}", l.name);
            }
            for s in &l.servers {
                assert!(cfg.server(s).is_some(), "{}: unknown server {s}", l.name);
            }
            if let Some(re) = &l.first_line {
                regex::Regex::new(re).unwrap_or_else(|e| panic!("{}: {e}", l.name));
            }
            let names = l
                .extensions
                .iter()
                .map(|e| format!("x.{e}"))
                .chain(l.filenames.iter().cloned());
            for n in names {
                // No two built-in languages claim the same extension or file name.
                if let Some(prev) = owner.insert(n.clone(), l.name.clone()) {
                    panic!("{n} claimed by {prev} and {}", l.name);
                }
                // Every claimed file gets real highlighting, not Plain Text.
                let p = Path::new(&n);
                let lang = cfg.language_for(p, "").map(|l| l.name.as_str());
                assert_eq!(lang, Some(l.name.as_str()), "{n}");
                let grammar = l.syntax.clone().unwrap_or_else(|| syn.detect(Some(p), ""));
                if grammar == "Plain Text" {
                    no_grammar.push(format!("{n} ({})", l.name));
                }
            }
        }
        assert!(
            no_grammar.is_empty(),
            "files without a grammar: {no_grammar:?}"
        );
        // Every built-in server's install recipe (if any) has a source.
        for s in &cfg.servers {
            if let Some(i) = &s.install {
                assert_ne!(i.source(), "none", "{}", s.name);
            }
        }
    }
}
