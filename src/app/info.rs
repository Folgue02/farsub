//! App: syntax & LSP info.

use super::*;

impl App {
    // ===================================================================== syntax & LSP info

    /// The language config that goes with a syntax (an explicit `syntax = …` wins, then
    /// shared file extensions).
    /// The language named `name` (by name or title, any case — older workspaces stored
    /// titles).
    pub(super) fn language_for_syntax(&self, name: &str) -> Option<crate::config::LanguageConfig> {
        self.config
            .languages
            .iter()
            .rev()
            .find(|l| l.name == name || l.title().eq_ignore_ascii_case(name))
            .cloned()
    }

    /// Set a buffer's language by hand ("Plain Text" for none); its highlighting, comments,
    /// indentation and LSP follow.
    pub fn set_syntax_at(&mut self, i: usize, name: &str) {
        let lang = self.language_for_syntax(name);
        if lang.is_none() && name != "Plain Text" {
            return;
        }
        let (syntax, grammar) = match &lang {
            Some(l) => (l.name.clone(), self.grammar_of(l)),
            None => ("Plain Text".to_string(), None),
        };
        let b = &mut self.buffers[i];
        b.hl.reset(syntax, grammar);
        b.syntax_override = true;
        let new_lang = lang.as_ref().map(|l| l.name.clone());
        if new_lang != b.lang {
            self.close_lsp_doc(i);
            let b = &mut self.buffers[i];
            b.apply_language(lang.as_ref(), &self.config.editor);
            b.lang = new_lang;
            self.lsp_warned.clear();
            self.lsp_open(i);
        }
    }

    /// The status bar's diagnostic: the one under the caret (inside its range, or just at
    /// its end), else one on the caret's line; the most severe first. (severity,
    /// "error: message" on one line).
    pub fn diagnostic_at_cursor(&self) -> Option<(u8, String)> {
        let b = self.buf()?;
        let ds = self.diagnostics.get(b.path.as_ref()?)?;
        let at = b.cursor;
        let on_line = |d: &&Diag| d.start.line <= at.line && d.end.line >= at.line;
        let under = |d: &&Diag| on_line(d) && d.start <= at && at <= d.end;
        let d = ds
            .iter()
            .filter(under)
            .min_by_key(|d| d.severity)
            .or_else(|| ds.iter().filter(on_line).min_by_key(|d| d.severity))?;
        let label = match d.severity {
            1 => "error",
            2 => "warning",
            3 => "info",
            _ => "hint",
        };
        let text = d.message.split_whitespace().collect::<Vec<_>>().join(" ");
        Some((d.severity, format!("{label}: {text}")))
    }

    /// Help › Licenses and Attribution: the page build.rs generates (every crate and piece
    /// of bundled data in this build, with the license texts), in a read-only Markdown tab.
    /// Opening it again goes to the tab already open.
    pub fn show_licenses(&mut self) {
        const TITLE: &str = "Licenses and Attribution";
        if let Some(i) = self
            .buffers
            .iter()
            .position(|b| b.path.is_none() && b.title.as_deref() == Some(TITLE))
        {
            self.set_cur(i);
            self.focus = Focus::Editor;
            return;
        }
        let id = self.next_buf_id;
        self.next_buf_id += 1;
        let mut b = Buffer::from_text(id, None, LICENSES, HlCache::new(String::new()));
        b.title = Some(TITLE.into());
        b.read_only = true;
        let i = self.push_buffer(b);
        // Markdown highlighting, without a language server for this file-less tab.
        if let Some(l) = self.language_for_syntax("markdown") {
            let grammar = self.grammar_of(&l);
            let b = &mut self.buffers[i];
            b.hl.reset(l.name.clone(), grammar);
            b.apply_language(Some(&l), &self.config.editor);
        }
        self.set_cur(i);
        self.focus = Focus::Editor;
    }

