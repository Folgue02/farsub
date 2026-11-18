//! App: the find panel (Find, Replace, Find in Files) and the Find Results buffer.
//!
//! Like Sublime: one panel at the bottom of the window with toggles (regex, case, whole
//! word, wrap, in selection, context), searching as you type in a file; Find in Files
//! writes its results into a read-only "Find Results" tab whose lines open their match.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::search::{self, FileHits, FindOptions, Matcher, ResultKind, ResultLine, SearchEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindMode {
    Find,
    Replace,
    Files,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindField {
    Find,
    Where,
    Replace,
}

/// A find-panel toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindOpt {
    Regex,
    Case,
    Word,
    Wrap,
    InSelection,
    Context,
}

/// The panel's state; its texts are kept while it's closed.
pub struct FindPanel {
    pub open: bool,
    pub mode: FindMode,
    pub field: FindField,
    pub find: LineEdit,
    pub replace: LineEdit,
    pub place: LineEdit,
    /// Where the cursor was when the panel opened (searching as you type starts there).
    origin: Pos,
    /// The selection the panel was opened with ("in selection" searches only there).
    pub scope: Option<(Pos, Pos)>,
    /// A bad regex or Where field: the field it's about and what's wrong.
    pub error: Option<(FindField, String)>,
    pub not_found: bool,
}

impl Default for FindPanel {
    fn default() -> Self {
        FindPanel {
            open: false,
            mode: FindMode::Find,
            field: FindField::Find,
            find: LineEdit::new(""),
            replace: LineEdit::new(""),
            place: LineEdit::new(""),
            origin: Pos::default(),
            scope: None,
            error: None,
            not_found: false,
        }
    }
}

impl FindPanel {
    /// The fields the mode shows, top to bottom.
    pub fn fields(&self) -> &'static [FindField] {
        match self.mode {
            FindMode::Find => &[FindField::Find],
            FindMode::Replace => &[FindField::Find, FindField::Replace],
            FindMode::Files => &[FindField::Find, FindField::Where, FindField::Replace],
        }
    }

    pub fn field_mut(&mut self, f: FindField) -> &mut LineEdit {
        match f {
            FindField::Find => &mut self.find,
            FindField::Where => &mut self.place,
            FindField::Replace => &mut self.replace,
        }
    }

    pub fn field_ref(&self, f: FindField) -> &LineEdit {
        match f {
            FindField::Find => &self.find,
            FindField::Where => &self.place,
            FindField::Replace => &self.replace,
        }
    }

    /// The toggles the mode shows, in order.
    pub fn opts(&self) -> &'static [FindOpt] {
        use FindOpt::*;
        match self.mode {
            FindMode::Files => &[Regex, Case, Word, Context],
            _ => &[Regex, Case, Word, Wrap, InSelection],
        }
    }
}

/// Match count for the panel, cached per (buffer, version, query, options, scope).
pub struct FindCount {
    key: (usize, i32, String, FindOptions, Option<(Pos, Pos)>),
    pub total: usize,
    matches: Vec<(Pos, Pos)>,
}

/// A running (or finished) Find in Files.
pub struct FileRun {
    run_id: u64,
    cancel: Arc<AtomicBool>,
    /// The Find Results buffer it writes to.
    buf_id: usize,
    matcher: Matcher,
    template: String,
    /// Ask to replace when the search is done.
    replace_after: bool,
    files: Vec<PathBuf>,
    matches: usize,
    pub done: bool,
}

/// Count matches up to this many (more is shown as "50000+").
const COUNT_MAX: usize = 50_000;

pub fn opt_name(o: FindOpt) -> &'static str {
    match o {
        FindOpt::Regex => "Regular expression",
        FindOpt::Case => "Case sensitive",
        FindOpt::Word => "Whole word",
        FindOpt::Wrap => "Wrap around",
        FindOpt::InSelection => "In selection",
        FindOpt::Context => "Show context",
    }
}

pub fn opt_command(o: FindOpt) -> Command {
    match o {
        FindOpt::Regex => Command::ToggleFindRegex,
        FindOpt::Case => Command::ToggleFindCase,
        FindOpt::Word => Command::ToggleFindWord,
        FindOpt::Wrap => Command::ToggleFindWrap,
        FindOpt::InSelection => Command::ToggleFindInSelection,
        FindOpt::Context => Command::ToggleFindContext,
    }
}

