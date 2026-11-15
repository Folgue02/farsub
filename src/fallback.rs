//! Fallback highlighting with syntect (Sublime-style regex grammars), for files tree-sitter
//! can't highlight: the language has no tree-sitter grammar yet, its grammar is still
//! installing, or it can't be compiled here (no C compiler).
//!
//! Grammars are two-face's set (bat's) plus runtime/syntaxes, precompiled by build.rs and
//! only deserialized when a file first needs one. Their TextMate scopes are turned into the
//! theme's capture names by `scopes.toml`, so every theme works as is.
//!
//! Parsing is line by line from the top; the parser state is kept every `CHECKPOINT` lines
//! so an edit only redoes the text from the changed line down to the bottom of the view.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::config::LanguageConfig;
use crate::highlight::{Span, Style, Theme};

/// Every fallback grammar (loaded on first use).
pub fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| {
        syntect::dumps::from_uncompressed_data(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/syntaxes.packdump"
        )))
        .expect("bundled syntax dump")
    })
}

/// The fallback grammar for a language (an index into `syntaxes()`): the one it names in
/// `fallback_syntax`, else the one titled or named like it, else the one claiming one of its
/// file extensions or file names.
pub fn find(l: &LanguageConfig) -> Option<usize> {
    let ss = syntaxes();
    let index = |name: &str| ss.syntaxes().iter().position(|s| s.name == name);
    if let Some(name) = &l.fallback_syntax {
        return if name.is_empty() { None } else { index(name) };
    }
    let by_name = |n: &str| {
        ss.syntaxes()
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(n))
    };
    by_name(l.title())
        .or_else(|| by_name(&l.name))
        .or_else(|| {
            l.extensions
                .iter()
                .chain(&l.filenames)
                .find_map(|e| ss.find_syntax_by_extension(e))
                .and_then(|s| index(&s.name))
        })
        .filter(|&i| ss.syntaxes()[i].name != "Plain Text")
}

// ---------------------------------------------------------------- scope → capture

/// Scope prefix → theme capture (`scopes.toml`). An empty capture falls through to the
/// scope around it.
#[derive(Clone, Debug, Default)]
pub struct ScopeMap {
    map: HashMap<String, String>,
}

#[derive(Deserialize)]
struct ScopeFile {
    #[serde(default)]
    scopes: HashMap<String, String>,
}

impl ScopeMap {
    /// Add (or override) rules from a `scopes.toml`.
    pub fn merge(&mut self, text: &str) -> Result<(), String> {
        let f: ScopeFile = toml::from_str(text).map_err(|e| e.message().to_string())?;
        self.map.extend(f.scopes);
        Ok(())
    }

    /// The capture for one scope: its longest prefix with a rule. `None`: no rule.
    pub fn capture(&self, scope: &str) -> Option<&str> {
        let mut s = scope;
        loop {
            if let Some(c) = self.map.get(s) {
                return Some(c);
            }
            s = &s[..s.rfind('.')?];
        }
    }
}

/// The style of a token: its innermost scope whose capture the theme styles.
fn style_of(
    stack: &ScopeStack,
    cache: &mut HashMap<Scope, Option<Style>>,
    map: &ScopeMap,
    theme: &Theme,
) -> Option<Style> {
    stack.as_slice().iter().rev().find_map(|scope| {
        *cache.entry(*scope).or_insert_with(|| {
            map.capture(&scope.build_string())
                .filter(|c| !c.is_empty())
                .and_then(|c| theme.style_for(c))
        })
    })
}

// ---------------------------------------------------------------- per-buffer state

/// Lines longer than this aren't highlighted (regex grammars get very slow on them).
const MAX_LINE: usize = 4000;
const CHECKPOINT: usize = 64;

