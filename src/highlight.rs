//! Syntax highlighting with tree-sitter (and a regex fallback, `fallback.rs`, for files
//! with no usable tree-sitter grammar).
//!
//! Each buffer keeps a parser and its syntax tree. Edits are fed to the tree as they happen
//! (`HlCache::edit`), so re-parsing is incremental; a long first parse is spread over
//! frames. Only the visible lines are highlighted, by running the grammar's highlight query
//! over that byte range.
//!
//! Everything visual is data: grammars' queries in `queries/<grammar>/highlights.scm` and
//! themes in `themes/*.toml` (built-ins in runtime/, overridable from packages and the
//! config folder).

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use eframe::egui::Color32;
use serde::Deserialize;
use tree_sitter::{
    InputEdit, Language, Node, ParseOptions, Parser, Point, Query, QueryCursor, StreamingIterator,
    TextProvider, Tree,
};

/// Color + font flags for a highlighted span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub fg: Color32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// (start byte, end byte, style) within one line.
pub type Span = (usize, usize, Style);

// ---------------------------------------------------------------- themes

/// A color theme (`themes/*.toml`): editor colors and a style per highlight capture.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    pub name: String,
    editor: HashMap<String, Color32>,
    syntax: HashMap<String, Style>,
}

#[derive(Deserialize)]
struct ThemeFile {
    name: Option<String>,
    #[serde(default)]
    editor: HashMap<String, String>,
    #[serde(default)]
    syntax: HashMap<String, StyleDef>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StyleDef {
    Color(String),
    Full {
        fg: Option<String>,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
        #[serde(default)]
        underline: bool,
    },
}

fn color(s: &str) -> Option<Color32> {
    crate::config::parse_hex_color(s).map(|(r, g, b)| Color32::from_rgb(r, g, b))
}

impl Theme {
    /// Parse a theme file; `fallback_name` is used when it has no `name`.
    pub fn parse(text: &str, fallback_name: &str) -> Result<Theme, String> {
        let f: ThemeFile = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let editor: HashMap<String, Color32> = f
            .editor
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), color(v)?)))
            .collect();
        let fg = editor
            .get("foreground")
            .copied()
            .unwrap_or(Color32::from_rgb(248, 248, 242));
        let syntax = f
            .syntax
            .into_iter()
            .filter_map(|(k, v)| {
                let st = match v {
                    StyleDef::Color(c) => Style {
                        fg: color(&c)?,
                        bold: false,
                        italic: false,
                        underline: false,
                    },
                    StyleDef::Full {
                        fg: c,
                        bold,
                        italic,
                        underline,
                    } => Style {
                        fg: c.as_deref().and_then(color).unwrap_or(fg),
                        bold,
                        italic,
                        underline,
                    },
                };
                Some((k, st))
            })
            .collect();
        Ok(Theme {
            name: f.name.unwrap_or_else(|| fallback_name.to_string()),
            editor,
            syntax,
        })
    }

    /// Style for a capture name, falling back through its parents
    /// (`function.builtin.static` → `function.builtin` → `function`), then, for the
    /// markup names many themes leave out, to a color every theme has.
    pub fn style_for(&self, capture: &str) -> Option<Style> {
        self.own_style(capture).or_else(|| {
            MARKUP_FALLBACKS
                .iter()
                .find(|(m, _)| capture == *m || capture.starts_with(&format!("{m}.")))
                .and_then(|(_, other)| self.own_style(other))
        })
    }

    fn own_style(&self, capture: &str) -> Option<Style> {
        let mut name = capture;
        loop {
            if let Some(s) = self.syntax.get(name) {
                return Some(*s);
            }
            name = &name[..name.rfind('.')?];
        }
    }

    fn editor_color(&self, key: &str) -> Option<Color32> {
        self.editor.get(key).copied()
    }
}

/// Markup a theme doesn't style borrows another color, so Markdown (and similar) still
/// shows its structure: code like strings, links like constants, headings like keywords.
const MARKUP_FALLBACKS: &[(&str, &str)] = &[
    ("markup.raw", "string"),
    ("markup.link", "constant"),
    ("markup.heading", "keyword"),
];

// ---------------------------------------------------------------- grammars

/// A grammar and its highlight query (compiled on first use).
pub struct Grammar {
    pub name: String,
    pub language: Language,
    query_src: String,
    query: OnceLock<Result<Arc<Query>, String>>,
}

impl Grammar {
    fn query(&self) -> Result<Arc<Query>, String> {
        self.query
            .get_or_init(|| {
                Query::new(&self.language, &self.query_src)
                    .map(Arc::new)
                    .map_err(|e| format!("{}: highlights.scm: {e}", self.name))
            })
            .clone()
    }
}

/// Grammars and themes.
pub struct Syntax {
    grammars: HashMap<String, Arc<Grammar>>,
    /// Bundled highlight queries for grammars that aren't compiled in (installed ones).
    spare_queries: HashMap<String, String>,
    themes: Vec<Theme>,
    pub theme: Theme,
    /// Bumped on every theme change (cached highlighting must be redone).
    pub theme_gen: u64,
    /// Problems loading themes / queries (shown in the console).
    pub errors: Vec<String>,
    /// Scope → capture table for the regex fallback (`scopes.toml`).
    pub scopes: crate::fallback::ScopeMap,
    /// Languages by name, to find their fallback grammar.
    languages: HashMap<String, crate::config::LanguageConfig>,
}