impl App {
    /// Whether the option is on.
    pub fn find_opt(&self, o: FindOpt) -> bool {
        let f = &self.find_opts;
        match o {
            FindOpt::Regex => f.regex,
            FindOpt::Case => f.case,
            FindOpt::Word => f.word,
            FindOpt::Wrap => f.wrap,
            FindOpt::InSelection => f.in_selection,
            FindOpt::Context => f.context,
        }
    }

    /// Commands of the find panel; false if `cmd` isn't one.
    pub(super) fn find_command(&mut self, cmd: Command) -> bool {
        use Command::*;
        match cmd {
            Find => self.open_find(FindMode::Find),
            Replace => self.open_find(FindMode::Replace),
            FindInFiles => self.open_find(FindMode::Files),
            FindNext => self.find_next(true),
            FindPrev => self.find_next(false),
            ReplaceNext => self.replace_next(),
            ReplaceAll => self.replace_all(),
            FindAll => self.find_all(),
            NextResult => self.step_result(true),
            PrevResult => self.step_result(false),
            ToggleFindRegex => self.toggle_find_opt(FindOpt::Regex),
            ToggleFindCase => self.toggle_find_opt(FindOpt::Case),
            ToggleFindWord => self.toggle_find_opt(FindOpt::Word),
            ToggleFindWrap => self.toggle_find_opt(FindOpt::Wrap),
            ToggleFindInSelection => self.toggle_find_opt(FindOpt::InSelection),
            ToggleFindContext => self.toggle_find_opt(FindOpt::Context),
            _ => return false,
        }
        true
    }

    pub fn open_find(&mut self, mode: FindMode) {
        if mode != FindMode::Files && self.buf().is_none() {
            return;
        }
        self.completion = None;
        self.hover = None;
        let (sel_text, origin, scope) = match self.buf() {
            Some(b) => {
                let sel = b.selection();
                let text = b.selected_text();
                let multi = text.as_ref().is_some_and(|t| t.contains('\n'));
                (
                    text.filter(|_| !multi),
                    sel.map(|s| s.0).unwrap_or(b.cursor),
                    // A selection of several lines is what "in selection" searches.
                    sel.filter(|_| multi),
                )
            }
            None => (None, Pos::default(), None),
        };
        let p = &mut self.find;
        p.open = true;
        p.mode = mode;
        p.field = FindField::Find;
        p.error = None;
        p.not_found = false;
        p.origin = origin;
        p.scope = scope;
        if let Some(t) = sel_text {
            p.find.set(&t);
        } else if p.find.is_empty()
            && let Some(s) = &self.search
        {
            p.find.set(s);
        }
        p.find.command(Command::SelectAll);
        self.focus = Focus::Find;
        if mode != FindMode::Files && !self.find.find.is_empty() {
            self.find_changed();
        }
    }

    pub fn close_find(&mut self) {
        self.find.open = false;
        if self.focus == Focus::Find {
            self.focus = Focus::Editor;
        }
    }

    /// Focus a field (clicked).
    pub fn focus_find_field(&mut self, f: FindField) {
        if self.find.fields().contains(&f) {
            self.find.field = f;
            self.focus = Focus::Find;
        }
    }

    pub fn toggle_find_opt(&mut self, o: FindOpt) {
        let f = &mut self.find_opts;
        let v = match o {
            FindOpt::Regex => &mut f.regex,
            FindOpt::Case => &mut f.case,
            FindOpt::Word => &mut f.word,
            FindOpt::Wrap => &mut f.wrap,
            FindOpt::InSelection => &mut f.in_selection,
            FindOpt::Context => &mut f.context,
        };
        *v = !*v;
        let on = *v;
        if o == FindOpt::InSelection && on && self.find.scope.is_none() {
            // Take the current selection as the scope (Sublime does the same).
            self.find.scope = self.buf().and_then(|b| b.selection());
            if self.find.scope.is_none() {
                self.msg("In selection: select some text first");
            }
        }
        if !self.find.open {
            self.msg(format!(
                "Find: {} {}",
                opt_name(o),
                if on { "on" } else { "off" }
            ));
        }
        if self.find.open && self.find.mode != FindMode::Files {
            self.find_changed();
        }
    }

