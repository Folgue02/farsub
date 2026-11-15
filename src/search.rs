//! Text search: the matcher behind the find panel (literal or regex, case, whole word),
//! replacement templates, and Find in Files (a background walk over folders that streams
//! per-file results).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::project::Folder;

/// The find panel's toggles (kept in the workspace, like Sublime).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct FindOptions {
    pub regex: bool,
    pub case: bool,
    pub word: bool,
    /// Find Next past the last match starts over at the top.
    pub wrap: bool,
    /// Find / Replace All only inside the selection the panel was opened with.
    pub in_selection: bool,
    /// Find in Files shows lines around each match.
    pub context: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        FindOptions {
            regex: false,
            case: false,
            word: false,
            wrap: true,
            in_selection: false,
            context: true,
        }
    }
}

impl FindOptions {
    /// "regex, case sensitive" — for the Find Results header.
    pub fn describe(&self) -> String {
        let mut v = vec![];
        if self.regex {
            v.push("regex");
        }
        if self.case {
            v.push("case sensitive");
        }
        if self.word {
            v.push("whole word");
        }
        if v.is_empty() {
            String::new()
        } else {
            format!(" ({})", v.join(", "))
        }
    }
}

/// A compiled search: always a regex underneath (a literal is escaped).
#[derive(Clone, Debug)]
pub struct Matcher {
    re: Regex,
    word: bool,
    regex: bool,
}

/// Lines longer than this are searched but not shown whole in Find Results.
const SHOW_MAX: usize = 400;

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Matcher {
    pub fn new(pattern: &str, o: &FindOptions) -> Result<Matcher, String> {
        if pattern.is_empty() {
            return Err("Nothing to find".into());
        }
        let src = if o.regex {
            pattern.to_string()
        } else {
            regex::escape(pattern)
        };
        let re = RegexBuilder::new(&src)
            .case_insensitive(!o.case)
            .multi_line(true)
            .size_limit(50 << 20)
            .build()
            .map_err(|e| match e {
                regex::Error::Syntax(s) => {
                    // The last line of the parser's message says what's wrong.
                    let msg = s.lines().last().unwrap_or("").trim();
                    let msg = msg.strip_prefix("error: ").unwrap_or(msg);
                    format!("Invalid regex: {msg}")
                }
                regex::Error::CompiledTooBig(_) => "Regex too big".into(),
                _ => "Invalid regex".into(),
            })?;
        Ok(Matcher {
            re,
            word: o.word,
            regex: o.regex,
        })
    }

    /// Whether the text has any match at all (cheap pre-check for whole files).
    pub fn any(&self, text: &str) -> bool {
        self.re.is_match(text)
    }

    /// Byte ranges of the matches in one line (empty matches are skipped).
    pub fn find_in(&self, line: &str) -> Vec<(usize, usize)> {
        self.re
            .find_iter(line)
            .filter(|m| m.start() < m.end())
            .filter(|m| !self.word || bounded(line, m.start(), m.end()))
            .map(|m| (m.start(), m.end()))
            .collect()
    }

    /// The text that replaces the match `range` of `line`: the template with `$1`, `\1`,
    /// `${name}`, `\n` and `\t` expanded for a regex; the template itself otherwise.
    pub fn replacement(&self, line: &str, range: (usize, usize), template: &str) -> String {
        if !self.regex {
            return template.to_string();
        }
        let tpl = regex_template(template);
        match self.re.captures_at(line, range.0) {
            Some(c) if c.get(0).is_some_and(|m| m.start() == range.0) => {
                let mut out = String::new();
                c.expand(&tpl, &mut out);
                out
            }
            _ => tpl.replace("$$", "$"),
        }
    }
}

/// A whole-word match: a word character at either end of the match isn't continued by
/// another one outside it.
fn bounded(line: &str, s: usize, e: usize) -> bool {
    let m = &line[s..e];
    let first = m.chars().next().is_some_and(is_word_char);
    let last = m.chars().next_back().is_some_and(is_word_char);
    let before = line[..s].chars().next_back().is_some_and(is_word_char);
    let after = line[e..].chars().next().is_some_and(is_word_char);
    (!first || !before) && (!last || !after)
}

