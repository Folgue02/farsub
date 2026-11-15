//! Text buffer with Sublime-style editing operations and grouped undo/redo.

use crate::commands::Command;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::highlight::HlCache;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    pub line: usize,
    /// Column in chars (not bytes).
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Pos { line, col }
    }
}

#[derive(Clone, Debug)]
struct Edit {
    start: Pos,
    removed: String,
    inserted: String,
}

#[derive(Clone, Debug)]
struct Group {
    edits: Vec<Edit>,
    before: (Pos, Option<Pos>),
    after: (Pos, Option<Pos>),
    typing: bool,
    at: Instant,
}

pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

pub fn byte_idx(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map(|(i, _)| i).unwrap_or(s.len())
}

pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if is_word(c) {
        1
    } else {
        2
    }
}

pub fn end_of(start: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => Pos::new(start.line, start.col + char_len(text)),
        Some(i) => Pos::new(
            start.line + text.matches('\n').count(),
            char_len(&text[i + 1..]),
        ),
    }
}

pub fn comment_token(path: Option<&Path>) -> Option<&'static str> {
    let ext = path?.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "js" | "jsx" | "ts" | "tsx"
        | "go" | "java" | "kt" | "swift" | "zig" | "cs" | "scala" | "dart" | "php" | "proto"
        | "jsonc" | "scss" | "less" | "glsl" | "wgsl" | "m" | "mm" => "//",
        "py" | "sh" | "bash" | "zsh" | "fish" | "rb" | "toml" | "yaml" | "yml" | "pl" | "r"
        | "nix" | "ex" | "exs" | "conf" | "cmake" | "ps1" | "tf" | "jl" | "nim" | "mk" => "#",
        "lua" | "sql" | "hs" | "elm" | "ada" => "--",
        "vim" => "\"",
        "lisp" | "clj" | "cljs" | "el" | "scm" | "asm" | "s" | "ini" => ";",
        "tex" | "erl" => "%",
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(usize),
}

/// Guess a file's indentation like Sublime: tabs vs spaces by majority of indented lines,
/// and the space width from the most common indentation *change* between lines.
pub fn guess_indent(lines: &[String]) -> Option<Indent> {
    let mut tabs = 0;
    let mut spaces = 0;
    let mut deltas = [0usize; 9];
    let mut prev = 0usize;
    for l in lines.iter().take(5000) {
        if l.trim().is_empty() {
            continue;
        }
        let lead: String = l.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if lead.starts_with('\t') {
            tabs += 1;
            continue;
        }
        let n = lead.len();
        if n > 0 && !l.trim_start().starts_with('*') {
            spaces += 1;
        }
        // Block-comment continuation lines (" * foo") don't count.
        if !l.trim_start().starts_with('*') {
            let d = n.abs_diff(prev);
            if (2..=8).contains(&d) {
                deltas[d] += 1;
            }
            prev = n;
        }
    }
    if tabs == 0 && spaces == 0 {
        return None;
    }
    if tabs > spaces {
        return Some(Indent::Tabs);
    }
    let best = (2..=8).max_by_key(|&d| (deltas[d], d == 4, d == 2))?;
    Some(Indent::Spaces(if deltas[best] == 0 { 4 } else { best }))
}

pub struct Buffer {
    pub id: usize,
    pub path: Option<PathBuf>,
    pub lines: Vec<String>,
    pub cursor: Pos,
    pub anchor: Option<Pos>,
    pub want_col: usize,
    pub scroll_y: usize,
    pub scroll_x: usize,
    pub modified: bool,
    pub crlf: bool,
    pub read_only: bool,
    pub version: i32,
    pub lsp_dirty: bool,
    /// Edits since the last LSP sync, as (start, end, text) with columns in UTF-16 units.
    pub lsp_changes: Vec<(Pos, Pos, String)>,
    /// Too many edits were queued; send the full text instead.
    pub lsp_full: bool,
    pub lsp_client: Option<usize>,
    /// Name of the configured language (see config.rs), if any.
    pub lang: Option<String>,
    pub hl: HlCache,
    pub indent_unit: String,
    pub comment: Option<String>,
    /// The file's own indentation was detected (otherwise config defaults apply).
    pub indent_detected: bool,
    /// Display width of a tab character.
    pub tab_size: usize,
    /// Indentation was set by hand (kept in the workspace).
    pub indent_override: bool,
    /// The syntax was picked by hand (kept in the workspace).
    pub syntax_override: bool,
    /// Editor group (pane column) the buffer's tab lives in.
    pub group: usize,
    /// The file was deleted on disk while open (the buffer keeps its contents).
    pub deleted: bool,
    /// The file's encoding (saved back in it).
    pub encoding: crate::encoding::TextEncoding,
    /// The file on disk as last read or saved (a change means someone else wrote it).
    pub disk: Option<crate::fileops::DiskStamp>,
    /// Word wrap chosen for this file (View: Toggle Word Wrap); None = the setting.
    pub word_wrap: Option<bool>,
    /// Tab name for a buffer that isn't a file ("Find Results").
    pub title: Option<String>,
    /// Set on the Find Results buffer: what each of its lines points at.
    pub results: Option<Box<crate::search::Results>>,
    /// With word wrap: which visual row of the `scroll_y` line is at the top.
    pub scroll_row: usize,
    /// Pixels the top row is scrolled past (smooth scrolling between whole rows).
    pub scroll_px: f32,
    /// Visual column kept across Up/Down in wrapped text, valid while the cursor is still
    /// at the position it was set for.
    want_vx: Option<(Pos, usize)>,
    undo: Vec<Group>,
    redo: Vec<Group>,
    pending: Vec<Edit>,
    seal: bool,
}

impl Buffer {
    pub fn from_text(id: usize, path: Option<PathBuf>, text: &str, hl: HlCache) -> Buffer {
        let crlf = text.contains("\r\n");
        let norm = if crlf {
            text.replace("\r\n", "\n")
        } else {
            text.to_string()
        };
        let lines: Vec<String> = norm.split('\n').map(String::from).collect();
        let guess = guess_indent(&lines);
        let indent_detected = guess.is_some();
        let indent_unit = match guess {
            Some(Indent::Tabs) => "\t".to_string(),
            Some(Indent::Spaces(n)) => " ".repeat(n),
            None => "    ".to_string(),
        };
        // A file indented with N spaces uses N as its tab width too (like Sublime).
        let tab_size = match guess {
            Some(Indent::Spaces(n)) => n,
            _ => 4,
        };
        let comment = comment_token(path.as_deref()).map(String::from);
        let lang = None;
        Buffer {
            id,
            path,
            lines,
            cursor: Pos::default(),
            anchor: None,
            want_col: 0,
            scroll_y: 0,
            scroll_x: 0,
            modified: false,
            crlf,
            read_only: false,
            version: 1,
            lsp_dirty: false,
            lsp_changes: vec![],
            lsp_full: false,
            lsp_client: None,
            lang,
            hl,
            indent_unit,
            comment,
            indent_detected,
            tab_size,
            indent_override: false,
            syntax_override: false,
            group: 0,
            deleted: false,
            encoding: Default::default(),
            disk: None,
            word_wrap: None,
            title: None,
            results: None,
            scroll_row: 0,
            scroll_px: 0.0,
            want_vx: None,
            undo: vec![],
            redo: vec![],
            pending: vec![],
            seal: true,
        }
    }