    /// A compiled matcher for `pattern` with the current options (cached).
    pub fn matcher_for(&mut self, pattern: &str) -> Result<Matcher, String> {
        if let Some((p, o, m)) = &self.matcher_cache
            && p == pattern
            && *o == self.find_opts
        {
            return m.clone();
        }
        let m = Matcher::new(pattern, &self.find_opts);
        self.matcher_cache = Some((pattern.to_string(), self.find_opts, m.clone()));
        m
    }

    fn scope(&self) -> Option<(Pos, Pos)> {
        self.find_opts
            .in_selection
            .then_some(self.find.scope)
            .flatten()
    }

    /// The matcher whose matches the editor highlights (the panel is open on a file).
    pub fn highlight_matcher(&mut self) -> Option<Matcher> {
        if !self.find.open || self.find.mode == FindMode::Files || self.find.find.is_empty() {
            return None;
        }
        let q = self.find.find.text().to_string();
        self.matcher_for(&q).ok()
    }

    /// Search as you type: select the first match from where the panel opened.
    pub(super) fn find_changed(&mut self) {
        if self.find.field != FindField::Find || self.find.mode == FindMode::Files {
            self.find.error = None;
            return;
        }
        let q = self.find.find.text().to_string();
        self.find.not_found = false;
        if q.is_empty() {
            self.find.error = None;
            let o = self.find.origin;
            if let Some(b) = self.buf_mut() {
                b.goto(o);
            }
            return;
        }
        let m = match self.matcher_for(&q) {
            Ok(m) => m,
            Err(e) => {
                self.find.error = Some((FindField::Find, e));
                return;
            }
        };
        self.find.error = None;
        self.search = Some(q);
        let (origin, wrap, scope, pg) = (
            self.find.origin,
            self.find_opts.wrap,
            self.scope(),
            self.page,
        );
        let Some(b) = self.buf_mut() else { return };
        let from = scope.map(|s| s.0.max(origin)).unwrap_or(origin);
        match b.find_with(&m, from, true, wrap, scope) {
            Some((s, e)) => select_match(b, s, e, pg),
            None => {
                b.goto(origin);
                self.find.not_found = true;
            }
        }
    }

    /// Keys of the focused panel that aren't text editing. True if handled.
    pub(super) fn find_key(&mut self, k: Key) -> bool {
        let plain = !k.ctrl && !k.alt && !k.sup;
        match k.code {
            KeyCode::Esc => {
                self.close_find();
                true
            }
            KeyCode::Tab if plain => {
                let fs = self.find.fields();
                let i = fs.iter().position(|f| *f == self.find.field).unwrap_or(0);
                let n = fs.len();
                let j = if k.shift {
                    (i + n - 1) % n
                } else {
                    (i + 1) % n
                };
                self.find.field = fs[j];
                self.find.field_mut(fs[j]).command(Command::SelectAll);
                true
            }
            KeyCode::Enter if plain => {
                match (self.find.mode, self.find.field) {
                    (FindMode::Files, FindField::Replace) => self.find_in_files(true),
                    (FindMode::Files, _) => self.find_in_files(false),
                    (FindMode::Replace, FindField::Replace) if !k.shift => self.replace_next(),
                    _ => self.find_next(!k.shift),
                }
                true
            }
            // Alt+Enter: a cursor on every match (Sublime's Find All).
            KeyCode::Enter if k.alt && !k.ctrl && !k.sup && self.find.mode != FindMode::Files => {
                self.find_all();
                true
            }
            _ => false,
        }
    }

    /// Select every match in the file (each one a cursor) and go back to the editor.
    pub(super) fn find_all(&mut self) {
        let q = self.find.find.text().to_string();
        if q.is_empty() {
            return self.msg("Nothing to find");
        }
        let m = match self.matcher_for(&q) {
            Ok(m) => m,
            Err(e) => return self.msg(e),
        };
        let scope = self.scope();
        let Some(b) = self.buf_mut() else { return };
        let all = b.matches_with(&m, scope, usize::MAX);
        if all.is_empty() {
            self.find.not_found = true;
            return self.msg(format!("'{q}' not found"));
        }
        let n = all.len();
        b.select_ranges(&all);
        self.close_find();
        self.focus = Focus::Editor;
        self.msg(format!("{n} selected"));
    }