/// Sublime-style replacement escapes into the regex crate's syntax.
fn regex_template(t: &str) -> String {
    let mut out = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(d @ '0'..='9') => out.push_str(&format!("${{{d}}}")),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('$') => out.push_str("$$"),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

// ================================================================ Find in Files

/// What the "Where" field asked for.
#[derive(Clone, Debug, Default)]
pub struct Where {
    /// Folders (with their project excludes) and files to search.
    pub roots: Vec<(PathBuf, Option<Folder>)>,
    includes: Option<GlobSet>,
    excludes: Option<GlobSet>,
}

fn glob_set(pats: &[String]) -> Result<Option<GlobSet>, String> {
    if pats.is_empty() {
        return Ok(None);
    }
    let mut b = GlobSetBuilder::new();
    for p in pats {
        let g = Glob::new(p).map_err(|e| format!("Bad filter '{p}': {e}"))?;
        b.add(g);
    }
    b.build().map(Some).map_err(|e| e.to_string())
}

/// The placeholder shown in an empty Where field.
pub const WHERE_DEFAULT: &str = "<project folders>";

/// Parse the Where field: comma-separated folders/files (absolute, `~/…`, or relative to
/// the first project folder), `<project folders>`, `<open files>`, `<current file>`,
/// include filters (`*.rs`) and exclude filters (`-target/`, `-*.min.js`). Nothing but
/// filters means the project folders.
pub fn parse_where(
    text: &str,
    folders: &[Folder],
    open_files: &[PathBuf],
    current: Option<&Path>,
) -> Result<Where, String> {
    let mut w = Where::default();
    let (mut inc, mut exc) = (vec![], vec![]);
    let base = folders.first().map(|f| f.path.clone());
    let add_folders = |w: &mut Where| {
        for f in folders {
            w.roots.push((f.path.clone(), Some(f.clone())));
        }
    };
    let mut any_root = false;
    for tok in text.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        match tok {
            "<project folders>" | "<open folders>" | "<project>" => {
                any_root = true;
                add_folders(&mut w);
            }
            "<open files>" => {
                any_root = true;
                w.roots.extend(open_files.iter().map(|p| (p.clone(), None)));
            }
            "<current file>" => {
                any_root = true;
                match current {
                    Some(p) => w.roots.push((p.to_path_buf(), None)),
                    None => return Err("<current file>: the file isn't saved anywhere".into()),
                }
            }
            _ => {
                if let Some(x) = tok.strip_prefix('-') {
                    exc.push(x.trim().trim_end_matches('/').to_string());
                    continue;
                }
                let expanded = crate::paths::expand_tilde(tok).or_else(|| Some(PathBuf::from(tok)));
                let path = expanded.map(|p| match (&base, p.is_absolute()) {
                    (Some(b), false) => b.join(p),
                    _ => p,
                });
                match path.filter(|p| p.exists()) {
                    Some(p) => {
                        any_root = true;
                        let p = p.canonicalize().unwrap_or(p);
                        // A project folder (or one inside it) keeps the project's excludes.
                        let folder = folders.iter().find(|f| p.starts_with(&f.path)).cloned();
                        w.roots.push((p, folder));
                    }
                    None if tok.contains(['*', '?', '[']) || !tok.contains(['/', '\\']) => {
                        inc.push(tok.trim_end_matches('/').to_string());
                    }
                    None => return Err(format!("No such folder or file: {tok}")),
                }
            }
        }
    }
    if !any_root {
        if folders.is_empty() {
            return Err("No folder open: add a folder to Where".into());
        }
        add_folders(&mut w);
    }
    w.includes = glob_set(&inc)?;
    w.excludes = glob_set(&exc)?;
    Ok(w)
}