const DEFAULT_THEME: &str = "Monokai Extended";

impl Syntax {
    /// The built-in grammars, queries and themes (runtime/).
    pub fn load(theme: &str) -> Self {
        let mut queries: HashMap<String, String> = HashMap::new();
        let mut themes = vec![];
        let mut errors = vec![];
        let mut scopes = crate::fallback::ScopeMap::default();
        for (path, text) in crate::config::RUNTIME_FILES {
            if *path == "scopes.toml" {
                if let Err(e) = scopes.merge(text) {
                    errors.push(format!("built-in {path}: {e}"));
                }
            } else if let Some(rest) = path.strip_prefix("queries/")
                && let Some(g) = rest.strip_suffix("/highlights.scm")
            {
                queries.insert(g.to_string(), text.to_string());
            } else if let Some(file) = path.strip_prefix("themes/") {
                match Theme::parse(text, file.trim_end_matches(".toml")) {
                    Ok(t) => themes.push(t),
                    Err(e) => errors.push(format!("built-in {path}: {e}")),
                }
            }
        }
        let grammars = crate::grammars::builtin()
            .into_iter()
            .map(|(name, language)| {
                let g = Grammar {
                    name: name.to_string(),
                    language,
                    query_src: queries.remove(name).unwrap_or_default(),
                    query: OnceLock::new(),
                };
                (name.to_string(), Arc::new(g))
            })
            .collect();
        let mut s = Syntax {
            grammars,
            spare_queries: queries,
            themes,
            theme: Theme::default(),
            theme_gen: 0,
            errors,
            scopes,
            languages: HashMap::new(),
        };
        s.set_theme(theme);
        s
    }

    /// Grammars installed from recipes (compiled libraries in the data folder). Built-in
    /// grammars win; the query is farsub's bundled one, else the one from the source.
    pub fn add_installed(&mut self, data: &Path) {
        use crate::grammar_install::{installed, installed_query, load};
        for (r, lib) in installed(data) {
            if self.grammars.contains_key(&r.name) {
                continue;
            }
            match load(&lib, &r.symbol) {
                Ok(language) => {
                    let query_src = self
                        .spare_queries
                        .get(&r.name)
                        .cloned()
                        .or_else(|| installed_query(data, &r.name))
                        .unwrap_or_default();
                    let g = Grammar {
                        name: r.name.clone(),
                        language,
                        query_src,
                        query: OnceLock::new(),
                    };
                    self.grammars.insert(r.name.clone(), Arc::new(g));
                }
                Err(e) => self.errors.push(format!("grammar {}: {e}", r.name)),
            }
        }
    }