    pub fn text(&self) -> String {
        let s = self.lines.join("\n");
        if self.crlf {
            s.replace('\n', "\r\n")
        } else {
            s
        }
    }

    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| self.title.clone())
            .unwrap_or_else(|| "untitled".into())
    }

    /// Append text to a generated buffer (Find Results): no undo, not "modified".
    pub fn append_generated(&mut self, text: &str) {
        let l = self.lines.len() - 1;
        let end = Pos::new(l, char_len(&self.lines[l]));
        self.raw_insert(end, text);
        self.lsp_changes.clear();
        self.version += 1;
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let Some(path) = &self.path else {
            anyhow::bail!("buffer has no path")
        };
        let bytes = self
            .encoding
            .encode(&self.text())
            .map_err(|e| anyhow::anyhow!(e))?;
        crate::fileops::write_atomic(path, &bytes)?;
        self.disk = crate::fileops::DiskStamp::of(path);
        self.modified = false;
        self.seal = true;
        Ok(())
    }

    /// Replace the text with what's on disk now, as one undoable edit: only the part that
    /// differs is replaced, so the cursor and view stay put where the text didn't change.
    pub fn reload_from(&mut self, text: &str) {
        let crlf = text.contains("\r\n");
        let new = if crlf {
            text.replace("\r\n", "\n")
        } else {
            text.to_string()
        };
        let old = self.lines.join("\n");
        let pre = old
            .char_indices()
            .zip(new.chars())
            .find(|((_, a), b)| a != b)
            .map(|((i, _), _)| i)
            .unwrap_or(old.len().min(new.len()));
        let max_suf = (old.len() - pre).min(new.len() - pre);
        let mut suf = old
            .bytes()
            .rev()
            .zip(new.bytes().rev())
            .take(max_suf)
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suf) || !new.is_char_boundary(new.len() - suf) {
            suf -= 1;
        }
        let pos_of = |byte: usize| {
            let line = old[..byte].matches('\n').count();
            let start = old[..byte].rfind('\n').map_or(0, |i| i + 1);
            Pos::new(line, old[start..byte].chars().count())
        };
        let (a, b) = (pos_of(pre), pos_of(old.len() - suf));
        let before = self.begin();
        self.seal = true;
        self.delete(a, b);
        self.insert(a, &new[pre..new.len() - suf]);
        self.cursor = self.clamp(self.cursor);
        self.anchor = self.anchor.map(|p| self.clamp(p));
        self.commit(before, false);
        self.seal = true;
        self.crlf = crlf;
        self.modified = false;
        if let Some(p) = &self.path {
            self.disk = crate::fileops::DiskStamp::of(p);
        }
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.comment = comment_token(Some(&path)).map(String::from);
        self.lang = None;
        // A new file name (Save As): what was on disk under the old one no longer applies.
        if self.path.as_ref() != Some(&path) {
            self.disk = None;
        }
        self.path = Some(path);
    }

    /// Apply a language's settings: name, comment token and (if the file itself gives no
    /// hint) indentation.
    pub fn apply_language(
        &mut self,
        lang: Option<&crate::config::LanguageConfig>,
        editor: &crate::config::EditorConfig,
    ) {
        self.lang = lang.map(|l| l.name.clone());
        if let Some(c) = lang.and_then(|l| l.comment.clone()) {
            self.comment = Some(c);
        }
        let size = lang
            .and_then(|l| l.tab_size)
            .unwrap_or(editor.tab_size)
            .max(1);
        let detected_spaces = self.indent_detected && self.indent_unit != "\t";
        if !self.indent_override && !detected_spaces {
            self.tab_size = size;
        }
        if !self.indent_detected && !self.indent_override {
            let tabs = lang.and_then(|l| l.use_tabs).unwrap_or(editor.use_tabs);
            self.indent_unit = if tabs { "\t".into() } else { " ".repeat(size) };
        }
    }

    // ---------- indentation ----------

    pub fn indent_kind(&self) -> Indent {
        if self.indent_unit == "\t" {
            Indent::Tabs
        } else {
            Indent::Spaces(self.indent_unit.len())
        }
    }

    /// Indent using spaces/tabs and/or change the width (Sublime's Indentation menu).
    pub fn set_indent(&mut self, kind: Indent) {
        self.indent_override = true;
        match kind {
            Indent::Tabs => self.indent_unit = "\t".into(),
            Indent::Spaces(n) => {
                self.indent_unit = " ".repeat(n.clamp(1, 16));
                self.tab_size = n.clamp(1, 16);
            }
        }
    }

    pub fn set_tab_width(&mut self, n: usize) {
        let n = n.clamp(1, 16);
        self.tab_size = n;
        if self.indent_unit != "\t" {
            self.indent_unit = " ".repeat(n);
        }
        self.indent_override = true;
    }

    /// Re-detect indentation from the buffer's contents. Returns what was found.
    pub fn guess_indentation(&mut self) -> Option<Indent> {
        let g = guess_indent(&self.lines);
        if let Some(k) = g {
            self.set_indent(k);
            self.indent_override = false;
        }
        g
    }

    /// Rewrite leading whitespace as tabs or spaces (one undo step). Returns lines changed.
    pub fn convert_indentation(&mut self, to_tabs: bool) -> usize {
        if self.read_only {
            return 0;
        }
        let ts = self.tab_size.max(1);
        let mut edits = vec![];
        for (i, l) in self.lines.iter().enumerate() {
            let lead: String = l.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            if lead.is_empty() {
                continue;
            }
            let width = lead
                .chars()
                .fold(0, |w, c| if c == '\t' { w + ts - w % ts } else { w + 1 });
            let new = if to_tabs {
                "\t".repeat(width / ts) + &" ".repeat(width % ts)
            } else {
                " ".repeat(width)
            };
            if new != lead {
                edits.push((Pos::new(i, 0), Pos::new(i, char_len(&lead)), new));
            }
        }
        let n = edits.len();
        self.apply_edits(edits);
        if to_tabs {
            self.indent_unit = "\t".into();
        } else {
            self.indent_unit = " ".repeat(ts);
        }
        self.indent_override = true;
        n
    }

    /// Indentation width (display columns) of a line; blank lines take the next non-blank's,
    /// like Sublime's indent guides.
    pub fn indent_width(&self, l: usize) -> usize {
        let ts = self.tab_size.max(1);
        let width = |s: &str| {
            s.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .fold(0, |w, c| if c == '\t' { w + ts - w % ts } else { w + 1 })
        };
        let line = &self.lines[l];
        if !line.trim().is_empty() {
            return width(line);
        }
        let next = self.lines[l + 1..]
            .iter()
            .take(200)
            .find(|x| !x.trim().is_empty())
            .map(|x| width(x))
            .unwrap_or(0);
        let prev = self.lines[..l]
            .iter()
            .rev()
            .take(200)
            .find(|x| !x.trim().is_empty())
            .map(|x| width(x))
            .unwrap_or(0);
        next.min(prev.max(next))
    }

    // ---------- geometry ----------

    pub fn line_len(&self, l: usize) -> usize {
        self.lines.get(l).map(|s| char_len(s)).unwrap_or(0)
    }

    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len().saturating_sub(1));
        Pos::new(line, p.col.min(self.line_len(line)))
    }

    pub fn last_pos(&self) -> Pos {
        let l = self.lines.len() - 1;
        Pos::new(l, self.line_len(l))
    }

    pub fn char_at(&self, p: Pos) -> Option<char> {
        self.lines.get(p.line)?.chars().nth(p.col)
    }

    fn char_before(&self, p: Pos) -> Option<char> {
        if p.col == 0 {
            None
        } else {
            self.char_at(Pos::new(p.line, p.col - 1))
        }
    }

    pub fn selection(&self) -> Option<(Pos, Pos)> {
        let a = self.anchor?;
        if a == self.cursor {
            return None;
        }
        Some(if a < self.cursor {
            (a, self.cursor)
        } else {
            (self.cursor, a)
        })
    }

    pub fn text_range(&self, a: Pos, b: Pos) -> String {
        if a >= b {
            return String::new();
        }
        if a.line == b.line {
            let l = &self.lines[a.line];
            return l[byte_idx(l, a.col)..byte_idx(l, b.col)].to_string();
        }
        let first = &self.lines[a.line];
        let mut s = first[byte_idx(first, a.col)..].to_string();
        for l in a.line + 1..b.line {
            s.push('\n');
            s.push_str(&self.lines[l]);
        }
        s.push('\n');
        let last = &self.lines[b.line];
        s.push_str(&last[..byte_idx(last, b.col)]);
        s
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(a, b)| self.text_range(a, b))
    }

    pub fn word_bounds(&self, p: Pos) -> Option<(Pos, Pos)> {
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        let mut s = p.col.min(chars.len());
        let mut e = s;
        while s > 0 && is_word(chars[s - 1]) {
            s -= 1;
        }
        while e < chars.len() && is_word(chars[e]) {
            e += 1;
        }
        (s < e).then(|| (Pos::new(p.line, s), Pos::new(p.line, e)))
    }

    pub fn word_start(&self, p: Pos) -> Pos {
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        let mut s = p.col.min(chars.len());
        while s > 0 && is_word(chars[s - 1]) {
            s -= 1;
        }
        Pos::new(p.line, s)
    }

    pub fn word_left_pos(&self, p: Pos) -> Pos {
        if p.col == 0 {
            return if p.line > 0 {
                Pos::new(p.line - 1, self.line_len(p.line - 1))
            } else {
                p
            };
        }
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        let mut i = p.col.min(chars.len());
        while i > 0 && class(chars[i - 1]) == 0 {
            i -= 1;
        }
        if i > 0 {
            let k = class(chars[i - 1]);
            while i > 0 && class(chars[i - 1]) == k {
                i -= 1;
            }
        }
        Pos::new(p.line, i)
    }

    pub fn word_right_pos(&self, p: Pos) -> Pos {
        let chars: Vec<char> = self.lines[p.line].chars().collect();
        if p.col >= chars.len() {
            return if p.line + 1 < self.lines.len() {
                Pos::new(p.line + 1, 0)
            } else {
                p
            };
        }
        let mut i = p.col;
        while i < chars.len() && class(chars[i]) == 0 {
            i += 1;
        }
        if i < chars.len() {
            let k = class(chars[i]);
            while i < chars.len() && class(chars[i]) == k {
                i += 1;
            }
        }
        Pos::new(p.line, i)
    }

    // ---------- primitive edits ----------

    fn u16pos(&self, p: Pos) -> Pos {
        let col = self.lines[p.line]
            .chars()
            .take(p.col)
            .map(char::len_utf16)
            .sum();
        Pos::new(p.line, col)
    }

    fn record_lsp(&mut self, a: Pos, b: Pos, text: &str) {
        if self.lsp_full {
            return;
        }
        if self.lsp_changes.len() >= 256 {
            self.lsp_changes.clear();
            self.lsp_full = true;
            return;
        }
        let (ua, ub) = (self.u16pos(a), self.u16pos(b));
        self.lsp_changes.push((ua, ub, text.to_string()));
    }

    /// Byte offset of `p` in the whole text (lines joined with "\n") and its byte column.
    fn byte_offset(&self, p: Pos) -> (usize, usize) {
        let col = byte_idx(&self.lines[p.line], p.col);
        let before: usize = self.lines[..p.line].iter().map(|l| l.len() + 1).sum();
        (before + col, col)
    }

    fn raw_insert(&mut self, pos: Pos, text: &str) -> Pos {
        if self.hl.wants_edits() {
            // Tell the syntax tree exactly what changed (incremental re-parse).
            let (start, col) = self.byte_offset(pos);
            let rows = text.matches('\n').count();
            let end_col = match text.rfind('\n') {
                Some(i) => text.len() - i - 1,
                None => col + text.len(),
            };
            self.hl.edit(
                start,
                start,
                start + text.len(),
                (pos.line, col),
                (pos.line, col),
                (pos.line + rows, end_col),
            );
        } else {
            self.hl.changed_line(pos.line);
        }
        self.record_lsp(pos, pos, text);
        let mut parts = text.split('\n');
        let first = parts.next().unwrap_or("");
        let rest: Vec<String> = parts.map(String::from).collect();
        let line = &mut self.lines[pos.line];
        let b = byte_idx(line, pos.col);
        let tail = line.split_off(b);
        line.push_str(first);
        let n = rest.len();
        if n > 0 {
            let at = pos.line + 1;
            self.lines.splice(at..at, rest);
        }
        self.lines[pos.line + n].push_str(&tail);
        end_of(pos, text)
    }

    fn raw_delete(&mut self, a: Pos, b: Pos) -> String {
        if a >= b {
            return String::new();
        }
        if self.hl.wants_edits() {
            let (start, acol) = self.byte_offset(a);
            let (end, bcol) = self.byte_offset(b);
            self.hl.edit(
                start,
                end,
                start,
                (a.line, acol),
                (b.line, bcol),
                (a.line, acol),
            );
        } else {
            self.hl.changed_line(a.line);
        }
        self.record_lsp(a, b, "");
        if a.line == b.line {
            let line = &mut self.lines[a.line];
            let (s, e) = (byte_idx(line, a.col), byte_idx(line, b.col));
            return line.drain(s..e).collect();
        }
        let removed = self.text_range(a, b);
        let e = byte_idx(&self.lines[b.line], b.col);
        let tail = self.lines[b.line][e..].to_string();
        let s = byte_idx(&self.lines[a.line], a.col);
        self.lines[a.line].truncate(s);
        self.lines[a.line].push_str(&tail);
        self.lines.drain(a.line + 1..=b.line);
        removed
    }

    fn touch(&mut self) {
        self.modified = true;
        self.version += 1;
        self.lsp_dirty = true;
    }

    pub fn insert(&mut self, pos: Pos, text: &str) -> Pos {
        if text.is_empty() {
            return pos;
        }
        let end = self.raw_insert(pos, text);
        self.pending.push(Edit {
            start: pos,
            removed: String::new(),
            inserted: text.to_string(),
        });
        self.touch();
        end
    }

    pub fn delete(&mut self, a: Pos, b: Pos) -> String {
        let r = self.raw_delete(a, b);
        if !r.is_empty() {
            self.pending.push(Edit {
                start: a,
                removed: r.clone(),
                inserted: String::new(),
            });
            self.touch();
        }
        r
    }

    fn begin(&self) -> (Pos, Option<Pos>) {
        (self.cursor, self.anchor)
    }

    fn commit(&mut self, before: (Pos, Option<Pos>), typing: bool) {
        self.want_col = self.cursor.col;
        if self.pending.is_empty() {
            return;
        }
        let edits = std::mem::take(&mut self.pending);
        self.redo.clear();
        let after = (self.cursor, self.anchor);
        if typing
            && !self.seal
            && let Some(g) = self.undo.last_mut()
            && g.typing
            && g.at.elapsed().as_millis() < 1500
        {
            g.edits.extend(edits);
            g.after = after;
            g.at = Instant::now();
            return;
        }
        self.undo.push(Group {
            edits,
            before,
            after,
            typing,
            at: Instant::now(),
        });
        self.seal = false;
    }

    pub fn undo(&mut self) -> bool {
        let Some(g) = self.undo.pop() else {
            return false;
        };
        for e in g.edits.iter().rev() {
            let end = end_of(e.start, &e.inserted);
            self.raw_delete(e.start, end);
            self.raw_insert(e.start, &e.removed);
        }
        self.cursor = self.clamp(g.before.0);
        self.anchor = g.before.1.map(|a| self.clamp(a));
        self.want_col = self.cursor.col;
        self.redo.push(g);
        self.touch();
        self.seal = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(g) = self.redo.pop() else {
            return false;
        };
        for e in g.edits.iter() {
            let end = end_of(e.start, &e.removed);
            self.raw_delete(e.start, end);
            self.raw_insert(e.start, &e.inserted);
        }
        self.cursor = self.clamp(g.after.0);
        self.anchor = g.after.1.map(|a| self.clamp(a));
        self.want_col = self.cursor.col;
        self.undo.push(g);
        self.touch();
        self.seal = true;
        true
    }

    // ---------- high level editing ----------

    pub fn delete_selection(&mut self) -> bool {
        if let Some((a, b)) = self.selection() {
            self.delete(a, b);
            self.cursor = a;
            self.anchor = None;
            true
        } else {
            self.anchor = None;
            false
        }
    }

    pub fn insert_char(&mut self, c: char) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        let pair_close = match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            '`' => Some('`'),
            _ => None,
        };
        // Wrap selection with brackets/quotes.
        if let (Some(cl), Some((a, b))) = (pair_close, self.selection()) {
            self.insert(b, &cl.to_string());
            self.insert(a, &c.to_string());
            self.anchor = Some(Pos::new(a.line, a.col + 1));
            self.cursor = if a.line == b.line {
                Pos::new(b.line, b.col + 1)
            } else {
                b
            };
            self.commit(before, false);
            return;
        }
        self.delete_selection();
        let next = self.char_at(self.cursor);
        let prev = self.char_before(self.cursor);
        // Type over an auto-inserted closing char.
        if matches!(c, ')' | ']' | '}' | '"' | '\'' | '`') && next == Some(c) {
            self.cursor.col += 1;
            self.seal = true;
            self.commit(before, true);
            return;
        }
        let next_ok = next.is_none_or(|n| n.is_whitespace() || ")]};,".contains(n));
        let is_quote = matches!(c, '"' | '\'' | '`');
        let quote_ok = !is_quote || prev.is_none_or(|p| !is_word(p) && p != c);
        match pair_close {
            Some(cl) if next_ok && quote_ok => {
                self.insert(self.cursor, &format!("{c}{cl}"));
            }
            _ => {
                self.insert(self.cursor, &c.to_string());
            }
        }
        self.cursor.col += 1;
        self.commit(before, true);
    }

    pub fn insert_text(&mut self, text: &str) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        self.delete_selection();
        let t = text.replace("\r\n", "\n");
        self.cursor = self.insert(self.cursor, &t);
        self.commit(before, false);
    }

    pub fn insert_indent(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        self.delete_selection();
        let unit = if self.indent_unit == "\t" {
            "\t".to_string()
        } else {
            let w = self.indent_unit.len();
            " ".repeat(w - self.cursor.col % w)
        };
        self.cursor = self.insert(self.cursor, &unit);
        self.commit(before, true);
    }

    /// Enter: a new line indented like this one (one level deeper after an opener). With
    /// `trim_ws`, a line holding only indentation is emptied first, so blank lines don't
    /// keep trailing spaces (Sublime's `trim_automatic_white_space`).
    pub fn newline(&mut self, trim_ws: bool) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        self.delete_selection();
        let l = self.cursor.line;
        let blank = self.lines[l].chars().all(|c| c == ' ' || c == '\t');
        if trim_ws && blank && !self.lines[l].is_empty() && self.cursor.col == self.line_len(l) {
            // Its indentation still carries to the new line below.
            let indent = self.lines[l].clone();
            self.delete(Pos::new(l, 0), self.cursor);
            self.cursor = self.insert(Pos::new(l, 0), &format!("\n{indent}"));
            self.commit(before, false);
            return;
        }
        let line = &self.lines[self.cursor.line];
        let indent: String = line
            .chars()
            .take(self.cursor.col)
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let left: String = line.chars().take(self.cursor.col).collect();
        let prev = left.trim_end().chars().last();
        let next = self.char_at(self.cursor);
        let opener = matches!(prev, Some('{' | '(' | '['))
            || (prev == Some(':') && self.lang.as_deref() == Some("python"));
        let unit = self.indent_unit.clone();
        let c = self.cursor;
        if opener && matches!(next, Some('}' | ')' | ']')) {
            self.insert(c, &format!("\n{indent}{unit}\n{indent}"));
            self.cursor = Pos::new(c.line + 1, char_len(&indent) + char_len(&unit));
        } else if opener {
            self.cursor = self.insert(c, &format!("\n{indent}{unit}"));
        } else {
            self.cursor = self.insert(c, &format!("\n{indent}"));
        }
        self.commit(before, false);
    }

    pub fn insert_line_after(&mut self, above: bool) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        self.anchor = None;
        let l = self.cursor.line;
        let indent: String = self.lines[l]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if above {
            self.insert(Pos::new(l, 0), &format!("{indent}\n"));
            self.cursor = Pos::new(l, char_len(&indent));
        } else {
            let end = Pos::new(l, self.line_len(l));
            self.cursor = self.insert(end, &format!("\n{indent}"));
        }
        self.commit(before, false);
    }

    pub fn backspace(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        if self.delete_selection() {
            self.commit(before, false);
            return;
        }
        let c = self.cursor;
        if c.col == 0 {
            if c.line == 0 {
                return;
            }
            let pl = self.line_len(c.line - 1);
            self.delete(Pos::new(c.line - 1, pl), c);
            self.cursor = Pos::new(c.line - 1, pl);
        } else {
            let prev = self.char_before(c);
            let next = self.char_at(c);
            let prefix: String = self.lines[c.line].chars().take(c.col).collect();
            let pairs = [
                ('(', ')'),
                ('[', ']'),
                ('{', '}'),
                ('"', '"'),
                ('\'', '\''),
                ('`', '`'),
            ];
            let n = if pairs
                .iter()
                .any(|&(o, cl)| prev == Some(o) && next == Some(cl))
            {
                self.delete(Pos::new(c.line, c.col - 1), Pos::new(c.line, c.col + 1));
                1
            } else if self.indent_unit != "\t" && prefix.chars().all(|ch| ch == ' ') {
                let w = self.indent_unit.len();
                let n = (c.col - 1) % w + 1;
                self.delete(Pos::new(c.line, c.col - n), c);
                n
            } else {
                self.delete(Pos::new(c.line, c.col - 1), c);
                1
            };
            self.cursor.col -= n;
        }
        self.commit(before, true);
    }

    pub fn delete_forward(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        if self.delete_selection() {
            self.commit(before, false);
            return;
        }
        let c = self.cursor;
        if c.col >= self.line_len(c.line) {
            if c.line + 1 < self.lines.len() {
                self.delete(c, Pos::new(c.line + 1, 0));
            }
        } else {
            self.delete(c, Pos::new(c.line, c.col + 1));
        }
        self.commit(before, true);
    }

    pub fn delete_word(&mut self, forward: bool) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        if !self.delete_selection() {
            let c = self.cursor;
            if forward {
                let e = self.word_right_pos(c);
                self.delete(c, e);
            } else {
                let s = self.word_left_pos(c);
                self.delete(s, c);
                self.cursor = s;
            }
        }
        self.commit(before, false);
    }

    pub fn delete_to_line_start(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        if !self.delete_selection() {
            let c = self.cursor;
            self.delete(Pos::new(c.line, 0), c);
            self.cursor.col = 0;
        }
        self.commit(before, false);
    }

    // ---------- movement ----------

    pub fn set_cursor(&mut self, p: Pos, select: bool) {
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = self.clamp(p);
        self.seal = true;
    }

    pub fn goto(&mut self, p: Pos) {
        self.set_cursor(p, false);
        self.want_col = self.cursor.col;
    }

    pub fn move_left(&mut self, select: bool, word: bool) {
        if !select && let Some((a, _)) = self.selection() {
            self.goto(a);
            return;
        }
        let c = self.cursor;
        let p = if word {
            self.word_left_pos(c)
        } else if c.col > 0 {
            Pos::new(c.line, c.col - 1)
        } else if c.line > 0 {
            Pos::new(c.line - 1, self.line_len(c.line - 1))
        } else {
            c
        };
        self.set_cursor(p, select);
        self.want_col = self.cursor.col;
    }

    pub fn move_right(&mut self, select: bool, word: bool) {
        if !select && let Some((_, b)) = self.selection() {
            self.goto(b);
            return;
        }
        let c = self.cursor;
        let p = if word {
            self.word_right_pos(c)
        } else if c.col < self.line_len(c.line) {
            Pos::new(c.line, c.col + 1)
        } else if c.line + 1 < self.lines.len() {
            Pos::new(c.line + 1, 0)
        } else {
            c
        };
        self.set_cursor(p, select);
        self.want_col = self.cursor.col;
    }

    pub fn move_vert(&mut self, delta: isize, select: bool) {
        let c = self.cursor;
        let target = c.line as isize + delta;
        let p = if target < 0 {
            Pos::new(0, 0)
        } else if target as usize >= self.lines.len() {
            self.last_pos()
        } else {
            Pos::new(target as usize, self.want_col)
        };
        let want = self.want_col;
        self.set_cursor(p, select);
        if target >= 0 && (target as usize) < self.lines.len() {
            self.want_col = want;
        } else {
            self.want_col = self.cursor.col;
        }
    }

    /// Apply a cursor-movement / selection / deletion command (the editing keys from
    /// keymap.toml). `page` is the page height in rows, `wrap` the wrap width if wrapping.
    /// Returns false for commands that aren't motions.
    pub fn apply_motion(&mut self, cmd: Command, page: usize, wrap: Option<usize>) -> bool {
        use Command::*;
        let page = page.max(1);
        let vert = |b: &mut Buffer, d: isize, sel: bool| match wrap {
            Some(w) => b.move_visual(d, sel, w),
            None => b.move_vert(d, sel),
        };
        match cmd {
            MoveLeft | SelectLeft => self.move_left(cmd == SelectLeft, false),
            MoveRight | SelectRight => self.move_right(cmd == SelectRight, false),
            MoveWordLeft | SelectWordLeft => self.move_left(cmd == SelectWordLeft, true),
            MoveWordRight | SelectWordRight => self.move_right(cmd == SelectWordRight, true),
            MoveLineStart | SelectToLineStart => self.move_home(cmd == SelectToLineStart),
            MoveLineEnd | SelectToLineEnd => self.move_end(cmd == SelectToLineEnd),
            MoveDocStart | SelectToDocStart => self.move_doc(false, cmd == SelectToDocStart),
            MoveDocEnd | SelectToDocEnd => self.move_doc(true, cmd == SelectToDocEnd),
            MoveUp | SelectUp => vert(self, -1, cmd == SelectUp),
            MoveDown | SelectDown => vert(self, 1, cmd == SelectDown),
            MovePageUp | SelectPageUp => {
                self.move_vert(-(page as isize), cmd == SelectPageUp);
                self.scroll_y = self.scroll_y.saturating_sub(page);
            }
            MovePageDown | SelectPageDown => {
                self.move_vert(page as isize, cmd == SelectPageDown);
                self.scroll_y = (self.scroll_y + page).min(self.lines.len().saturating_sub(1));
            }
            DeleteLeft => self.backspace(),
            DeleteRight => self.delete_forward(),
            DeleteWordLeft => self.delete_word(false),
            DeleteWordRight => self.delete_word(true),
            DeleteToLineStart => self.delete_to_line_start(),
            ScrollLineUp => self.scroll_y = self.scroll_y.saturating_sub(1),
            ScrollLineDown => {
                self.scroll_y = (self.scroll_y + 1).min(self.lines.len().saturating_sub(1))
            }
            _ => return false,
        }
        true
    }

    /// Up/Down by visual rows when lines wrap at `width` columns (like Sublime).
    pub fn move_visual(&mut self, delta: isize, select: bool, width: usize) {
        use crate::wrap::{col_in_seg, dcol, seg_of, segments};
        let tab = self.tab_size;
        let c = self.cursor;
        let segs = segments(&self.lines[c.line], tab, width);
        let d = dcol(&self.lines[c.line], c.col, tab);
        let i = seg_of(&segs, d);
        // Keep the visual column across consecutive moves.
        let vx = match self.want_vx {
            Some((p, vx)) if p == c => vx,
            _ => d - segs[i].start + segs[i].indent,
        };
        let (mut line, mut seg) = (c.line, i);
        let mut at_edge = None;
        for _ in 0..delta.unsigned_abs() {
            if delta > 0 {
                let n = segments(&self.lines[line], tab, width).len();
                if seg + 1 < n {
                    seg += 1;
                } else if line + 1 < self.lines.len() {
                    line += 1;
                    seg = 0;
                } else {
                    at_edge = Some(self.last_pos());
                    break;
                }
            } else if seg > 0 {
                seg -= 1;
            } else if line > 0 {
                line -= 1;
                seg = segments(&self.lines[line], tab, width).len() - 1;
            } else {
                at_edge = Some(Pos::new(0, 0));
                break;
            }
        }
        let p = at_edge.unwrap_or_else(|| {
            let segs = segments(&self.lines[line], tab, width);
            let g = segs[seg];
            let target = g.start as f32 + vx.saturating_sub(g.indent) as f32;
            Pos::new(
                line,
                col_in_seg(&self.lines[line], tab, &g, target, seg + 1 == segs.len()),
            )
        });
        self.set_cursor(p, select);
        self.want_col = self.cursor.col;
        self.want_vx = at_edge.is_none().then_some((self.cursor, vx));
    }

    pub fn move_home(&mut self, select: bool) {
        let l = self.cursor.line;
        let first = self.lines[l]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        let col = if self.cursor.col == first { 0 } else { first };
        self.set_cursor(Pos::new(l, col), select);
        self.want_col = col;
    }

    pub fn move_end(&mut self, select: bool) {
        let l = self.cursor.line;
        self.set_cursor(Pos::new(l, self.line_len(l)), select);
        self.want_col = self.cursor.col;
    }

    pub fn move_doc(&mut self, end: bool, select: bool) {
        let p = if end { self.last_pos() } else { Pos::new(0, 0) };
        self.set_cursor(p, select);
        self.want_col = self.cursor.col;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(Pos::new(0, 0));
        self.cursor = self.last_pos();
        self.seal = true;
    }

    pub fn select_line(&mut self) {
        let (s, e) = match self.selection() {
            Some((a, b)) => (a.line, b.line),
            None => (self.cursor.line, self.cursor.line),
        };
        let e = if self.selection().is_some() && self.cursor.col == 0 && e > s {
            e
        } else {
            e + 1
        };
        self.anchor = Some(Pos::new(s, 0));
        self.cursor = if e < self.lines.len() {
            Pos::new(e, 0)
        } else {
            self.last_pos()
        };
        self.seal = true;
    }

    /// Ctrl+D: select word under cursor, or jump to the next occurrence of the selection.
    pub fn select_word_or_next(&mut self) -> bool {
        if let Some((a, b)) = self.selection() {
            let needle = self.text_range(a, b);
            if let Some((s, e)) = self.find(&needle, b, true, false) {
                self.anchor = Some(s);
                self.cursor = e;
                return true;
            }
            false
        } else if let Some((s, e)) = self.word_bounds(self.cursor) {
            self.anchor = Some(s);
            self.cursor = e;
            true
        } else {
            false
        }
    }

    // ---------- line operations ----------

    fn sel_lines(&self) -> (usize, usize) {
        if let Some((a, b)) = self.selection() {
            let end = if b.col == 0 && b.line > a.line {
                b.line - 1
            } else {
                b.line
            };
            (a.line, end)
        } else {
            (self.cursor.line, self.cursor.line)
        }
    }

    pub fn delete_line(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        let (s, e) = self.sel_lines();
        if e + 1 < self.lines.len() {
            self.delete(Pos::new(s, 0), Pos::new(e + 1, 0));
        } else if s > 0 {
            self.delete(
                Pos::new(s - 1, self.line_len(s - 1)),
                Pos::new(e, self.line_len(e)),
            );
        } else {
            self.delete(Pos::new(0, 0), Pos::new(e, self.line_len(e)));
        }
        self.anchor = None;
        let want = self.want_col;
        self.cursor = self.clamp(Pos::new(s, want));
        self.commit(before, false);
        self.want_col = want;
    }

    pub fn duplicate(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        if let Some((a, b)) = self.selection() {
            let t = self.text_range(a, b);
            self.insert(b, &t);
            self.anchor = Some(b);
            self.cursor = end_of(b, &t);
        } else {
            let l = self.cursor.line;
            let t = format!("{}\n", self.lines[l]);
            self.insert(Pos::new(l, 0), &t);
            self.cursor.line += 1;
        }
        self.commit(before, false);
    }

    pub fn move_lines(&mut self, up: bool) {
        if self.read_only {
            return;
        }
        let (s, e) = self.sel_lines();
        if (up && s == 0) || (!up && e + 1 >= self.lines.len()) {
            return;
        }
        let before = self.begin();
        if up {
            let t = self.lines[s - 1].clone();
            self.delete(Pos::new(s - 1, 0), Pos::new(s, 0));
            let le = self.line_len(e - 1);
            self.insert(Pos::new(e - 1, le), &format!("\n{t}"));
            self.cursor.line -= 1;
            if let Some(a) = &mut self.anchor {
                a.line -= 1;
            }
        } else {
            let t = self.lines[e + 1].clone();
            self.delete(
                Pos::new(e, self.line_len(e)),
                Pos::new(e + 1, self.line_len(e + 1)),
            );
            self.insert(Pos::new(s, 0), &format!("{t}\n"));
            self.cursor.line += 1;
            if let Some(a) = &mut self.anchor {
                a.line += 1;
            }
        }
        self.commit(before, false);
    }

    pub fn indent(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        let (s, e) = self.sel_lines();
        let unit = self.indent_unit.clone();
        let ul = char_len(&unit);
        for l in s..=e {
            if !self.lines[l].is_empty() || s == e {
                self.insert(Pos::new(l, 0), &unit);
            }
        }
        self.cursor.col += ul;
        if let Some(a) = &mut self.anchor
            && (a.col > 0 || a.line == self.cursor.line)
        {
            a.col += ul;
        }
        self.commit(before, false);
    }

    pub fn outdent(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        let (s, e) = self.sel_lines();
        let w = if self.indent_unit == "\t" {
            4
        } else {
            self.indent_unit.len()
        };
        for l in s..=e {
            let line = &self.lines[l];
            let n = if line.starts_with('\t') {
                1
            } else {
                line.chars().take(w).take_while(|c| *c == ' ').count()
            };
            if n > 0 {
                self.delete(Pos::new(l, 0), Pos::new(l, n));
                if self.cursor.line == l {
                    self.cursor.col = self.cursor.col.saturating_sub(n);
                }
                if let Some(a) = &mut self.anchor
                    && a.line == l
                {
                    a.col = a.col.saturating_sub(n);
                }
            }
        }
        self.commit(before, false);
    }

    pub fn toggle_comment(&mut self) -> bool {
        if self.read_only {
            return false;
        }
        let Some(tok) = self.comment.clone() else {
            return false;
        };
        let before = self.begin();
        let (s, e) = self.sel_lines();
        let nonempty: Vec<usize> = (s..=e)
            .filter(|&l| !self.lines[l].trim().is_empty())
            .collect();
        if nonempty.is_empty() {
            return true;
        }
        let all = nonempty
            .iter()
            .all(|&l| self.lines[l].trim_start().starts_with(tok.as_str()));
        if all {
            for &l in &nonempty {
                let line = &self.lines[l];
                let ws = line.chars().take_while(|c| c.is_whitespace()).count();
                let mut n = char_len(&tok);
                if line.chars().nth(ws + n) == Some(' ') {
                    n += 1;
                }
                self.delete(Pos::new(l, ws), Pos::new(l, ws + n));
                if self.cursor.line == l {
                    self.cursor.col = self.cursor.col.saturating_sub(n);
                }
            }
        } else {
            let min_ws = nonempty
                .iter()
                .map(|&l| {
                    self.lines[l]
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .count()
                })
                .min()
                .unwrap_or(0);
            let ins = format!("{tok} ");
            for &l in &nonempty {
                self.insert(Pos::new(l, min_ws), &ins);
                if self.cursor.line == l && self.cursor.col >= min_ws {
                    self.cursor.col += char_len(&ins);
                }
            }
        }
        if let Some(a) = self.anchor {
            self.anchor = Some(self.clamp(a));
        }
        self.cursor = self.clamp(self.cursor);
        self.commit(before, false);
        true
    }

    pub fn join_lines(&mut self) {
        if self.read_only {
            return;
        }
        let before = self.begin();
        let (s, e) = self.sel_lines();
        let e = if e == s { s + 1 } else { e };
        for _ in s..e {
            if s + 1 >= self.lines.len() {
                break;
            }
            let ll = self.line_len(s);
            let ws = self.lines[s + 1]
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            self.delete(Pos::new(s, ll), Pos::new(s + 1, ws));
            let need_space = self.lines[s]
                .chars()
                .last()
                .is_some_and(|c| !c.is_whitespace())
                && self.char_at(Pos::new(s, ll)).is_some();
            if need_space {
                self.insert(Pos::new(s, ll), " ");
            }
            self.cursor = Pos::new(s, ll);
        }
        self.anchor = None;
        self.commit(before, false);
    }

    pub fn transform_selection(&mut self, f: impl Fn(&str) -> String) {
        if self.read_only {
            return;
        }
        let Some((a, b)) = self.selection() else {
            return;
        };
        let before = self.begin();
        let t = self.text_range(a, b);
        let n = f(&t);
        self.delete(a, b);
        let end = self.insert(a, &n);
        self.anchor = Some(a);
        self.cursor = end;
        self.commit(before, false);
    }

    pub fn sort_lines(&mut self) {
        if self.read_only {
            return;
        }
        let (s, e) = self.sel_lines();
        if s == e {
            return;
        }
        let before = self.begin();
        let mut ls: Vec<String> = self.lines[s..=e].to_vec();
        ls.sort();
        let end = Pos::new(e, self.line_len(e));
        self.delete(Pos::new(s, 0), end);
        let end = self.insert(Pos::new(s, 0), &ls.join("\n"));
        self.anchor = Some(Pos::new(s, 0));
        self.cursor = end;
        self.commit(before, false);
    }

    // ---------- search ----------

    fn smart_lower(needle: &str) -> bool {
        !needle.chars().any(|c| c.is_uppercase())
    }

    /// Byte ranges of `needle` in a line (smart-case, ascii insensitive).
    pub fn matches_in_line(line: &str, needle: &str) -> Vec<(usize, usize)> {
        if needle.is_empty() || needle.contains('\n') {
            return vec![];
        }
        let ci = Self::smart_lower(needle);
        let (hay, nd) = if ci {
            (line.to_ascii_lowercase(), needle.to_ascii_lowercase())
        } else {
            (line.to_string(), needle.to_string())
        };
        hay.match_indices(&nd)
            .map(|(i, m)| (i, i + m.len()))
            .collect()
    }

    pub fn find(
        &self,
        needle: &str,
        from: Pos,
        forward: bool,
        _wrap_only: bool,
    ) -> Option<(Pos, Pos)> {
        let first = needle.split('\n').next().unwrap_or("");
        if first.is_empty() {
            return None;
        }
        let n = self.lines.len();
        let to_pos = |l: usize, b: usize, s: &str| -> (Pos, Pos) {
            let c = char_len(&s[..b]);
            (Pos::new(l, c), Pos::new(l, c + char_len(first)))
        };
        if forward {
            for i in 0..=n {
                let l = (from.line + i) % n;
                let line = &self.lines[l];
                let start = if i == 0 { byte_idx(line, from.col) } else { 0 };
                let ms = Self::matches_in_line(line, first);
                let hit = if i == n {
                    ms.into_iter().find(|m| m.0 < byte_idx(line, from.col))
                } else {
                    ms.into_iter().find(|m| m.0 >= start)
                };
                if let Some((b, _)) = hit {
                    return Some(to_pos(l, b, line));
                }
            }
        } else {
            for i in 0..=n {
                let l = (from.line + n * 2 - i) % n;
                let line = &self.lines[l];
                let limit = if i == 0 {
                    byte_idx(line, from.col)
                } else {
                    line.len() + 1
                };
                let ms = Self::matches_in_line(line, first);
                let hit = if i == n {
                    ms.into_iter()
                        .rev()
                        .find(|m| m.0 >= byte_idx(line, from.col))
                } else {
                    ms.into_iter().rev().find(|m| m.0 < limit)
                };
                if let Some((b, _)) = hit {
                    return Some(to_pos(l, b, line));
                }
            }
        }
        None
    }

    // ---------- find panel (search.rs matcher) ----------

    /// Matches in line `l` as (start, end) positions.
    pub fn line_matches(&self, m: &crate::search::Matcher, l: usize) -> Vec<(Pos, Pos)> {
        let line = &self.lines[l];
        m.find_in(line)
            .into_iter()
            .map(|(s, e)| {
                (
                    Pos::new(l, char_len(&line[..s])),
                    Pos::new(l, char_len(&line[..e])),
                )
            })
            .collect()
    }

    /// Every match (inside `scope` if given), up to `limit`.
    pub fn matches_with(
        &self,
        m: &crate::search::Matcher,
        scope: Option<(Pos, Pos)>,
        limit: usize,
    ) -> Vec<(Pos, Pos)> {
        let (first, last) = match scope {
            Some((a, b)) => (a.line, b.line.min(self.lines.len() - 1)),
            None => (0, self.lines.len() - 1),
        };
        let mut out = vec![];
        for l in first..=last {
            for r in self.line_matches(m, l) {
                if scope.is_none_or(|(a, b)| r.0 >= a && r.1 <= b) {
                    out.push(r);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// The next (or previous) match from `from`, wrapping around the document if `wrap`.
    pub fn find_with(
        &self,
        m: &crate::search::Matcher,
        from: Pos,
        forward: bool,
        wrap: bool,
        scope: Option<(Pos, Pos)>,
    ) -> Option<(Pos, Pos)> {
        let n = self.lines.len();
        let ok = |r: &(Pos, Pos)| scope.is_none_or(|(a, b)| r.0 >= a && r.1 <= b);
        let lines: Vec<usize> = if forward {
            let mut v: Vec<usize> = (from.line..n).collect();
            if wrap {
                v.extend(0..=from.line);
            }
            v
        } else {
            let mut v: Vec<usize> = (0..=from.line).rev().collect();
            if wrap {
                v.extend((from.line..n).rev());
            }
            v
        };
        for (i, &l) in lines.iter().enumerate() {
            let ms = self.line_matches(m, l);
            let wrapped = i > 0 && l == from.line;
            let hit = if forward {
                ms.into_iter()
                    .filter(ok)
                    .find(|r| wrapped || l != from.line || r.0 >= from)
            } else {
                ms.into_iter()
                    .rev()
                    .filter(ok)
                    .find(|r| wrapped || l != from.line || r.1 <= from)
            };
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    /// Replace one match (a range on one line) with the expanded template, as one undo
    /// step. Returns where the replacement ends.
    pub fn replace_match(
        &mut self,
        m: &crate::search::Matcher,
        r: (Pos, Pos),
        template: &str,
    ) -> Option<Pos> {
        if self.read_only || r.0.line != r.1.line || r.0.line >= self.lines.len() {
            return None;
        }
        let line = &self.lines[r.0.line];
        let range = (byte_idx(line, r.0.col), byte_idx(line, r.1.col));
        let text = m.replacement(line, range, template);
        let before = self.begin();
        self.delete(r.0, r.1);
        let end = self.insert(r.0, &text);
        self.anchor = None;
        self.cursor = end;
        self.commit(before, false);
        Some(end)
    }

    /// Replace every match (inside `scope` if given) as one undo step; the count.
    pub fn replace_with(
        &mut self,
        m: &crate::search::Matcher,
        template: &str,
        scope: Option<(Pos, Pos)>,
    ) -> usize {
        if self.read_only {
            return 0;
        }
        let all = self.matches_with(m, scope, usize::MAX);
        if all.is_empty() {
            return 0;
        }
        // Expanded against the original text, then applied from the end backwards.
        let reps: Vec<((Pos, Pos), String)> = all
            .into_iter()
            .map(|r| {
                let line = &self.lines[r.0.line];
                let range = (byte_idx(line, r.0.col), byte_idx(line, r.1.col));
                (r, m.replacement(line, range, template))
            })
            .collect();
        let before = self.begin();
        for ((a, b), t) in reps.iter().rev() {
            self.delete(*a, *b);
            self.insert(*a, t);
        }
        self.anchor = None;
        self.cursor = self.clamp(self.cursor);
        self.commit(before, false);
        reps.len()
    }

    /// Apply a set of non-overlapping edits (e.g. from an LSP) as one undo step.
    pub fn apply_edits(&mut self, mut edits: Vec<(Pos, Pos, String)>) {
        if edits.is_empty() {
            return;
        }
        let before = self.begin();
        edits.sort_by(|a, b| b.0.cmp(&a.0));
        for (a, b, t) in edits {
            let a = self.clamp(a);
            let b = self.clamp(b);
            // Keep the cursor roughly anchored when edits happen before it.
            let c = self.cursor;
            self.delete(a, b);
            let end = self.insert(a, &t);
            if c.line > b.line {
                let delta = end.line as isize - b.line as isize;
                self.cursor.line = (c.line as isize + delta).max(0) as usize;
            }
        }
        self.anchor = None;
        self.cursor = self.clamp(self.cursor);
        self.commit(before, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::from_text(
            1,
            Some(PathBuf::from("t.rs")),
            text,
            HlCache::new("Plain Text".into()),
        )
    }

    #[test]
    fn insert_delete_undo_redo() {
        let mut b = buf("hello\nworld");
        b.goto(Pos::new(0, 5));
        b.insert_text(" there\nnew");
        assert_eq!(b.text(), "hello there\nnew\nworld");
        assert_eq!(b.cursor, Pos::new(1, 3));
        b.undo();
        assert_eq!(b.text(), "hello\nworld");
        b.redo();
        assert_eq!(b.text(), "hello there\nnew\nworld");
    }

    #[test]
    fn typing_groups_into_one_undo() {
        let mut b = buf("");
        for c in "abc".chars() {
            b.insert_char(c);
        }
        assert_eq!(b.text(), "abc");
        b.undo();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn autopair_and_skip_over() {
        let mut b = buf("");
        b.insert_char('(');
        assert_eq!(b.text(), "()");
        b.insert_char('x');
        b.insert_char(')');
        assert_eq!(b.text(), "(x)");
        assert_eq!(b.cursor.col, 3);
        b.goto(Pos::new(0, 1));
        b.backspace();
        assert_eq!(b.text(), "x)");
    }

    #[test]
    fn newline_indents_between_braces() {
        let mut b = buf("fn a() {}");
        b.goto(Pos::new(0, 8));
        b.newline(true);
        assert_eq!(b.text(), "fn a() {\n    \n}");
        assert_eq!(b.cursor, Pos::new(1, 4));
    }

    #[test]
    fn enter_on_an_indentation_only_line_leaves_it_empty() {
        let mut b = buf("def main():");
        b.lang = Some("python".into());
        b.goto(Pos::new(0, 11));
        b.newline(true);
        b.insert_text("pass");
        b.newline(true);
        b.newline(true);
        // The blank line between the body and what follows has no trailing spaces, and the
        // new line is still indented.
        assert_eq!(b.text(), "def main():\n    pass\n\n    ");
        assert_eq!(b.cursor, Pos::new(3, 4));
        b.undo();
        assert_eq!(b.text(), "def main():\n    pass\n    ", "one undo step");
        // Off: the indentation stays.
        let mut b = buf("    ");
        b.goto(Pos::new(0, 4));
        b.newline(false);
        assert_eq!(b.text(), "    \n    ");
    }

    #[test]
    fn line_ops() {
        let mut b = buf("a\nb\nc");
        b.goto(Pos::new(0, 0));
        b.move_lines(false);
        assert_eq!(b.text(), "b\na\nc");
        assert_eq!(b.cursor.line, 1);
        b.move_lines(true);
        assert_eq!(b.text(), "a\nb\nc");
        b.duplicate();
        assert_eq!(b.text(), "a\na\nb\nc");
        b.delete_line();
        assert_eq!(b.text(), "a\nb\nc");
        b.goto(Pos::new(2, 0));
        b.delete_line();
        assert_eq!(b.text(), "a\nb");
    }

    #[test]
    fn comment_toggle() {
        let mut b = buf("  x\n  y");
        b.select_all();
        b.toggle_comment();
        assert_eq!(b.text(), "  // x\n  // y");
        b.select_all();
        b.toggle_comment();
        assert_eq!(b.text(), "  x\n  y");
    }

    #[test]
    fn find_and_next_occurrence() {
        let mut b = buf("foo bar foo\nfoo");
        b.goto(Pos::new(0, 1));
        b.select_word_or_next();
        assert_eq!(b.selected_text().as_deref(), Some("foo"));
        b.select_word_or_next();
        assert_eq!(b.selection(), Some((Pos::new(0, 8), Pos::new(0, 11))));
        b.select_word_or_next();
        assert_eq!(b.selection(), Some((Pos::new(1, 0), Pos::new(1, 3))));
        b.select_word_or_next();
        assert_eq!(b.selection(), Some((Pos::new(0, 0), Pos::new(0, 3))));
        assert_eq!(
            b.find("foo", Pos::new(0, 0), false, false),
            Some((Pos::new(1, 0), Pos::new(1, 3)))
        );
    }

    #[test]
    fn find_panel_search_and_replace_in_a_buffer() {
        use crate::search::{FindOptions, Matcher};
        let m = |p: &str, regex: bool| {
            Matcher::new(
                p,
                &FindOptions {
                    regex,
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let mut b = buf("foo bar foo\nfoo é foo");
        let f = m("foo", false);
        // Forward from inside the first match, wrapping, and backward.
        assert_eq!(
            b.find_with(&f, Pos::new(0, 1), true, true, None),
            Some((Pos::new(0, 8), Pos::new(0, 11)))
        );
        assert_eq!(
            b.find_with(&f, Pos::new(1, 7), true, true, None),
            Some((Pos::new(0, 0), Pos::new(0, 3))),
            "wraps to the top"
        );
        assert_eq!(b.find_with(&f, Pos::new(1, 7), true, false, None), None);
        assert_eq!(
            b.find_with(&f, Pos::new(0, 0), false, true, None),
            Some((Pos::new(1, 6), Pos::new(1, 9))),
            "backward wraps to the bottom"
        );
        // Character columns, not bytes, after a multi-byte character.
        assert_eq!(b.line_matches(&f, 1)[1], (Pos::new(1, 6), Pos::new(1, 9)));
        // In a selection only.
        let scope = Some((Pos::new(0, 4), Pos::new(1, 3)));
        assert_eq!(b.matches_with(&f, scope, usize::MAX).len(), 2);
        assert_eq!(b.replace_with(&f, "X", scope), 2);
        assert_eq!(b.text(), "foo bar X\nX é foo");
        b.undo();
        assert_eq!(b.text(), "foo bar foo\nfoo é foo", "one undo step");
        // Regex groups, one match at a time.
        let r = m(r"(\w+) (\w+)", true);
        let first = b.find_with(&r, Pos::new(0, 0), true, true, None).unwrap();
        b.replace_match(&r, first, "$2 $1");
        assert_eq!(b.lines[0], "bar foo foo");
        assert_eq!(b.replace_with(&m("o", false), "0", None), 8);
    }

    #[test]
    fn reload_replaces_only_what_changed_and_is_undoable() {
        for (old, new) in [
            ("a\nb\nc", "a\nB\nc"),
            ("é", "ĩ"),         // same last byte, different characters
            ("aé\nx", "aè\nx"), // multi-byte change mid-line
            ("abc", "abcabc"),  // grown at the end
            ("abcabc", "abc"),  // shrunk
            ("", "new\ntext\n"),
            ("x\r\ny", "x\r\ny\r\nz"),
        ] {
            let mut b = buf(old);
            b.reload_from(new);
            assert_eq!(b.text(), new, "{old:?} → {new:?}");
            assert!(!b.modified);
            b.undo();
            assert_eq!(
                b.lines.join("\n"),
                old.replace("\r\n", "\n"),
                "undo {old:?}"
            );
        }
        // The cursor stays where the text didn't change.
        let mut b = buf("one\ntwo\nthree");
        b.goto(Pos::new(2, 4));
        b.reload_from("ONE\ntwo\nthree");
        assert_eq!(b.cursor, Pos::new(2, 4));
    }

    #[test]
    fn crlf_roundtrip_and_unicode() {
        let mut b = buf("é€\r\nb");
        assert!(b.crlf);
        b.goto(Pos::new(0, 2));
        b.insert_char('x');
        assert_eq!(b.text(), "é€x\r\nb");
        b.move_left(false, false);
        b.delete_forward();
        assert_eq!(b.text(), "é€\r\nb");
    }

    #[test]
    fn apply_edits_reverse_order() {
        let mut b = buf("let a = 1;\nlet b = 2;");
        b.apply_edits(vec![
            (Pos::new(0, 4), Pos::new(0, 5), "x".into()),
            (Pos::new(1, 4), Pos::new(1, 5), "y".into()),
        ]);
        assert_eq!(b.text(), "let x = 1;\nlet y = 2;");
        b.undo();
        assert_eq!(b.text(), "let a = 1;\nlet b = 2;");
    }

    #[test]
    fn indentation_guess_and_convert() {
        let two: Vec<String> = "a\n  b\n    c\n  d\n".lines().map(String::from).collect();
        assert_eq!(guess_indent(&two), Some(Indent::Spaces(2)));
        let four: Vec<String> = "a\n    b\n        c\n    /*\n     * x\n     */\n"
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(guess_indent(&four), Some(Indent::Spaces(4)));
        let tabs: Vec<String> = "a\n\tb\n\t\tc\n".lines().map(String::from).collect();
        assert_eq!(guess_indent(&tabs), Some(Indent::Tabs));
        assert_eq!(guess_indent(&["x".to_string()]), None);

        let mut b = buf("fn a() {\n    x;\n        y;\n}");
        assert_eq!(b.indent_kind(), Indent::Spaces(4));
        assert_eq!(b.convert_indentation(true), 2);
        assert_eq!(b.text(), "fn a() {\n\tx;\n\t\ty;\n}");
        assert_eq!(b.indent_kind(), Indent::Tabs);
        b.undo();
        assert_eq!(b.text(), "fn a() {\n    x;\n        y;\n}");
        // Undo restores the text; the setting stays (as in Sublime). Switch back to spaces.
        b.set_indent(Indent::Spaces(4));
        b.set_tab_width(2);
        assert_eq!(b.indent_unit, "  ");
        b.convert_indentation(true);
        assert_eq!(b.lines[2], "\t\t\t\ty;");
        // Blank lines take the surrounding indentation (for guides).
        let b = buf("a\n    b\n\n    c\n");
        assert_eq!(b.indent_width(2), 4);
    }
}