    /// Find Next / Previous (Enter / Shift+Enter in the panel, F3 / Shift+F3).
    pub(super) fn find_next(&mut self, forward: bool) {
        let q = Some(self.find.find.text().to_string())
            .filter(|q| !q.is_empty() && (self.find.open || self.search.is_none()))
            .or_else(|| self.search.clone())
            .filter(|q| !q.is_empty());
        let Some(q) = q else {
            let k = self.keymap.shortcut(Command::Find);
            return self.msg(format!("Nothing to find ({k})"));
        };
        let m = match self.matcher_for(&q) {
            Ok(m) => m,
            Err(e) => {
                self.find.error = Some((FindField::Find, e.clone()));
                return self.msg(e);
            }
        };
        self.search = Some(q.clone());
        let (wrap, scope, pg) = (self.find_opts.wrap, self.scope(), self.page);
        let Some(b) = self.buf_mut() else { return };
        let from = match (b.selection(), forward) {
            (Some((_, e)), true) => e,
            (Some((s, _)), false) => s,
            (None, _) => b.cursor,
        };
        match b.find_with(&m, from, forward, wrap, scope) {
            Some((s, e)) => {
                let wrapped = if forward { s < from } else { s > from };
                select_match(b, s, e, pg);
                self.find.not_found = false;
                if wrapped {
                    self.msg(if forward {
                        "Find wrapped to the top"
                    } else {
                        "Find wrapped to the bottom"
                    });
                }
            }
            None => {
                self.find.not_found = true;
                self.msg(format!("'{q}' not found"));
            }
        }
    }

    /// Replace the selected match, then select the next one.
    pub(super) fn replace_next(&mut self) {
        let q = self.find.find.text().to_string();
        let Ok(m) = self.matcher_for(&q) else {
            return self.find_next(true);
        };
        let t = self.find.replace.text().to_string();
        let scope = self.scope();
        if let Some(b) = self.buf_mut()
            && let Some(sel) = b.selection()
            && b.find_with(&m, sel.0, true, false, scope) == Some(sel)
        {
            if b.read_only {
                return self.msg("This file is read-only");
            }
            b.replace_match(&m, sel, &t);
        }
        self.find_next(true);
    }

    pub(super) fn replace_all(&mut self) {
        let q = self.find.find.text().to_string();
        let m = match self.matcher_for(&q) {
            Ok(m) => m,
            Err(e) => return self.msg(e),
        };
        let t = self.find.replace.text().to_string();
        let scope = self.scope();
        let Some(b) = self.buf_mut() else { return };
        if b.read_only {
            return self.msg("This file is read-only");
        }
        let n = b.replace_with(&m, &t, scope);
        // The selection's text changed: its old range no longer means anything.
        self.find.scope = None;
        self.msg(match n {
            0 => format!("'{q}' not found"),
            1 => "Replaced 1 occurrence".into(),
            n => format!("Replaced {n} occurrences"),
        });
    }

    /// "3 of 12" for the panel (recounted when the file or the query changes).
    pub fn find_count(&mut self) -> Option<(Option<usize>, usize)> {
        if !self.find.open || self.find.mode == FindMode::Files || self.find.find.is_empty() {
            return None;
        }
        let q = self.find.find.text().to_string();
        let m = self.matcher_for(&q).ok()?;
        let scope = self.scope();
        let opts = self.find_opts;
        let b = self.buf()?;
        let key = (b.id, b.version, q, opts, scope);
        let sel = b.selection();
        if self.find_counted.as_ref().is_none_or(|c| c.key != key) {
            let matches = self.buf()?.matches_with(&m, scope, COUNT_MAX + 1);
            self.find_counted = Some(FindCount {
                key,
                total: matches.len(),
                matches,
            });
        }
        let c = self.find_counted.as_ref()?;
        let at = sel.and_then(|s| c.matches.binary_search(&s).ok());
        Some((at, c.total))
    }

    // ============================================================ Find in Files

    /// Whether the last Find in Files has finished.
    #[cfg(test)]
    pub fn find_in_files_done(&self) -> bool {
        self.file_run.as_ref().is_some_and(|r| r.done)
    }

    /// The panel's Find / Replace buttons in Find in Files.
    pub fn run_find_in_files(&mut self, replace: bool) {
        self.find_in_files(replace);
    }