/// One line of a file's results: a match line (with byte ranges) or a context line.
#[derive(Clone, Debug, PartialEq)]
pub struct HitLine {
    /// 0-based line number.
    pub line: usize,
    pub text: String,
    /// Byte ranges of the matches in `text` (empty for a context line).
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub struct FileHits {
    pub path: PathBuf,
    pub lines: Vec<HitLine>,
    pub matches: usize,
}

pub enum SearchEvent {
    File(FileHits),
    Done {
        files: usize,
        matches: usize,
        truncated: bool,
        elapsed: Duration,
    },
}

/// Everything a Find in Files run needs (moved to its thread).
pub struct FileSearch {
    pub matcher: Matcher,
    pub place: Where,
    /// Lines of context around matches (0 = none).
    pub context: usize,
    /// Open files are searched as they are in the editor, unsaved changes included.
    pub open: HashMap<PathBuf, String>,
}

/// Stop after this many matches (the results would be unreadable anyway).
pub const MAX_MATCHES: usize = 20_000;
/// Files bigger than this are skipped.
const MAX_FILE: u64 = 32 << 20;

impl FileSearch {
    /// Run on the calling thread, sending each file with matches as it's found (in file
    /// name order). Files are read and searched in parallel, a batch at a time.
    pub fn run(self, cancel: &AtomicBool, send: &mut dyn FnMut(SearchEvent)) {
        let t0 = Instant::now();
        let (mut files, mut matches, mut truncated) = (0usize, 0usize, false);
        let mut seen = std::collections::HashSet::new();
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(16);
        'roots: for (root, folder) in &self.place.roots {
            let list: Vec<PathBuf> = self
                .walk(root, folder.as_ref())
                .into_iter()
                .filter(|p| seen.insert(p.clone()))
                .collect();
            for batch in list.chunks(workers * 32) {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                files += batch.len();
                for h in self
                    .search_batch(batch, workers, cancel)
                    .into_iter()
                    .flatten()
                {
                    matches += h.matches;
                    send(SearchEvent::File(h));
                    if matches >= MAX_MATCHES {
                        truncated = true;
                        break 'roots;
                    }
                }
            }
        }
        send(SearchEvent::Done {
            files,
            matches,
            truncated,
            elapsed: t0.elapsed(),
        });
    }