    /// Add themes (`themes/*.toml`), highlight-query overrides
    /// (`queries/<grammar>/highlights.scm`) and fallback scope rules (`scopes.toml`) from a
    /// package or the config folder.
    pub fn add_dir(&mut self, dir: &Path) {
        let sc = dir.join("scopes.toml");
        if let Ok(text) = std::fs::read_to_string(&sc)
            && let Err(e) = self.scopes.merge(&text)
        {
            self.errors.push(format!("{}: {e}", sc.display()));
        }
        if let Ok(rd) = std::fs::read_dir(dir.join("themes")) {
            let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            files.sort();
            for f in files
                .iter()
                .filter(|f| f.extension().is_some_and(|e| e == "toml"))
            {
                let stem = f.file_stem().unwrap_or_default().to_string_lossy();
                match std::fs::read_to_string(f)
                    .map_err(|e| e.to_string())
                    .and_then(|t| Theme::parse(&t, &stem))
                {
                    Ok(t) => {
                        self.themes.retain(|x| x.name != t.name);
                        self.themes.push(t);
                    }
                    Err(e) => self.errors.push(format!("{}: {e}", f.display())),
                }
            }
        }
        if let Ok(rd) = std::fs::read_dir(dir.join("queries")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let q = e.path().join("highlights.scm");
                let Some(g) = self.grammars.get(&name) else {
                    continue;
                };
                if let Ok(src) = std::fs::read_to_string(&q) {
                    let ng = Grammar {
                        name: name.clone(),
                        language: g.language.clone(),
                        query_src: src,
                        query: OnceLock::new(),
                    };
                    if let Err(err) = ng.query() {
                        self.errors.push(format!("{}: {err}", q.display()));
                        continue;
                    }
                    self.grammars.insert(name, Arc::new(ng));
                }
            }
        }
    }

    /// Switch theme by name; falls back to Monokai Extended. Returns false if unknown.
    pub fn set_theme(&mut self, name: &str) -> bool {
        let found = self.themes.iter().find(|t| t.name == name).cloned();
        let ok = found.is_some();
        self.theme = found
            .or_else(|| {
                self.themes
                    .iter()
                    .find(|t| t.name == DEFAULT_THEME)
                    .cloned()
            })
            .or_else(|| self.themes.first().cloned())
            .unwrap_or_default();
        self.theme_gen += 1;
        ok
    }

    pub fn theme_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.themes.iter().map(|t| t.name.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The languages (to find a file's fallback grammar).
    pub fn set_languages(&mut self, langs: &[crate::config::LanguageConfig]) {
        self.languages = langs.iter().map(|l| (l.name.clone(), l.clone())).collect();
    }

    /// The regex fallback grammar for a language, if one fits.
    pub fn fallback_for(&self, language: &str) -> Option<usize> {
        self.languages.get(language).and_then(crate::fallback::find)
    }

    pub fn has_grammar(&self, name: &str) -> bool {
        self.grammars.contains_key(name)
    }

    #[cfg(test)]
    pub fn grammar_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.grammars.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn grammar(&self, name: &str) -> Option<Arc<Grammar>> {
        self.grammars.get(name).cloned()
    }

    pub fn bg(&self) -> Color32 {
        self.theme
            .editor_color("background")
            .unwrap_or(Color32::from_rgb(39, 40, 34))
    }
    pub fn fg(&self) -> Color32 {
        self.theme
            .editor_color("foreground")
            .unwrap_or(Color32::from_rgb(248, 248, 242))
    }
    pub fn selection(&self) -> Color32 {
        self.theme
            .editor_color("selection")
            .unwrap_or(Color32::from_rgb(73, 72, 62))
    }
    pub fn line_highlight(&self) -> Color32 {
        self.theme
            .editor_color("line_highlight")
            .unwrap_or(Color32::from_rgb(50, 51, 44))
    }
    pub fn guide(&self) -> Option<Color32> {
        self.theme.editor_color("guide")
    }
    pub fn active_guide(&self) -> Option<Color32> {
        self.theme.editor_color("active_guide")
    }
    /// Outline of find matches (theme `find_highlight`).
    pub fn find_highlight(&self) -> Option<Color32> {
        self.theme.editor_color("find_highlight")
    }
    /// A theme style for a highlight capture name (with dotted fallbacks).
    pub fn capture_style(&self, capture: &str) -> Option<Style> {
        self.theme.style_for(capture)
    }
    pub fn gutter(&self) -> Color32 {
        self.theme
            .editor_color("gutter")
            .unwrap_or(Color32::from_rgb(117, 113, 94))
    }
    /// Any other `[editor]` color of the theme (the clock and timer widgets' keys).
    pub fn editor_color(&self, key: &str) -> Option<Color32> {
        self.theme.editor_color(key)
    }
}

// ---------------------------------------------------------------- per-buffer state

/// A buffer's parser, syntax tree and the highlighted visible lines.
pub struct HlCache {
    /// The language shown to the user ("rust", "Plain Text").
    pub syntax: String,
    /// The tree-sitter grammar, if the language has one.
    pub grammar: Option<String>,
    parser: Option<Parser>,
    tree: Option<Tree>,
    /// Text changed since the tree was parsed.
    dirty: bool,
    /// A parse was cut short by the frame deadline (it resumes next frame).
    parsing: bool,
    edits: Vec<InputEdit>,
    tree_gen: u64,
    /// Highlighted lines `first .. first + spans.len()`.
    first: usize,
    spans: Vec<Vec<Span>>,
    /// (tree generation, theme generation, from, to) the spans were made for.
    made_for: Option<(u64, u64, usize, usize)>,
    /// Regex highlighting, used while there's no usable tree-sitter grammar.
    fallback: Option<crate::fallback::Fallback>,
    /// The fallback grammar was looked up (it may not exist).
    fallback_tried: bool,
    /// The last `ensure` used the fallback.
    using_fallback: bool,
}

impl HlCache {
    pub fn new(syntax: String) -> Self {
        HlCache {
            syntax,
            grammar: None,
            parser: None,
            tree: None,
            dirty: true,
            parsing: false,
            edits: vec![],
            tree_gen: 0,
            first: 0,
            spans: vec![],
            made_for: None,
            fallback: None,
            fallback_tried: false,
            using_fallback: false,
        }
    }

    /// Switch language / grammar (drops the tree).
    pub fn reset(&mut self, syntax: String, grammar: Option<String>) {
        *self = HlCache {
            grammar,
            ..HlCache::new(syntax)
        };
    }

    /// Whether edits should be reported (only when there's a grammar to keep in sync).
    pub fn wants_edits(&self) -> bool {
        self.grammar.is_some()
    }

    /// The text changed without exact edit info: parse again from scratch.
    pub fn invalidate(&mut self, line: usize) {
        self.changed_line(line);
        self.tree = None;
        self.edits.clear();
        if self.parsing {
            if let Some(p) = &mut self.parser {
                p.reset();
            }
            self.parsing = false;
        }
        self.dirty = true;
        self.made_for = None;
    }

    /// Text changed from line `line` on (for the regex fallback, which needs no exact edits).
    pub fn changed_line(&mut self, line: usize) {
        if let Some(f) = &mut self.fallback {
            f.invalidate(line);
        }
    }

    /// The fallback grammar's name, when the buffer is highlighted with it.
    #[cfg(test)]
    pub fn fallback_name(&self) -> Option<&'static str> {
        self.fallback
            .as_ref()
            .filter(|_| self.using_fallback)
            .map(|f| f.name())
    }

    /// Report an edit, in bytes and (row, byte column) points, so the tree is re-parsed
    /// incrementally.
    pub fn edit(
        &mut self,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
        start: (usize, usize),
        old_end: (usize, usize),
        new_end: (usize, usize),
    ) {
        self.changed_line(start.0);
        if self.grammar.is_none() {
            return;
        }
        let pt = |(row, column): (usize, usize)| Point { row, column };
        self.edits.push(InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position: pt(start),
            old_end_position: pt(old_end),
            new_end_position: pt(new_end),
        });
        self.dirty = true;
        self.made_for = None;
    }

    /// Highlight spans for line `ln`, if it's in the highlighted range.
    pub fn line(&self, ln: usize) -> Option<&Vec<Span>> {
        if self.using_fallback {
            return self.fallback.as_ref().and_then(|f| f.line(ln));
        }
        ln.checked_sub(self.first).and_then(|i| self.spans.get(i))
    }

    /// Make sure lines `from..to` are highlighted, working until `deadline`. Returns false
    /// if parsing isn't finished yet (the caller should ask for another frame).
    pub fn ensure(
        &mut self,
        syn: &Syntax,
        text: &[String],
        from: usize,
        to: usize,
        deadline: std::time::Instant,
    ) -> bool {
        let to = to.min(text.len());
        let tree_sitter = self
            .grammar
            .as_deref()
            .and_then(|n| syn.grammar(n))
            .and_then(|g| g.query().ok().map(|q| (g, q)));
        let Some((g, query)) = tree_sitter else {
            return self.ensure_fallback(syn, text, to, deadline);
        };
        self.using_fallback = false;
        if self.parser.is_none() {
            let mut p = Parser::new();
            if p.set_language(&g.language).is_err() {
                self.grammar = None;
                return self.ensure_fallback(syn, text, to, deadline);
            }
            self.parser = Some(p);
            self.tree = None;
            self.dirty = true;
        }
        let mut done = true;
        if self.dirty || self.tree.is_none() {
            let parser = self.parser.as_mut().unwrap();
            if !self.edits.is_empty() {
                // New edits invalidate a parse that was cut short.
                if self.parsing {
                    parser.reset();
                    self.parsing = false;
                }
                if let Some(t) = &mut self.tree {
                    for e in &self.edits {
                        t.edit(e);
                    }
                }
                self.edits.clear();
            }
            let mut input = |_byte: usize, p: Point| -> &[u8] {
                match text.get(p.row) {
                    Some(line) if p.column < line.len() => &line.as_bytes()[p.column..],
                    Some(_) if p.row + 1 < text.len() => b"\n",
                    _ => &[],
                }
            };
            let mut stop = |_: &tree_sitter::ParseState| {
                if std::time::Instant::now() > deadline {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let opts = ParseOptions::new().progress_callback(&mut stop);
            match parser.parse_with_options(&mut input, self.tree.as_ref(), Some(opts)) {
                Some(t) => {
                    self.tree = Some(t);
                    self.dirty = false;
                    self.parsing = false;
                    self.tree_gen += 1;
                }
                None => {
                    self.parsing = true;
                    done = false;
                }
            }
        }
        let key = (self.tree_gen, syn.theme_gen, from, to);
        if self.made_for == Some(key) {
            return done;
        }
        let Some(tree) = &self.tree else {
            return done;
        };
        self.spans = highlight_lines(tree, &query, &syn.theme, text, from, to);
        self.first = from;
        self.made_for = Some(key);
        done
    }

    /// No usable tree-sitter grammar: highlight with the regex fallback, if the language
    /// has one.
    fn ensure_fallback(
        &mut self,
        syn: &Syntax,
        text: &[String],
        to: usize,
        deadline: std::time::Instant,
    ) -> bool {
        self.spans.clear();
        if !self.fallback_tried {
            self.fallback_tried = true;
            self.fallback = syn
                .fallback_for(&self.syntax)
                .map(crate::fallback::Fallback::new);
        }
        let Some(f) = &mut self.fallback else {
            self.using_fallback = false;
            return true;
        };
        self.using_fallback = true;
        f.ensure(&syn.scopes, &syn.theme, syn.theme_gen, text, to, deadline)
    }
}

/// Node text for query predicates (`#eq?`, `#match?`), read straight from the lines.
struct Lines<'a>(&'a [String]);