    /// A folder picked with "Add Folder…" joins the Where field.
    pub fn add_find_folder(&mut self, path: PathBuf) {
        let cur = self
            .find
            .place
            .text()
            .trim()
            .trim_end_matches(',')
            .to_string();
        let p = path.display().to_string();
        let text = if cur.is_empty() {
            p
        } else {
            format!("{cur}, {p}")
        };
        self.find.place.set(&text);
        self.find.field = FindField::Where;
        self.focus = Focus::Find;
    }

    /// Start a Find in Files from the panel (`replace`: ask to replace when it's done).
    pub(super) fn find_in_files(&mut self, replace: bool) {
        let q = self.find.find.text().to_string();
        let m = match self.matcher_for(&q) {
            Ok(m) => m,
            Err(e) => {
                self.find.error = Some((FindField::Find, e));
                self.find.field = FindField::Find;
                return;
            }
        };
        let open: Vec<PathBuf> = self.buffers.iter().filter_map(|b| b.path.clone()).collect();
        let current = self.buf().and_then(|b| b.path.clone());
        let place_text = self.find.place.text().to_string();
        let place = match search::parse_where(
            &place_text,
            &self.project.folders,
            &open,
            current.as_deref(),
        ) {
            Ok(w) => w,
            Err(e) => {
                self.find.error = Some((FindField::Where, e));
                self.find.field = FindField::Where;
                return;
            }
        };
        self.find.error = None;
        self.search = Some(q.clone());
        if let Some(r) = &self.file_run {
            r.cancel.store(true, Ordering::Relaxed);
        }
        // Unsaved changes are searched (and replaced) as they are in the editor.
        let unsaved = self
            .buffers
            .iter()
            .filter(|b| b.modified)
            .filter_map(|b| Some((b.path.clone()?, b.lines.join("\n"))))
            .collect();
        let buf_id = self.results_buffer();
        let where_label = if place_text.trim().is_empty() {
            search::WHERE_DEFAULT.to_string()
        } else {
            place_text.trim().to_string()
        };
        let header = format!(
            "Searching for \"{q}\"{} in {where_label}\n\n",
            self.find_opts.describe()
        );
        if let Some(i) = self.buf_index(buf_id) {
            let b = &mut self.buffers[i];
            b.append_generated(&header);
            let r = b.results.get_or_insert_default();
            for _ in 0..2 {
                r.lines.push(ResultLine::text());
            }
        }
        self.search_gen += 1;
        let run_id = self.search_gen;
        let cancel = Arc::new(AtomicBool::new(false));
        self.file_run = Some(FileRun {
            run_id,
            cancel: cancel.clone(),
            buf_id,
            matcher: m.clone(),
            template: self.find.replace.text().to_string(),
            replace_after: replace,
            files: vec![],
            matches: 0,
            done: false,
        });
        let context = if self.find_opts.context { 2 } else { 0 };
        let tx = self.tx.clone();
        search::spawn(
            search::FileSearch {
                matcher: m,
                place,
                context,
                open: unsaved,
            },
            cancel,
            move |ev| {
                tx.send(AppEvent::Search(run_id, ev));
            },
        );
        self.find.open = false;
        self.focus = Focus::Editor;
    }

    /// The Find Results buffer, emptied (created in the focused group if there's none).
    fn results_buffer(&mut self) -> usize {
        if let Some(i) = self.buffers.iter().position(|b| b.results.is_some()) {
            let b = &mut self.buffers[i];
            b.lines = vec![String::new()];
            b.cursor = Pos::default();
            b.anchor = None;
            b.scroll_y = 0;
            b.scroll_row = 0;
            b.scroll_px = 0.0;
            b.hl.reset(String::new(), None);
            b.version += 1;
            b.results = Some(Box::default());
            let id = b.id;
            self.set_cur(i);
            return id;
        }
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let mut b = Buffer::from_text(id, None, "", HlCache::new(String::new()));
        b.title = Some("Find Results".into());
        b.read_only = true;
        b.word_wrap = Some(false);
        b.results = Some(Box::default());
        let i = self.push_buffer(b);
        self.set_cur(i);
        id
    }