    /// Everything LSP Info shows, as styled lines (kept here so it's testable).
    pub fn lsp_info(&self) -> Vec<(Tone, String)> {
        use Tone::*;
        let mut out: Vec<(Tone, String)> = vec![];
        let row = |k: &str, v: String| format!("  {k:<14}{v}");
        let b = self.buf();
        out.push((Heading, "This file".into()));
        let Some(b) = b else {
            out.push((Dim, "  No file open.".into()));
            return self.lsp_info_running(out);
        };
        out.push((
            Text,
            row(
                "file",
                b.path
                    .as_ref()
                    .map(|p| self.rel(p))
                    .unwrap_or_else(|| "untitled".into()),
            ),
        ));
        out.push((
            Text,
            row(
                "syntax",
                format!(
                    "{}{}",
                    self.language_title(&b.hl.syntax),
                    if b.syntax_override {
                        "  (set by hand)"
                    } else {
                        ""
                    }
                ),
            ),
        ));
        let lang = b.lang.as_ref().and_then(|l| self.config.language(l));
        match lang {
            None => {
                out.push((
                    Warn,
                    row(
                        "language",
                        "none — no language config matches this file".into(),
                    ),
                ));
                out.push((
                    Dim,
                    "  Pick a syntax (click it in the status bar) or add languages/<name>.toml."
                        .into(),
                ));
            }
            Some(l) => {
                out.push((
                    Text,
                    row(
                        "language",
                        format!("{}  (languageId \"{}\")", l.name, l.language_id()),
                    ),
                ));
                if l.servers.is_empty() {
                    out.push((
                        Dim,
                        "  No language servers are configured for this language.".into(),
                    ));
                } else {
                    out.push((Text, String::new()));
                    out.push((
                        Heading,
                        "Servers for this language (first available is used)".into(),
                    ));
                    let attached = b.lsp_client.and_then(|c| self.lsp.clients.get(&c));
                    for name in &l.servers {
                        let Some(s) = self.config.server(name) else {
                            out.push((
                                Bad,
                                row(name, "not defined (no servers/<name>.toml)".into()),
                            ));
                            continue;
                        };
                        let running = self
                            .lsp
                            .clients
                            .values()
                            .any(|c| c.alive && c.server == *name);
                        let (tone, what) = if attached.is_some_and(|c| c.server == *name && c.alive)
                        {
                            (Good, "running — attached to this file".to_string())
                        } else if running {
                            (Good, "running (another project root)".to_string())
                        } else {
                            match self.server_status(s) {
                                ServerStatus::Installed(p) => {
                                    (Text, format!("installed  {}", p.display()))
                                }
                                ServerStatus::OnPath(p) => {
                                    (Text, format!("on PATH  {}", p.display()))
                                }
                                ServerStatus::Installing => (Warn, "installing…".into()),
                                ServerStatus::Failed(e) => (Bad, format!("install failed: {e}")),
                                ServerStatus::Missing => match &s.install {
                                    Some(i) => (
                                        Warn,
                                        format!(
                                            "not installed — press i to install ({})",
                                            i.source()
                                        ),
                                    ),
                                    None => (
                                        Bad,
                                        format!("not found — install `{}` yourself", s.command),
                                    ),
                                },
                            }
                        };
                        out.push((tone, row(name, what)));
                    }
                }
            }
        }
        if let Some(c) = b.lsp_client.and_then(|c| self.lsp.clients.get(&c)) {
            out.push((Text, String::new()));
            out.push((Heading, format!("Attached server: {}", c.server)));
            let state = if !c.alive {
                (Bad, "exited".to_string())
            } else if !c.ready {
                (Warn, "starting…".to_string())
            } else {
                (Good, "ready".to_string())
            };
            let up = c.started.elapsed().as_secs();
            out.push((
                state.0,
                row(
                    "status",
                    format!(
                        "{}  · pid {} · up {}m{:02}s",
                        state.1,
                        c.pid,
                        up / 60,
                        up % 60
                    ),
                ),
            ));
            out.push((Text, row("command", c.path.display().to_string())));
            out.push((Text, row("root", c.root.display().to_string())));
            if let Some(p) = &c.progress {
                out.push((Warn, row("progress", p.clone())));
            }
            out.push((
                Text,
                row(
                    "sync",
                    if c.sync_kind == 2 {
                        "incremental".into()
                    } else {
                        "full document".into()
                    },
                ),
            ));
            let caps: Vec<&str> = [
                ("completionProvider", "completion"),
                ("hoverProvider", "hover"),
                ("definitionProvider", "definition"),
                ("referencesProvider", "references"),
                ("renameProvider", "rename"),
                ("documentFormattingProvider", "formatting"),
                ("documentSymbolProvider", "symbols"),
                ("workspaceSymbolProvider", "project symbols"),
                ("codeActionProvider", "code actions"),
                ("signatureHelpProvider", "signature help"),
            ]
            .iter()
            .filter(|(k, _)| c.has_cap(k))
            .map(|(_, v)| *v)
            .collect();
            out.push((
                Text,
                row(
                    "capabilities",
                    if caps.is_empty() {
                        "(waiting for initialize)".into()
                    } else {
                        caps.join(", ")
                    },
                ),
            ));
            if let Some(ds) = b.path.as_ref().and_then(|p| self.diagnostics.get(p)) {
                let e = ds.iter().filter(|d| d.severity == 1).count();
                let w = ds.iter().filter(|d| d.severity == 2).count();
                let pl = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
                out.push((
                    Text,
                    row(
                        "diagnostics",
                        format!(
                            "{}, {}, {} other in this file",
                            pl(e, "error"),
                            pl(w, "warning"),
                            ds.len() - e - w
                        ),
                    ),
                ));
            }
            let err = c.stderr.lock().unwrap();
            if !err.is_empty() {
                out.push((Text, String::new()));
                out.push((Heading, "Server output (stderr, newest last)".into()));
                for l in err
                    .iter()
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                {
                    out.push((Dim, format!("  {}", l.replace('\t', "    "))));
                }
            }
        }
        self.lsp_info_running(out)
    }