impl<'a> TextProvider<&'a [u8]> for Lines<'a> {
    type I = std::vec::IntoIter<&'a [u8]>;
    fn text(&mut self, node: Node) -> Self::I {
        let (s, e) = (node.start_position(), node.end_position());
        let mut out = vec![];
        for row in s.row..=e.row.min(self.0.len().saturating_sub(1)) {
            let line = self.0[row].as_bytes();
            let a = if row == s.row {
                s.column.min(line.len())
            } else {
                0
            };
            let b = if row == e.row {
                e.column.min(line.len())
            } else {
                line.len()
            };
            if a < b {
                out.push(&line[a..b]);
            }
            if row < e.row {
                out.push(&b"\n"[..]);
            }
        }
        out.into_iter()
    }
}

/// Run the highlight query over lines `from..to` and turn captures into per-line spans.
/// Inner nodes win over the nodes containing them; for the same node, the earliest pattern
/// in the query wins (tree-sitter's usual precedence).
fn highlight_lines(
    tree: &Tree,
    query: &Query,
    theme: &Theme,
    text: &[String],
    from: usize,
    to: usize,
) -> Vec<Vec<Span>> {
    let mut out: Vec<Vec<Span>> = vec![vec![]; to.saturating_sub(from)];
    if from >= to {
        return out;
    }
    let start_byte: usize = text[..from].iter().map(|l| l.len() + 1).sum();
    let end_byte: usize = start_byte + text[from..to].iter().map(|l| l.len() + 1).sum::<usize>();
    let styles: Vec<Option<Style>> = query
        .capture_names()
        .iter()
        .map(|n| theme.style_for(n))
        .collect();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start_byte..end_byte);
    let mut caps = cursor.captures(query, tree.root_node(), Lines(text));
    // (start byte, end byte, pattern, style, start point, end point)
    let mut found: Vec<(usize, usize, usize, Style, Point, Point)> = vec![];
    while let Some((m, i)) = caps.next() {
        let c = m.captures()[*i];
        let Some(style) = styles[c.index as usize] else {
            continue;
        };
        let n = c.node;
        found.push((
            n.start_byte(),
            n.end_byte(),
            m.pattern_index,
            style,
            n.start_position(),
            n.end_position(),
        ));
    }
    // Paint outer nodes first, inner ones over them; for the same node, later patterns first
    // so the earliest pattern ends up on top.
    found.sort_by_key(|f| (f.0, Reverse(f.1), Reverse(f.2)));
    let mut paint: Vec<Vec<Option<Style>>> = vec![vec![]; out.len()];
    for (_, _, _, style, s, e) in found {
        for row in s.row.max(from)..=e.row.min(to - 1) {
            let len = text[row].len();
            let line = &mut paint[row - from];
            if line.is_empty() {
                line.resize(len, None);
            }
            let a = if row == s.row { s.column.min(len) } else { 0 };
            let b = if row == e.row { e.column.min(len) } else { len };
            for cell in &mut line[a..b] {
                *cell = Some(style);
            }
        }
    }
    for (i, cells) in paint.into_iter().enumerate() {
        let mut spans: Vec<Span> = vec![];
        let mut b = 0;
        while b < cells.len() {
            let st = cells[b];
            let mut e = b + 1;
            while e < cells.len() && cells[e] == st {
                e += 1;
            }
            if let Some(st) = st {
                spans.push((b, e, st));
            }
            b = e;
        }
        out[i] = spans;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.split('\n').map(String::from).collect()
    }

    #[test]
    fn themes_parse_and_fall_back_through_capture_parents() {
        let t = Theme::parse(
            "name = \"T\"\n[editor]\nforeground = \"#FFFFFF\"\n[syntax]\nfunction = \"#00FF00\"\ncomment = { fg = \"#777777\", italic = true }\n",
            "x",
        )
        .unwrap();
        assert_eq!(
            t.style_for("function.builtin.static").unwrap().fg,
            Color32::from_rgb(0, 255, 0)
        );
        assert!(t.style_for("comment.documentation").unwrap().italic);
        assert!(t.style_for("variable").is_none());
        // Every bundled theme parses and has the basics.
        let s = Syntax::load("Monokai Extended");
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert!(s.theme_names().len() >= 25);
        assert_eq!(s.theme.name, "Monokai Extended");
        assert!(s.theme.style_for("keyword").is_some() && s.theme.style_for("string").is_some());
        let ts = Syntax::load("Tsoding");
        assert_eq!(ts.theme.name, "Tsoding");
        assert_eq!(ts.bg(), Color32::from_rgb(0x18, 0x18, 0x18));
        assert_eq!(
            ts.theme.style_for("keyword.repeat").unwrap().fg,
            Color32::from_rgb(0xFF, 0xDD, 0x33)
        );
        // Plain `keyword` (most grammars' only keyword capture) is the yellow too.
        assert_eq!(
            ts.theme.style_for("keyword").unwrap().fg,
            Color32::from_rgb(0xFF, 0xDD, 0x33)
        );
    }

    #[test]
    fn every_bundled_query_compiles() {
        let s = Syntax::load("Monokai Extended");
        let mut bad = vec![];
        for name in s.grammar_names() {
            let g = s.grammar(&name).unwrap();
            if g.query_src.is_empty() {
                bad.push(format!("{name}: no highlights.scm"));
            } else if let Err(e) = g.query() {
                bad.push(e);
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn markdown_headings_code_and_links_are_colored() {
        let text = lines("# Title\n\n```\ncode here\n```\n\n[label]: https://x.org\n");
        let far = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // The default theme styles headings and links but not code (it borrows strings').
        for theme in ["Monokai Extended", "Tsoding", "Nord"] {
            let s = Syntax::load(theme);
            let mut hl = HlCache::new("markdown".into());
            hl.reset("markdown".into(), Some("markdown".into()));
            assert!(hl.ensure(&s, &text, 0, text.len(), far));
            let style_at = |ln: usize, col: usize| {
                hl.line(ln)
                    .unwrap()
                    .iter()
                    .find(|sp| sp.0 <= col && col < sp.1)
                    .map(|sp| sp.2)
            };
            let want = |c: &str| s.theme.style_for(c).unwrap_or_else(|| panic!("{theme}: {c}"));
            assert_eq!(style_at(0, 3), Some(want("markup.heading")), "{theme}: heading");
            assert_eq!(style_at(3, 2), Some(want("markup.raw")), "{theme}: code block");
            assert_eq!(style_at(6, 12), Some(want("markup.link.url")), "{theme}: link url");
        }
    }

    /// Queries use the capture names themes style (not nvim-treesitter's old `@text.*`).
    #[test]
    fn queries_use_current_capture_names() {
        let old: Vec<&str> = crate::config::RUNTIME_FILES
            .iter()
            .filter(|(n, t)| n.ends_with(".scm") && t.lines().any(|l| {
                !l.trim_start().starts_with(';') && l.contains("@text.")
            }))
            .map(|(n, _)| *n)
            .collect();
        assert!(old.is_empty(), "{old:?}");
    }

    #[test]
    fn highlights_visible_lines_and_reparses_incrementally() {
        let s = Syntax::load("Monokai Extended");
        let mut text = lines("// note\nfn main() {\n    let s = \"hi\";\n}\n");
        let mut hl = HlCache::new("rust".into());
        hl.reset("rust".into(), Some("rust".into()));
        let far = std::time::Instant::now() + std::time::Duration::from_secs(5);
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        let style_at = |hl: &HlCache, ln: usize, col: usize| {
            hl.line(ln)
                .unwrap()
                .iter()
                .find(|sp| sp.0 <= col && col < sp.1)
                .map(|sp| sp.2)
        };
        let comment = s.theme.style_for("comment").unwrap();
        let keyword = s.theme.style_for("keyword").unwrap();
        let string = s.theme.style_for("string").unwrap();
        assert_eq!(style_at(&hl, 0, 3), Some(comment));
        assert_eq!(style_at(&hl, 1, 0), Some(keyword), "fn");
        assert_eq!(style_at(&hl, 2, 13), Some(string), "\"hi\"");
        // Edit: turn line 2's string into a number; report the edit, re-highlight.
        let start = text[..2].iter().map(|l| l.len() + 1).sum::<usize>() + 12;
        text[2] = "    let s = 42;".into();
        hl.edit(start, start + 4, start + 2, (2, 12), (2, 16), (2, 14));
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        assert_eq!(style_at(&hl, 2, 12), s.theme.style_for("number"));
        // Only the requested range is highlighted.
        assert!(hl.ensure(&s, &text, 1, 2, far));
        assert!(hl.line(0).is_none() && hl.line(1).is_some() && hl.line(2).is_none());
    }
}

#[cfg(test)]
mod coverage {
    #[test]
    fn every_language_without_a_builtin_grammar_has_a_fallback() {
        // While a grammar installs (or can't be compiled, or doesn't exist), the file still
        // gets regex highlighting.
        let mut s = super::Syntax::load("Monokai Extended");
        let c = crate::config::Config::builtin();
        s.set_languages(&c.languages);
        let missing: Vec<&str> = c
            .languages
            .iter()
            .filter(|l| !s.has_grammar(l.grammar_name()))
            .filter(|l| s.fallback_for(&l.name).is_none())
            .map(|l| l.name.as_str())
            .collect();
        assert!(missing.is_empty(), "no fallback grammar: {missing:?}");
        // Pins name real grammars.
        for l in &c.languages {
            if let Some(n) = l.fallback_syntax.as_deref().filter(|n| !n.is_empty()) {
                assert!(
                    s.fallback_for(&l.name).is_some(),
                    "{}: unknown `{n}`",
                    l.name
                );
            }
        }
    }

    #[test]
    fn fallback_highlights_and_follows_edits() {
        use super::HlCache;
        let mut s = super::Syntax::load("Monokai Extended");
        s.set_languages(&crate::config::Config::builtin().languages);
        // Kotlin with no tree-sitter grammar installed.
        let mut text: Vec<String> = ["// note", "fun main() {", "    val s = \"hi\"", "}"]
            .iter()
            .map(|l| l.to_string())
            .collect();
        let mut hl = HlCache::new("kotlin".into());
        let far = std::time::Instant::now() + std::time::Duration::from_secs(5);
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        assert_eq!(hl.fallback_name(), Some("Kotlin"));
        let style_at = |hl: &HlCache, ln: usize, col: usize| {
            hl.line(ln)
                .unwrap()
                .iter()
                .find(|sp| sp.0 <= col && col < sp.1)
                .map(|sp| sp.2)
        };
        let comment = s.theme.style_for("comment").unwrap();
        let string = s.theme.style_for("string").unwrap();
        assert_eq!(style_at(&hl, 0, 3), Some(comment));
        assert_eq!(style_at(&hl, 2, 13), Some(string), "{:?}", hl.line(2));
        assert_eq!(
            style_at(&hl, 2, 12),
            Some(string),
            "quotes look like the string"
        );
        let kw = style_at(&hl, 1, 0).expect("`fun` is styled");
        assert_ne!(kw, string);
        assert_ne!(kw, comment);
        // An edit re-highlights from that line down: the comment becomes code.
        text[0] = "val x = 1".into();
        hl.changed_line(0);
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        assert_ne!(style_at(&hl, 0, 1), Some(comment));
        // A language with no fallback either stays plain.
        let mut plain = HlCache::new("Plain Text".into());
        assert!(plain.ensure(&s, &text, 0, text.len(), far));
        assert_eq!(plain.fallback_name(), None);
        assert!(plain.line(0).is_none_or(|l| l.is_empty()));
    }

    /// LakeScript's bundled regex grammar: variables, keywords, types, f-string
    /// interpolation and comments come out styled.
    #[test]
    fn lakescript_is_highlighted() {
        use super::HlCache;
        let mut s = super::Syntax::load("Monokai Extended");
        let c = crate::config::Config::builtin();
        s.set_languages(&c.languages);
        let lang = c.language_for(std::path::Path::new("x.lks"), "").unwrap();
        assert_eq!(lang.name, "lakescript");
        let shebang = c.language_for(std::path::Path::new("tool"), "#!/usr/bin/lks").unwrap();
        assert_eq!(shebang.name, "lakescript");
        let text: Vec<String> = [
            "// a comment",
            "bool prime?(number $n) {",
            "    let $d = 2;",
            "    outln(f\"n = {$n}\");",
            "}",
        ]
        .iter()
        .map(|l| l.to_string())
        .collect();
        let mut hl = HlCache::new("lakescript".into());
        let far = std::time::Instant::now() + std::time::Duration::from_secs(5);
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        assert_eq!(hl.fallback_name(), Some("LakeScript"));
        let at = |ln: usize, col: usize| {
            hl.line(ln)
                .unwrap()
                .iter()
                .find(|sp| sp.0 <= col && col < sp.1)
                .map(|sp| sp.2)
        };
        let style = |c: &str| s.theme.style_for(c).unwrap();
        assert_eq!(at(0, 4), Some(style("comment")));
        assert_eq!(at(3, 12), Some(style("string")), "{:?}", hl.line(3));
        let (kw, var, string) = (at(2, 5), at(2, 9), at(3, 12));
        assert!(kw.is_some() && var.is_some(), "{:?}", hl.line(2));
        assert_ne!(kw, var);
        assert_ne!(at(3, 17), string, "{{$n}} inside the f-string is code");
        assert!(at(1, 0).is_some(), "the return type is styled");
        assert!(at(1, 6).is_some(), "the function name is styled");

        // The other literals, from lakescript's own example.
        let text: Vec<String> = [
            r#"if ($w -match %r"^\d+\"x"i) outln(%x"cat {$p}");"#,
            r##"const $raw = r#"C:\p "q""#; const $s = :active;"##,
            "let $x = f\"{{literal}} {$y}\"; paths::load();",
            "let $q = 'unclosed",
            "let $after = 1;",
        ]
        .iter()
        .map(|l| l.to_string())
        .collect();
        let mut hl = HlCache::new("lakescript".into());
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        let at = |ln: usize, col: usize| {
            hl.line(ln)
                .unwrap()
                .iter()
                .find(|sp| sp.0 <= col && col < sp.1)
                .map(|sp| sp.2)
        };
        let line0 = &text[0];
        let col = |l: &str, pat: &str| l.find(pat).unwrap();
        let regex = at(0, col(line0, "^"));
        assert!(regex.is_some(), "{:?}", hl.line(0));
        assert_eq!(at(0, col(line0, "\"x") + 2), regex, "\\\" stays inside the regex");
        assert_ne!(at(0, col(line0, "outln")), regex, "the flags end it");
        assert_eq!(at(0, col(line0, "cat")), Some(style("string")));
        assert_ne!(at(0, col(line0, "$p")), Some(style("string")));
        assert_ne!(at(0, col(line0, "match")), None, "-match is an operator");
        let line1 = &text[1];
        assert_eq!(at(1, col(line1, "\"q") + 1), Some(style("string")), "raw string");
        assert_ne!(at(1, col(line1, "const $s")), Some(style("string")), "raw string ends");
        assert!(at(1, col(line1, ":active") + 1).is_some(), "symbol");
        let line2 = &text[2];
        assert_ne!(at(2, col(line2, "{{")), at(2, col(line2, "$y")), "{{{{ is literal");
        assert!(at(2, col(line2, "paths")).is_some(), "module path");
        assert_ne!(at(4, 0), Some(style("string")), "an unclosed string ends with its line");
    }

    /// LakeScript 0.7.1's sample script: `@` fields, contextual words (`command`, `flag`,
    /// `count`, `new`, `Self`, `implement`), enum values, doc comments, test operators.
    #[test]
    fn lakescript_contextual_words_and_fields() {
        use super::HlCache;
        let mut s = super::Syntax::load("Monokai Extended");
        s.set_languages(&crate::config::Config::builtin().languages);
        let text: Vec<String> = r#"#!/usr/bin/lks
use std/process;

/// A tiny todo list
command Todo {
    /// More output
    flag(v, count) number @verbose;
    subcommand Add {
        string @task;
        flag(p) Priority @priority = Priority::NORMAL;
    }
}

enum Priority { LOW, NORMAL, HIGH; }

interface IShape {
    number area();
    string describe() = f"area {@area().round(2)}";
}

class Circle extends Base {
    pub const number @radius;
    number $made = 0;
    pub new(@radius) {
        @super('circle');
        Self::$made += 1;
    }
    implement IShape { number area() = 3.14 * @radius ** 2; }
    implement IMul<number, Circle> { Circle mul(number $n) = Circle(@radius * $n); }
}

const $args = Todo::parse();
const $files = %w[a.txt b.txt];
if (-f $files[0] && $files[0] -like '*.txt') {
    %x"git add {$files}".pipe(%x"sort").run();
}
const $n = (try process::shell('ls | wc -l').output()).orElse('0');
outln($args.subcommand?.is(Todo::Add) ?? false);
command $c = %x"ls";
if ($files.empty?()) { outln(@); }"#
            .lines()
            .map(|l| l.to_string())
            .collect();
        let mut hl = HlCache::new("lakescript".into());
        let far = std::time::Instant::now() + std::time::Duration::from_secs(5);
        assert!(hl.ensure(&s, &text, 0, text.len(), far));
        let ln = |pat: &str| text.iter().position(|l| l.contains(pat)).unwrap();
        // The style at the start of `word` on the first line containing `line`.
        let at = |line: &str, word: &str| {
            let n = ln(line);
            let col = text[n].find(word).unwrap();
            hl.line(n)
                .unwrap()
                .iter()
                .find(|sp| sp.0 <= col && col < sp.1)
                .map(|sp| sp.2)
        };
        let style = |c: &str| s.theme.style_for(c);
        let check = |line: &str, word: &str, cap: &str| {
            assert!(style(cap).is_some(), "the theme styles {cap}");
            assert_eq!(at(line, word), style(cap), "`{word}` in `{line}` is {cap}");
        };
        // The checks below tell these apart.
        for (a, b) in [
            ("keyword", "type.builtin"),
            ("keyword", "type"),
            ("property", "function.call"),
            ("constant", "type"),
        ] {
            assert_ne!(style(a), style(b), "{a} and {b} look alike in this theme");
        }
        assert_ne!(at("flag(v, count)", "v,"), style("keyword"), "`v` is a plain name");
        assert_ne!(at("const $n", "orElse"), style("keyword"));
        // Doc comments get their own capture (this theme colors it like a comment).
        let mut m = crate::fallback::ScopeMap::default();
        m.merge(include_str!("../runtime/scopes.toml")).unwrap();
        assert_eq!(
            m.capture("comment.line.documentation.lakescript"),
            Some("comment.documentation")
        );
        check("/// A tiny", "A tiny", "comment.documentation");
        check("/// More", "More", "comment.documentation");
        check("command Todo", "command", "keyword");
        check("command Todo", "Todo", "type");
        check("command $c", "command", "type.builtin");
        check("flag(v, count)", "flag", "keyword");
        check("flag(v, count)", "count", "keyword");
        check("flag(v, count)", "number", "type.builtin");
        check("flag(v, count)", "@verbose", "property");
        check("subcommand Add", "subcommand", "keyword");
        check("subcommand?.is", "subcommand", "property");
        check("Priority::NORMAL", "NORMAL", "constant");
        check("Priority::NORMAL", "Priority @", "type");
        check("enum Priority", "LOW", "constant");
        check("interface IShape", "IShape", "type");
        check("number area();", "area", "function");
        check("string describe()", "describe", "function");
        assert_ne!(at("string describe()", "@area"), style("string"), "interpolated code");
        check("pub new(", "new", "keyword");
        check("@super(", "@super", "variable.builtin");
        check("Self::$made", "Self", "keyword");
        assert!(at("Self::$made", "$made").is_some());
        check("implement IShape {", "implement", "keyword");
        check("implement IShape {", "area", "function");
        check("implement IMul", "IMul", "type.builtin");
        check("implement IMul", "mul", "function");
        check("class Circle", "extends", "keyword");
        check("class Circle", "Base", "type");
        check("%w[", "a.txt", "string");
        check("-f $files", "-f", "keyword.operator");
        check("-f $files", "-like", "keyword.operator");
        check("git add", "git add", "string");
        assert_ne!(at("git add", "$files"), style("string"), "{{$files}} is code");
        check("process::shell", "process", "module");
        check("process::shell", "shell", "function.call");
        check("outln($args", "Add", "type");
        check("empty?()", "empty?", "function.call");
        check("use std/process", "use", "keyword.import");
        check("use std/process", "process", "module");
        assert!(at("outln(@)", "@)").is_some(), "a lone @ is the object");
    }

    #[test]
    fn scope_map_uses_the_longest_prefix() {
        let mut m = crate::fallback::ScopeMap::default();
        m.merge("[scopes]\n\"keyword\" = \"keyword\"\n\"keyword.operator\" = \"operator\"\n\"punctuation.definition.string\" = \"\"\n").unwrap();
        assert_eq!(m.capture("keyword.control.kotlin"), Some("keyword"));
        assert_eq!(
            m.capture("keyword.operator.arithmetic.kotlin"),
            Some("operator")
        );
        assert_eq!(
            m.capture("punctuation.definition.string.begin.kotlin"),
            Some("")
        );
        assert_eq!(m.capture("meta.block.kotlin"), None);
    }

    #[test]
    fn every_language_grammar_exists() {
        let s = super::Syntax::load("Monokai Extended");
        let c = crate::config::Config::builtin();
        let mut plain = vec![];
        for l in &c.languages {
            let g = l.grammar_name();
            if g.is_empty() {
                plain.push(l.name.as_str());
            } else {
                // Built in, or installable from a recipe.
                assert!(
                    s.has_grammar(g) || c.grammar_recipe(g).is_some(),
                    "{}: unknown grammar `{g}`",
                    l.name
                );
            }
        }
        // Every recipe has a bundled highlight query (installed grammars use it).
        for r in &c.grammars {
            assert!(
                crate::config::RUNTIME_FILES
                    .iter()
                    .any(|(p, _)| *p == format!("queries/{}/highlights.scm", r.name)),
                "grammar recipe `{}` has no runtime/queries/{0}/highlights.scm",
                r.name
            );
        }
        plain.sort();
        // Known gaps: no bundled tree-sitter grammar yet (shown as plain text).
        assert_eq!(
            plain,
            [
                "awk",
                "bibtex",
                "clojure",
                "crystal",
                "dotenv",
                "graphql",
                "just",
                "lakescript",
                "latex",
                "meson",
                "nim",
                "perl",
                "terraform",
                "typst",
                "wgsl"
            ]
        );
    }
}