    pub(super) fn on_search_event(&mut self, run_id: u64, ev: SearchEvent) {
        let Some(run) = self.file_run.as_mut().filter(|r| r.run_id == run_id) else {
            return;
        };
        let buf_id = run.buf_id;
        let mut status = None;
        let mut confirm = None;
        let text = match ev {
            SearchEvent::File(h) => {
                run.files.push(h.path.clone());
                run.matches += h.matches;
                let (text, lines) = render_file(&h);
                if let Some(i) = self.buf_index(buf_id)
                    && let Some(r) = self.buffers[i].results.as_mut()
                {
                    r.lines.extend(lines);
                    r.matches += h.matches;
                    r.files += 1;
                }
                text
            }
            SearchEvent::Done {
                files,
                matches,
                truncated,
                elapsed,
            } => {
                run.done = true;
                let shown = run.files.len();
                let mut s = match matches {
                    0 => format!("No matches in {files} files"),
                    _ => format!(
                        "{matches} match{} across {shown} file{} ({files} searched)",
                        if matches == 1 { "" } else { "es" },
                        if shown == 1 { "" } else { "s" },
                    ),
                };
                if truncated {
                    s.push_str(&format!(" — stopped after {} matches", search::MAX_MATCHES));
                }
                s.push_str(&format!(" in {:.2} s\n", elapsed.as_secs_f64()));
                status = Some(s.trim_end().to_string());
                if run.replace_after {
                    run.replace_after = false;
                    if run.matches > 0 {
                        let msg = format!(
                            "Replace {} match{} in {} file{} with \"{}\"? The files open with the changes unsaved (File › Save All writes them).",
                            run.matches,
                            if run.matches == 1 { "" } else { "es" },
                            run.files.len(),
                            if run.files.len() == 1 { "" } else { "s" },
                            run.template,
                        );
                        confirm = Some(Confirm {
                            message: msg,
                            kind: ConfirmKind::ReplaceInFiles,
                        });
                    }
                }
                if let Some(i) = self.buf_index(buf_id)
                    && let Some(r) = self.buffers[i].results.as_mut()
                {
                    r.lines.push(ResultLine::text());
                    r.done = true;
                }
                s
            }
        };
        if let Some(i) = self.buf_index(buf_id) {
            self.buffers[i].append_generated(&text);
        }
        if let Some(s) = status {
            self.msg(s);
        }
        if let Some(c) = confirm {
            self.modal = Some(Modal::Confirm(c));
        }
    }

    /// Replace in the files the last Find in Files found (confirmed).
    pub(super) fn replace_in_files(&mut self) {
        let Some(run) = self.file_run.take() else {
            return;
        };
        let (cur, group) = (self.cur, self.group);
        let (mut n, mut files) = (0, 0);
        for p in &run.files {
            let Some(i) = self.open_file(p, false) else {
                continue;
            };
            let k = self.buffers[i].replace_with(&run.matcher, &run.template, None);
            if k > 0 {
                n += k;
                files += 1;
            }
        }
        self.cur = cur.min(self.buffers.len().saturating_sub(1));
        self.group = group;
        self.msg(format!(
            "Replaced {n} match{} in {files} file{} — unsaved: File › Save All writes them",
            if n == 1 { "" } else { "es" },
            if files == 1 { "" } else { "s" },
        ));
        self.file_run = Some(run);
    }

    /// Enter / double-click on a Find Results line: open what it points at.
    pub fn open_result_line(&mut self, line: usize) -> bool {
        let Some(t) = self
            .buf()
            .and_then(|b| b.results.as_ref())
            .and_then(|r| r.lines.get(line))
            .and_then(|l| l.target.clone())
        else {
            return false;
        };
        self.open_target(t);
        true
    }

    fn open_target(&mut self, (path, s, e): (PathBuf, Pos, Option<Pos>)) {
        self.goto_location(&path, Some(s));
        if let Some(e) = e
            && let Some(b) = self.buf_mut()
            && b.path.as_deref() == Some(path.as_path())
        {
            b.anchor = Some(s);
            b.cursor = e;
        }
    }

    /// F4 / Shift+F4: the next / previous match line of Find Results.
    fn step_result(&mut self, forward: bool) {
        let Some(i) = self.buffers.iter().position(|b| b.results.is_some()) else {
            let k = self.keymap.shortcut(Command::FindInFiles);
            return self.msg(format!("No find results (Find in Files: {k})"));
        };
        let b = &self.buffers[i];
        let r = b.results.as_ref().expect("results");
        let at = b.cursor.line;
        let is_match = |l: &usize| r.lines[*l].kind == ResultKind::Match;
        let n = r.lines.len();
        let next = if forward {
            (at + 1..n).find(is_match).or_else(|| (0..n).find(is_match))
        } else {
            (0..at)
                .rev()
                .find(is_match)
                .or_else(|| (0..n).rev().find(is_match))
        };
        let Some(l) = next else {
            return self.msg("No find results");
        };
        let target = r.lines[l].target.clone();
        let pg = self.page;
        let b = &mut self.buffers[i];
        b.goto(Pos::new(l, 0));
        if l < b.scroll_y || l >= b.scroll_y + pg {
            center(b, pg);
        }
        if let Some(t) = target {
            self.open_target(t);
        }
    }
}