    pub(super) fn lsp_info_running(&self, mut out: Vec<(Tone, String)>) -> Vec<(Tone, String)> {
        out.push((Tone::Text, String::new()));
        out.push((Tone::Heading, "All running servers".into()));
        let mut cs: Vec<_> = self.lsp.clients.iter().collect();
        cs.sort_by_key(|(id, _)| **id);
        if cs.is_empty() {
            out.push((Tone::Dim, "  none".into()));
        }
        for (id, c) in cs {
            let files = self
                .buffers
                .iter()
                .filter(|b| b.lsp_client == Some(*id))
                .count();
            let state = if !c.alive {
                "exited"
            } else if !c.ready {
                "starting"
            } else {
                "ready"
            };
            let tone = if c.alive { Tone::Text } else { Tone::Bad };
            out.push((
                tone,
                format!(
                    "  {:<14}{:<9}{} file(s)   {}",
                    c.server,
                    state,
                    files,
                    c.root.display()
                ),
            ));
        }
        out
    }

    /// The first configured server for this file that isn't installed but could be.
    pub(super) fn installable_for_current(&self) -> Option<String> {
        let lang = self
            .buf()?
            .lang
            .as_ref()
            .and_then(|l| self.config.language(l))?;
        lang.servers
            .iter()
            .find(|n| {
                self.config.server(n).is_some_and(|s| {
                    s.install.as_ref().is_some_and(|i| i.supported_here())
                        && matches!(
                            self.server_status(s),
                            ServerStatus::Missing | ServerStatus::Failed(_)
                        )
                })
            })
            .cloned()
    }

    pub(super) fn lsp_info_key(&mut self, k: Key, mut v: LspInfoView) {
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Up => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::Down => v.scroll += 1,
            KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(15),
            KeyCode::PageDown => v.scroll += 15,
            KeyCode::Home => v.scroll = 0,
            _ => {}
        }
        self.modal = Some(Modal::LspInfo(v));
    }

    pub(super) fn lsp_info_char(&mut self, c: char) {
        match c {
            'i' => match self.installable_for_current() {
                Some(n) => self.install_server(&n),
                None => self.msg("Nothing to install for this file"),
            },
            'r' => {
                self.restart_lsp();
                self.msg("Language servers restarted");
            }
            's' => self.run(Command::ManageServers),
            'q' => self.modal = None,
            _ => {}
        }
    }
}

/// The licenses page, generated by build.rs (build/licenses.rs).
const LICENSES: &str = include_str!(concat!(env!("OUT_DIR"), "/licenses.md"));