    /// Search `batch` on `workers` threads; results in the batch's order.
    fn search_batch(
        &self,
        batch: &[PathBuf],
        workers: usize,
        cancel: &AtomicBool,
    ) -> Vec<Option<FileHits>> {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let slots: Vec<std::sync::Mutex<Option<FileHits>>> =
            batch.iter().map(|_| std::sync::Mutex::new(None)).collect();
        std::thread::scope(|sc| {
            for _ in 0..workers.min(batch.len()) {
                sc.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= batch.len() || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let hit = self.search_file(&batch[i]);
                        *slots[i].lock().unwrap() = hit;
                    }
                });
            }
        });
        slots.into_iter().map(|m| m.into_inner().unwrap()).collect()
    }

    /// Files under `root` that .gitignore, the project's excludes and the Where filters
    /// keep, in name order. A file root is just itself.
    fn walk(&self, root: &Path, folder: Option<&Folder>) -> Vec<PathBuf> {
        if root.is_file() {
            return vec![root.to_path_buf()];
        }
        let excludes = self.place.excludes.clone();
        let fold = folder.cloned();
        let base = root.to_path_buf();
        let walker = ignore::WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(false)
            .ignore(false)
            .require_git(false)
            .sort_by_file_name(|a, b| a.cmp(b))
            .filter_entry(move |e| {
                let name = e.file_name().to_string_lossy();
                if e.depth() == 0 {
                    return true;
                }
                let is_dir = e.file_type().is_some_and(|t| t.is_dir());
                if is_dir && name == ".git" {
                    return false;
                }
                if fold.as_ref().is_some_and(|f| f.excluded(&name, is_dir)) {
                    return false;
                }
                if let Some(x) = &excludes {
                    let rel = e.path().strip_prefix(&base).unwrap_or(e.path());
                    if x.is_match(&*name) || x.is_match(rel) {
                        return false;
                    }
                }
                true
            })
            .build();
        walker
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .filter(|e| match &self.place.includes {
                None => true,
                Some(inc) => {
                    let rel = e.path().strip_prefix(root).unwrap_or(e.path());
                    inc.is_match(e.file_name()) || inc.is_match(rel)
                }
            })
            .map(|e| e.into_path())
            .collect()
    }

    fn search_file(&self, path: &Path) -> Option<FileHits> {
        let owned;
        let text: &str = match self.open.get(path) {
            Some(t) => t,
            None => {
                if std::fs::metadata(path).ok()?.len() > MAX_FILE {
                    return None;
                }
                let bytes = std::fs::read(path).ok()?;
                // Binary files (NUL bytes) are skipped, like the sidebar's Goto Anything.
                if bytes.iter().take(8192).any(|&b| b == 0) {
                    return None;
                }
                owned = crate::encoding::decode(&bytes)?.0;
                &owned
            }
        };
        if !self.matcher.any(text) {
            return None;
        }
        let lines: Vec<&str> = text.lines().collect();
        let mut hits: Vec<(usize, Vec<(usize, usize)>)> = vec![];
        for (i, l) in lines.iter().enumerate() {
            let r = self.matcher.find_in(l);
            if !r.is_empty() {
                hits.push((i, r));
            }
        }
        if hits.is_empty() {
            return None;
        }
        let matches = hits.iter().map(|h| h.1.len()).sum();
        let mut out: Vec<HitLine> = vec![];
        let show = |i: usize, ranges: Vec<(usize, usize)>| {
            let (text, ranges) = clip_line(lines[i], ranges);
            HitLine {
                line: i,
                text,
                ranges,
            }
        };
        for (i, r) in hits.iter().cloned() {
            let from = i.saturating_sub(self.context);
            let next = out.last().map(|h| h.line + 1).unwrap_or(0);
            for c in from.max(next)..i {
                out.push(show(c, vec![]));
            }
            if out.last().is_some_and(|h| h.line == i) {
                // Already added as context of the previous match: it's a match line now.
                out.pop();
            }
            out.push(show(i, r));
            for c in i + 1..(i + 1 + self.context).min(lines.len()) {
                if hits.iter().any(|h| h.0 == c) {
                    break;
                }
                out.push(show(c, vec![]));
            }
        }
        Some(FileHits {
            path: path.to_path_buf(),
            lines: out,
            matches,
        })
    }
}

/// A long line is cut down for display (around its first match), dropping ranges that
/// fall outside what's shown.
fn clip_line(line: &str, ranges: Vec<(usize, usize)>) -> (String, Vec<(usize, usize)>) {
    if line.len() <= SHOW_MAX {
        return (line.to_string(), ranges);
    }
    let floor = |mut b: usize| {
        while !line.is_char_boundary(b) {
            b -= 1;
        }
        b
    };
    let start = floor(ranges.first().map(|r| r.0.saturating_sub(60)).unwrap_or(0));
    let end = floor((start + SHOW_MAX).min(line.len()));
    let pre = if start > 0 { "…" } else { "" };
    let off = pre.len() as isize - start as isize;
    let text = format!("{pre}{}…", &line[start..end]);
    let ranges = ranges
        .into_iter()
        .filter(|r| r.0 >= start && r.1 <= end)
        .map(|(a, b)| ((a as isize + off) as usize, (b as isize + off) as usize))
        .collect();
    (text, ranges)
}

/// What a line of the Find Results buffer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    Text,
    Header,
    Match,
    Context,
}

#[derive(Clone, Debug)]
pub struct ResultLine {
    pub kind: ResultKind,
    /// The file and where in it: match start (and end), or the line.
    pub target: Option<(PathBuf, crate::editor::Pos, Option<crate::editor::Pos>)>,
    /// Character columns of the matches as shown in the results line.
    pub spans: Vec<(usize, usize)>,
}

/// Kept on the Find Results buffer: one entry per line of its text.
#[derive(Clone, Debug, Default)]
pub struct Results {
    pub lines: Vec<ResultLine>,
    pub files: usize,
    pub matches: usize,
    pub done: bool,
}