fn select_match(b: &mut Buffer, s: Pos, e: Pos, pg: usize) {
    b.anchor = Some(s);
    b.cursor = e;
    if e.line < b.scroll_y || e.line >= b.scroll_y + pg.max(1) {
        center(b, pg);
    }
}

impl ResultLine {
    fn text() -> ResultLine {
        ResultLine {
            kind: ResultKind::Text,
            target: None,
            spans: vec![],
        }
    }
}

/// Width of the line-number column in Find Results ("  1234: ").
const NUM_W: usize = 6;

/// A file's block in Find Results (text, plus what each of its lines is).
fn render_file(h: &FileHits) -> (String, Vec<ResultLine>) {
    let mut text = format!("{}:\n", h.path.display());
    let first = h.lines.iter().find(|l| !l.ranges.is_empty());
    let mut lines = vec![ResultLine {
        kind: ResultKind::Header,
        target: first.map(|l| (h.path.clone(), Pos::new(l.line, 0), None)),
        spans: vec![],
    }];
    let mut prev: Option<usize> = None;
    for l in &h.lines {
        if prev.is_some_and(|p| l.line > p + 1) {
            text.push_str(&format!("{:>NUM_W$}\n", ".."));
            lines.push(ResultLine::text());
        }
        prev = Some(l.line);
        let is_match = !l.ranges.is_empty();
        text.push_str(&format!(
            "{:>NUM_W$}{} {}\n",
            l.line + 1,
            if is_match { ':' } else { ' ' },
            l.text
        ));
        let pre = NUM_W + 2;
        let chars = |b: usize| char_len(&l.text[..b]);
        let target = match l.ranges.first() {
            Some(&(s, e)) => (
                h.path.clone(),
                Pos::new(l.line, chars(s)),
                Some(Pos::new(l.line, chars(e))),
            ),
            None => (h.path.clone(), Pos::new(l.line, 0), None),
        };
        lines.push(ResultLine {
            kind: if is_match {
                ResultKind::Match
            } else {
                ResultKind::Context
            },
            target: Some(target),
            spans: l
                .ranges
                .iter()
                .map(|&(s, e)| (pre + chars(s), pre + chars(e)))
                .collect(),
        });
    }
    text.push('\n');
    lines.push(ResultLine::text());
    (text, lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::HitLine;

    #[test]
    fn results_block_lines_up_with_its_text() {
        let h = FileHits {
            path: PathBuf::from("/p/a.rs"),
            lines: vec![
                HitLine {
                    line: 0,
                    text: "fn é() { x }".into(),
                    ranges: vec![(10, 11)],
                },
                HitLine {
                    line: 1,
                    text: "ctx".into(),
                    ranges: vec![],
                },
                HitLine {
                    line: 9,
                    text: "x".into(),
                    ranges: vec![(0, 1)],
                },
            ],
            matches: 2,
        };
        let (text, lines) = render_file(&h);
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(
            rows,
            [
                "/p/a.rs:",
                "     1: fn é() { x }",
                "     2  ctx",
                "    ..",
                "    10: x",
                "",
                ""
            ]
        );
        assert_eq!(lines.len(), rows.len() - 1, "one entry per line of text");
        assert_eq!(lines[1].kind, ResultKind::Match);
        // Spans are character columns in the results line.
        let (s, e) = lines[1].spans[0];
        let shown: String = rows[1].chars().skip(s).take(e - s).collect();
        assert_eq!(shown, "x");
        assert_eq!(
            lines[1].target,
            Some((
                PathBuf::from("/p/a.rs"),
                Pos::new(0, 9),
                Some(Pos::new(0, 10))
            ))
        );
        assert_eq!(lines[3].kind, ResultKind::Text);
        assert_eq!(lines[4].target.as_ref().unwrap().1, Pos::new(9, 0));
    }
}