/// A buffer's fallback highlighting: parser states and the spans of lines `0..lines.len()`.
pub struct Fallback {
    syntax: usize,
    /// Parser state at the start of every `CHECKPOINT`-th line.
    checkpoints: Vec<(ParseState, ScopeStack)>,
    /// Running state at the start of line `lines.len()`.
    frontier: Option<(ParseState, ScopeStack)>,
    lines: Vec<Vec<Span>>,
    /// Style per scope for the current theme.
    styles: HashMap<Scope, Option<Style>>,
    theme_gen: Option<u64>,
}

impl Fallback {
    pub fn new(syntax: usize) -> Self {
        Fallback {
            syntax,
            checkpoints: vec![],
            frontier: None,
            lines: vec![],
            styles: HashMap::new(),
            theme_gen: None,
        }
    }

    /// The grammar's name ("Kotlin").
    #[cfg(test)]
    pub fn name(&self) -> &'static str {
        &syntaxes().syntaxes()[self.syntax].name
    }

    /// Text changed on line `line`: redo it and everything below.
    pub fn invalidate(&mut self, line: usize) {
        if line >= self.lines.len() {
            return;
        }
        let k = line / CHECKPOINT;
        self.checkpoints.truncate(k + 1);
        self.lines.truncate(k * CHECKPOINT);
        self.frontier = None;
    }

    pub fn line(&self, ln: usize) -> Option<&Vec<Span>> {
        self.lines.get(ln)
    }

    /// Highlight lines up to `upto`, stopping at `deadline`. Returns true when done; the
    /// rest fills in over later frames, so a jump deep into a huge file never freezes.
    pub fn ensure(
        &mut self,
        map: &ScopeMap,
        theme: &Theme,
        theme_gen: u64,
        text: &[String],
        upto: usize,
        deadline: std::time::Instant,
    ) -> bool {
        if self.theme_gen != Some(theme_gen) {
            self.theme_gen = Some(theme_gen);
            self.styles.clear();
            self.lines.clear();
            self.checkpoints.truncate(1);
            self.frontier = None;
        }
        let upto = upto.min(text.len());
        if self.lines.len() >= upto {
            return true;
        }
        let ss = syntaxes();
        if self.frontier.is_none() {
            self.frontier = Some(match self.checkpoints.get(self.lines.len() / CHECKPOINT) {
                Some(st) => st.clone(),
                None => {
                    self.lines.clear();
                    self.checkpoints.clear();
                    (
                        ParseState::new(&ss.syntaxes()[self.syntax]),
                        ScopeStack::new(),
                    )
                }
            });
        }
        let (ps, stack) = self.frontier.as_mut().unwrap();
        let mut line = String::new();
        while self.lines.len() < upto {
            let i = self.lines.len();
            if i % CHECKPOINT == 0 && self.checkpoints.len() == i / CHECKPOINT {
                self.checkpoints.push((ps.clone(), stack.clone()));
            }
            let src = &text[i];
            let mut spans: Vec<Span> = vec![];
            if src.len() <= MAX_LINE {
                line.clear();
                line.push_str(src);
                line.push('\n');
                let ops = ps.parse_line(&line, ss).unwrap_or_default();
                let mut push = |a: usize, b: usize, st: Option<Style>| {
                    let b = b.min(src.len());
                    let Some(st) = st.filter(|_| a < b) else {
                        return;
                    };
                    match spans.last_mut() {
                        Some(last) if last.1 == a && last.2 == st => last.1 = b,
                        _ => spans.push((a, b, st)),
                    }
                };
                let mut at = 0;
                let mut cur = style_of(stack, &mut self.styles, map, theme);
                for (off, op) in ops {
                    if off > at {
                        push(at, off, cur);
                        at = off;
                    }
                    let _ = stack.apply(&op);
                    cur = style_of(stack, &mut self.styles, map, theme);
                }
                push(at, src.len(), cur);
            }
            self.lines.push(spans);
            if i % 32 == 31 && std::time::Instant::now() > deadline {
                return self.lines.len() >= upto;
            }
        }
        true
    }
}