/// Start a search on its own thread; `cancel` stops it early.
pub fn spawn(
    search: FileSearch,
    cancel: Arc<AtomicBool>,
    mut send: impl FnMut(SearchEvent) + Send + 'static,
) {
    std::thread::spawn(move || search.run(&cancel, &mut send));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, f: impl Fn(&mut FindOptions)) -> Matcher {
        let mut o = FindOptions::default();
        f(&mut o);
        Matcher::new(p, &o).unwrap()
    }

    #[test]
    fn literal_case_word_and_regex() {
        // Literal: regex characters mean themselves; case-insensitive by default.
        assert_eq!(
            m("a.b", |_| {}).find_in("A.B axb a.b"),
            vec![(0, 3), (8, 11)]
        );
        assert_eq!(m("Foo", |o| o.case = true).find_in("foo Foo"), vec![(4, 7)]);
        // Whole word: inside another word doesn't count; punctuation ends a word.
        let w = m("foo", |o| o.word = true);
        assert_eq!(w.find_in("foo foobar _foo (foo)"), vec![(0, 3), (17, 20)]);
        assert_eq!(
            m("foo(", |o| o.word = true).find_in("xfoo( foo("),
            vec![(6, 10)]
        );
        // Regex, with anchors per line and no empty matches.
        let r = m(r"^\s*(\w+) = (\d+)", |o| o.regex = true);
        assert_eq!(r.find_in("  x = 10"), vec![(0, 8)]);
        assert!(m("x*", |o| o.regex = true).find_in("abc").is_empty());
        let e = Matcher::new(
            "(a",
            &FindOptions {
                regex: true,
                ..Default::default()
            },
        );
        assert!(
            e.unwrap_err().starts_with("Invalid regex"),
            "readable error"
        );
    }

    #[test]
    fn replacement_templates() {
        let r = m(r"(\w+)=(\w+)", |o| o.regex = true);
        let line = "a=b c=d";
        assert_eq!(r.replacement(line, (4, 7), "$2=$1"), "d=c");
        assert_eq!(r.replacement(line, (0, 3), r"\2:\1\t\\"), "b:a\t\\");
        assert_eq!(r.replacement(line, (0, 3), r"\$1 $$"), "$1 $");
        let n = m(r"(?P<k>\w+)=", |o| o.regex = true);
        assert_eq!(n.replacement(line, (0, 2), "${k}:"), "a:");
        // A literal search inserts the template as written.
        assert_eq!(m("a", |_| {}).replacement("a", (0, 1), "$1\\n"), "$1\\n");
    }

    #[test]
    fn where_field() {
        let d = std::env::temp_dir().join(format!("farsub-where-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        let d = d.canonicalize().unwrap();
        let folders = vec![Folder::plain(d.clone())];
        // Empty or filters only: the project folders.
        let w = parse_where("", &folders, &[], None).unwrap();
        assert_eq!(w.roots.len(), 1);
        let w = parse_where("*.rs, -target/", &folders, &[], None).unwrap();
        assert_eq!(w.roots[0].0, d);
        assert!(w.includes.unwrap().is_match("a.rs"));
        assert!(w.excludes.unwrap().is_match("target"));
        // A relative folder resolves against the first project folder.
        let w = parse_where("src", &folders, &[], None).unwrap();
        assert_eq!(w.roots[0].0, d.join("src"));
        assert!(w.roots[0].1.is_some(), "keeps the project's excludes");
        assert!(parse_where("nope/x", &folders, &[], None).is_err());
        assert!(parse_where("", &[], &[], None).is_err());
        let f = d.join("a.txt");
        let w = parse_where("<open files>", &[], std::slice::from_ref(&f), None).unwrap();
        assert_eq!(w.roots[0].0, f);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn files_are_searched_with_context_ignores_and_unsaved_text() {
        let d = std::env::temp_dir().join(format!("farsub-fif-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("target")).unwrap();
        std::fs::create_dir_all(d.join("skip")).unwrap();
        let d = d.canonicalize().unwrap();
        std::fs::write(d.join(".gitignore"), "skip/\n").unwrap();
        std::fs::write(
            d.join("src/a.rs"),
            "one\ntwo needle\nthree\nfour\nfive\nsix needle\n",
        )
        .unwrap();
        std::fs::write(d.join("src/b.txt"), "needle\n").unwrap();
        std::fs::write(d.join("target/c.rs"), "needle\n").unwrap();
        std::fs::write(d.join("skip/d.rs"), "needle\n").unwrap();
        std::fs::write(d.join("bin.dat"), b"needle\0\x01").unwrap();
        std::fs::write(d.join("open.rs"), "on disk\n").unwrap();
        let folders = vec![Folder::plain(d.clone())];
        let run = |where_: &str, context: usize, open: HashMap<PathBuf, String>| {
            let s = FileSearch {
                matcher: m("needle", |_| {}),
                place: parse_where(where_, &folders, &[], None).unwrap(),
                context,
                open,
            };
            let mut files = vec![];
            let mut done = None;
            s.run(&AtomicBool::new(false), &mut |e| match e {
                SearchEvent::File(h) => files.push(h),
                SearchEvent::Done { files, matches, .. } => done = Some((files, matches)),
            });
            (files, done.unwrap())
        };
        let names = |f: &[FileHits]| -> Vec<String> {
            f.iter()
                .map(|h| h.path.strip_prefix(&d).unwrap().display().to_string())
                .collect()
        };
        let (f, _) = run("", 0, HashMap::new());
        // .gitignore'd and binary files are skipped; name order.
        assert_eq!(names(&f), ["src/a.rs", "src/b.txt", "target/c.rs"]);
        let (f, (_, n)) = run("*.rs, -target", 0, HashMap::new());
        assert_eq!(names(&f), ["src/a.rs"]);
        assert_eq!(n, 2);
        // Context lines around the matches, without repeating shared ones.
        let (f, _) = run("*.rs, -target", 2, HashMap::new());
        let lines: Vec<(usize, bool)> = f[0]
            .lines
            .iter()
            .map(|l| (l.line, !l.ranges.is_empty()))
            .collect();
        assert_eq!(
            lines,
            [
                (0, false),
                (1, true),
                (2, false),
                (3, false),
                (4, false),
                (5, true)
            ]
        );
        // An open file is searched as it is in the editor.
        let open = HashMap::from([(d.join("open.rs"), "unsaved needle".to_string())]);
        let (f, _) = run("open.rs", 0, open);
        assert_eq!(f[0].lines[0].text, "unsaved needle");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    #[ignore]
    /// Find in Files speed on a big tree (cargo test --release -- --ignored bench_find_in_files --nocapture).
    fn bench_find_in_files() {
        let Some(root) = crate::paths::home_dir().map(|h| h.join(".cargo/registry/src")) else {
            return;
        };
        let folders = vec![Folder::plain(root)];
        for q in ["fn main", "zzqqxx_not_there"] {
            let s = FileSearch {
                matcher: m(q, |_| {}),
                place: parse_where("", &folders, &[], None).unwrap(),
                context: 2,
                open: HashMap::new(),
            };
            let mut done = None;
            s.run(&AtomicBool::new(false), &mut |e| {
                if let SearchEvent::Done {
                    files,
                    matches,
                    elapsed,
                    ..
                } = e
                {
                    done = Some((files, matches, elapsed));
                }
            });
            let (files, matches, elapsed) = done.unwrap();
            eprintln!(
                "{q:?}: {files} files, {matches} matches, {:.0} ms",
                elapsed.as_secs_f64() * 1000.0
            );
        }
    }

    #[test]
    fn long_lines_are_clipped_around_the_match() {
        let line = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        let (t, r) = clip_line(&line, vec![(1000, 1006)]);
        assert!(t.len() < 500);
        assert_eq!(&t[r[0].0..r[0].1], "needle");
    }
}
