//! App: LSP plumbing.

use super::*;

impl App {
    // ===================================================================== LSP plumbing

    /// The first configured server for a language that is installed or on PATH.
    pub(super) fn pick_server(
        &self,
        lang: &crate::config::LanguageConfig,
    ) -> Option<(crate::config::ServerConfig, PathBuf)> {
        let data = self.config.data_dir.as_deref();
        lang.servers
            .iter()
            .filter_map(|n| self.config.server(n))
            .find_map(|s| installer::resolve(data, s).map(|bin| (s.clone(), bin)))
    }

    pub(super) fn lsp_open(&mut self, i: usize) {
        let b = &self.buffers[i];
        let (Some(path), Some(lang)) = (b.path.clone(), b.lang.clone()) else {
            return;
        };
        let Some(cfg) = self.config.language(&lang).cloned() else {
            return;
        };
        if cfg.servers.is_empty() {
            return;
        }
        let Some((server, bin)) = self.pick_server(&cfg) else {
            if self.lsp_warned.insert(lang.clone()) {
                let installable: Vec<String> = cfg
                    .servers
                    .iter()
                    .filter(|n| {
                        self.config
                            .server(n)
                            .is_some_and(|s| s.install.as_ref().is_some_and(|i| i.supported_here()))
                    })
                    .cloned()
                    .collect();
                if installable.is_empty() {
                    let cmds: Vec<String> = cfg
                        .servers
                        .iter()
                        .filter_map(|n| self.config.server(n))
                        .map(|s| s.command.clone())
                        .collect();
                    self.msg(format!(
                        "No {lang} language server found (install {})",
                        cmds.join(" or ")
                    ));
                } else if self.config.editor.auto_install
                    && self.config.data_dir.is_some()
                    && !self.install_failed.contains_key(&installable[0])
                {
                    // Frictionless: install the first installable server; the file
                    // attaches to it when it's ready.
                    let name = installable[0].clone();
                    crate::log::info(
                        "lsp",
                        format!("{lang} needs a language server: installing {name}"),
                    );
                    self.install_server(&name);
                } else {
                    self.msg(format!(
                        "No {lang} language server installed — Tools › Language Servers can install {}",
                        installable.join(", ")
                    ));
                }
            }
            return;
        };
        // Managed installs expose their folder as ${install_dir} to the server config.
        let install_dir = self
            .config
            .data_dir
            .as_deref()
            .filter(|d| installer::installed_bin(d, &server).as_ref() == Some(&bin))
            .map(|d| installer::server_dir(d, &server.name));
        match self
            .lsp
            .client_for(&cfg, &server, &bin, install_dir.as_deref(), &path)
        {
            Ok(cid) => {
                let b = &mut self.buffers[i];
                b.lsp_client = Some(cid);
                b.lsp_dirty = false;
                b.lsp_changes.clear();
                b.lsp_full = false;
                let text = b.text();
                let version = b.version;
                if let Some(c) = self.lsp.clients.get_mut(&cid) {
                    c.notify(
                        "textDocument/didOpen",
                        json!({"textDocument": {"uri": path_to_uri(&path), "languageId": cfg.language_id(),
                               "version": version, "text": text}}),
                    );
                }
            }
            Err(e) => {
                if self.lsp_warned.insert(lang) {
                    self.msg(format!("LSP: {e}"));
                }
            }
        }
    }

    /// Push full-document changes for every buffer edited since the last sync.
    pub fn sync_lsp(&mut self) {
        for b in &mut self.buffers {
            if !b.lsp_dirty {
                continue;
            }
            b.lsp_dirty = false;
            let changes = std::mem::take(&mut b.lsp_changes);
            let full = std::mem::replace(&mut b.lsp_full, false);
            let (Some(cid), Some(path)) = (b.lsp_client, &b.path) else {
                continue;
            };
            let Some(c) = self.lsp.clients.get_mut(&cid) else {
                continue;
            };
            // Incremental sync sends only the edited ranges instead of the whole file.
            let content: Vec<Value> = if c.ready && c.sync_kind == 2 && !full {
                changes
                    .into_iter()
                    .map(|(a, z, text)| {
                        json!({"range": {"start": {"line": a.line, "character": a.col},
                                         "end": {"line": z.line, "character": z.col}},
                               "text": text})
                    })
                    .collect()
            } else {
                vec![json!({"text": b.text()})]
            };
            c.notify(
                "textDocument/didChange",
                json!({"textDocument": {"uri": path_to_uri(path), "version": b.version},
                       "contentChanges": content}),
            );
        }
    }

    pub(super) fn lsp_position_request(
        &mut self,
        method: &str,
        extra: Value,
        p: Pending,
        cap: &str,
    ) -> bool {
        let Some(at) = self.buf().map(|b| b.cursor) else {
            return false;
        };
        self.lsp_request_at(method, extra, p, cap, at, true)
    }

    /// Like `lsp_position_request` at an explicit position; `loud` reports missing servers.
    pub(super) fn lsp_request_at(
        &mut self,
        method: &str,
        extra: Value,
        p: Pending,
        cap: &str,
        at: Pos,
        loud: bool,
    ) -> bool {
        self.sync_lsp();
        let Some(b) = self.buf() else { return false };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            if loud {
                self.msg("No language server for this file");
            }
            return false;
        };
        let at = b.clamp(at);
        let pos = to_lsp(&b.lines[at.line], at);
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return false;
        };
        if c.ready && !c.has_cap(cap) {
            if loud {
                let name = c.cmd.clone();
                self.msg(format!("{name} does not support {method}"));
            }
            return false;
        }
        let mut params = json!({"textDocument": {"uri": path_to_uri(&path)}, "position": pos});
        if let (Some(o), Some(e)) = (params.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        c.request(method, params, p);
        true
    }

    pub(super) fn request_completion(&mut self) {
        let Some(b) = self.buf() else { return };
        let (id, pos) = (b.id, b.cursor);
        self.lsp_position_request(
            "textDocument/completion",
            json!({"context": {"triggerKind": 1}}),
            Pending::Completion { buf: id, pos },
            "completionProvider",
        );
    }

    pub(super) fn request_doc_symbols(&mut self) -> bool {
        self.sync_lsp();
        let Some(b) = self.buf() else { return false };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            return false;
        };
        let id = b.id;
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return false;
        };
        if c.ready && !c.has_cap("documentSymbolProvider") {
            return false;
        }
        c.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": path_to_uri(&path)}}),
            Pending::DocSymbols { buf: id },
        );
        true
    }

    pub(super) fn request_workspace_symbols(&mut self) {
        let cid = self.buf().and_then(|b| b.lsp_client).or_else(|| {
            self.lsp
                .clients
                .iter()
                .find(|(_, c)| c.alive)
                .map(|(k, _)| *k)
        });
        let Some(query) = self.palette().map(|p| p.needle().to_string()) else {
            return;
        };
        let Some(cid) = cid else {
            if let Some(p) = self.palette() {
                p.title = "Project symbols — no language server running".into();
            }
            return;
        };
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return;
        };
        let id = c.request(
            "workspace/symbol",
            json!({"query": query}),
            Pending::WorkspaceSymbols,
        );
        if let Some(p) = self.palette() {
            p.ws_request = Some(id);
            p.loading = true;
        }
    }

    pub(super) fn on_lsp(&mut self, cid: usize, msg: Value) {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).map(String::from);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (id, method) {
            (Some(id), Some(m)) => self.on_server_request(cid, id, &m, params),
            (None, Some(m)) => self.on_notification(cid, &m, params),
            (Some(id), None) => {
                let Some(c) = self.lsp.clients.get_mut(&cid) else {
                    return;
                };
                let Some(n) = id.as_i64() else { return };
                let Some(pending) = c.pending.remove(&n) else {
                    return;
                };
                if let Some(err) = msg.get("error") {
                    let m = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("error");
                    if matches!(pending, Pending::Initialize) {
                        // The server refused to start: mark it failed instead of "starting…" forever.
                        let m = m.to_string();
                        if let Some(c) = self.lsp.clients.get_mut(&cid) {
                            c.init_error = Some(m.clone());
                            let name = c.server.clone();
                            c.shutdown();
                            crate::log::error(
                                &format!("lsp:{name}"),
                                format!("initialize failed: {m}"),
                            );
                            self.msg(format!("{name} failed to start: {m}"));
                        }
                        return;
                    }
                    crate::log::warn("lsp", format!("request failed: {m}"));
                    if let Pending::Format { buf, save: true } = pending {
                        // Formatting failed: save the file as it is.
                        return self.save_after_format(buf);
                    }
                    if !matches!(
                        pending,
                        Pending::Completion { .. } | Pending::WorkspaceSymbols
                    ) {
                        self.msg(format!("LSP: {m}"));
                    }
                    return;
                }
                let result = msg.get("result").cloned().unwrap_or(Value::Null);
                self.on_response(cid, n, pending, result);
            }
            _ => {}
        }
    }

    pub(super) fn on_server_request(&mut self, cid: usize, id: Value, method: &str, params: Value) {
        let result = match method {
            "workspace/configuration" => {
                let items = params
                    .get("items")
                    .and_then(|i| i.as_array())
                    .cloned()
                    .unwrap_or_default();
                let c = self.lsp.clients.get(&cid);
                Value::Array(
                    items
                        .iter()
                        .map(|it| {
                            let section = it.get("section").and_then(|s| s.as_str());
                            c.map(|c| c.setting(section)).unwrap_or(Value::Null)
                        })
                        .collect(),
                )
            }
            "workspace/applyEdit" => {
                if let Some(edit) = params.get("edit").cloned()
                    && let Ok(e) = serde_json::from_value::<lt::WorkspaceEdit>(edit)
                {
                    self.apply_workspace_edit(e);
                }
                json!({"applied": true})
            }
            "workspace/workspaceFolders" => {
                let root = self
                    .lsp
                    .clients
                    .get(&cid)
                    .map(|c| c.root.clone())
                    .unwrap_or_default();
                json!([{"uri": path_to_uri(&root), "name": root.file_name().unwrap_or_default().to_string_lossy()}])
            }
            _ => Value::Null,
        };
        if let Some(c) = self.lsp.clients.get_mut(&cid) {
            c.respond(id, result);
        }
    }

    pub(super) fn on_notification(&mut self, cid: usize, method: &str, params: Value) {
        match method {
            "textDocument/publishDiagnostics" => {
                let Ok(p) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params) else {
                    return;
                };
                let Some(path) = uri_to_path(p.uri.as_str()) else {
                    return;
                };
                let buf = self.buffers.iter().find(|b| b.path.as_ref() == Some(&path));
                let conv = |l: u32, c: u32| match buf {
                    Some(b) => from_lsp(&b.lines, l, c),
                    None => Pos::new(l as usize, c as usize),
                };
                let ds = p
                    .diagnostics
                    .into_iter()
                    .map(|d| Diag {
                        raw: serde_json::to_value(&d).unwrap_or_default(),
                        start: conv(d.range.start.line, d.range.start.character),
                        end: conv(d.range.end.line, d.range.end.character),
                        severity: d.severity.map(|s| kind_num(&s) as u8).unwrap_or(1),
                        message: match d.source {
                            Some(s) => format!("{s}: {}", d.message),
                            None => d.message,
                        },
                    })
                    .collect();
                self.diagnostics.insert(path, ds);
            }
            "window/showMessage" | "window/logMessage" => {
                if let Some(m) = params.get("message").and_then(|m| m.as_str()) {
                    let t = params.get("type").and_then(|t| t.as_i64()).unwrap_or(3);
                    let src = format!(
                        "lsp:{}",
                        self.lsp
                            .clients
                            .get(&cid)
                            .map(|c| c.server.as_str())
                            .unwrap_or("?")
                    );
                    let level = match t {
                        1 => crate::log::Level::Error,
                        2 => crate::log::Level::Warn,
                        3 => crate::log::Level::Info,
                        _ => crate::log::Level::Debug,
                    };
                    crate::log::push(level, &src, m);
                    if method == "window/showMessage" && t <= 2 {
                        self.status = Some((format!("LSP: {m}"), Instant::now()));
                    }
                }
            }
            "$/progress" => {
                let Some(c) = self.lsp.clients.get_mut(&cid) else {
                    return;
                };
                let v = params.get("value").cloned().unwrap_or(Value::Null);
                match v.get("kind").and_then(|k| k.as_str()) {
                    Some("begin") | Some("report") => {
                        let title = v.get("title").and_then(|t| t.as_str()).map(String::from);
                        let m = v.get("message").and_then(|t| t.as_str()).unwrap_or("");
                        let prev = c.progress.clone().unwrap_or_default();
                        let t = title
                            .unwrap_or_else(|| prev.split(':').next().unwrap_or("").to_string());
                        c.progress = Some(if m.is_empty() { t } else { format!("{t}: {m}") });
                    }
                    Some("end") => c.progress = None,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_response(&mut self, cid: usize, req_id: i64, pending: Pending, result: Value) {
        match pending {
            Pending::Initialize => {
                let caps = result.get("capabilities").cloned().unwrap_or(Value::Null);
                if let Some(c) = self.lsp.clients.get_mut(&cid) {
                    c.on_initialized(caps);
                }
            }
            Pending::Completion { buf, pos } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                if bi != self.cur || self.focus != Focus::Editor || self.modal.is_some() {
                    return;
                }
                let b = &self.buffers[bi];
                if b.cursor.line != pos.line {
                    return;
                }
                let Ok(resp) = serde_json::from_value::<lt::CompletionResponse>(result) else {
                    return;
                };
                let list = match resp {
                    lt::CompletionResponse::Array(a) => a,
                    lt::CompletionResponse::List(l) => l.items,
                };
                let items: Vec<CompItem> = list
                    .into_iter()
                    .take(2000)
                    .map(|it| {
                        let snippet = it.insert_text_format == Some(lt::InsertTextFormat::SNIPPET);
                        let (text, start) = match &it.text_edit {
                            Some(lt::CompletionTextEdit::Edit(e)) => (
                                e.new_text.clone(),
                                Some(from_lsp(
                                    &b.lines,
                                    e.range.start.line,
                                    e.range.start.character,
                                )),
                            ),
                            Some(lt::CompletionTextEdit::InsertAndReplace(e)) => (
                                e.new_text.clone(),
                                Some(from_lsp(
                                    &b.lines,
                                    e.insert.start.line,
                                    e.insert.start.character,
                                )),
                            ),
                            None => (
                                it.insert_text.clone().unwrap_or_else(|| it.label.clone()),
                                None,
                            ),
                        };
                        let insert = if snippet { strip_snippet(&text) } else { text };
                        let additional = it
                            .additional_text_edits
                            .unwrap_or_default()
                            .into_iter()
                            .map(|e| {
                                (
                                    from_lsp(&b.lines, e.range.start.line, e.range.start.character),
                                    from_lsp(&b.lines, e.range.end.line, e.range.end.character),
                                    e.new_text,
                                )
                            })
                            .collect();
                        CompItem {
                            filter: it.filter_text.clone().unwrap_or_else(|| it.label.clone()),
                            sort: it.sort_text.clone().unwrap_or_else(|| it.label.clone()),
                            kind: it.kind.map(|k| completion_kind(kind_num(&k))).unwrap_or(""),
                            detail: it.detail.unwrap_or_default(),
                            label: it.label,
                            insert,
                            edit_start: start,
                            additional,
                        }
                    })
                    .collect();
                if items.is_empty() {
                    return;
                }
                let start = b.word_start(b.cursor);
                self.completion = Some(Completion {
                    buf_id: buf,
                    start,
                    items,
                    shown: vec![],
                    sel: None,
                });
                self.comp_refilter();
            }
            Pending::Hover { buf, pos, mouse } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                if mouse {
                    // Only if the pointer is still on the same word.
                    let still = self.hover_track.as_ref().is_some_and(|t| {
                        t.buf == buf && t.pos >= t.word.0 && pos >= t.word.0 && pos <= t.word.1
                    });
                    if bi != self.cur || !still {
                        return;
                    }
                } else if bi != self.cur || self.buffers[bi].cursor != pos {
                    return;
                }
                let diags = self.diagnostics_at(pos);
                let h = serde_json::from_value::<Option<lt::Hover>>(result)
                    .ok()
                    .flatten();
                let Some(h) = h else {
                    if mouse {
                        if !diags.is_empty() {
                            self.hover = Some(Hover {
                                lines: vec![],
                                diagnostics: diags,
                                anchor: Some(pos),
                            });
                        }
                        return;
                    }
                    return self.msg("No hover info");
                };
                let ms = |m: lt::MarkedString| match m {
                    lt::MarkedString::String(s) => s,
                    lt::MarkedString::LanguageString(l) => l.value,
                };
                let text = match h.contents {
                    lt::HoverContents::Scalar(m) => ms(m),
                    lt::HoverContents::Array(a) => {
                        a.into_iter().map(ms).collect::<Vec<_>>().join("\n---\n")
                    }
                    lt::HoverContents::Markup(m) => m.value,
                };
                let lines = markdown_to_lines(&text);
                if lines.is_empty() && diags.is_empty() {
                    if !mouse {
                        self.msg("No hover info");
                    }
                    return;
                }
                self.hover = Some(Hover {
                    lines,
                    diagnostics: diags,
                    anchor: mouse.then_some(pos),
                });
            }
            Pending::Definition | Pending::References => {
                let locs = parse_locations(result);
                if locs.is_empty() {
                    return self.msg("No results");
                }
                let is_def = matches!(pending, Pending::Definition);
                if is_def && locs.len() == 1 {
                    let (path, l, c) = locs[0].clone();
                    let pos = self.lsp_pos_for(&path, l, c);
                    self.goto_location(&path, Some(pos));
                    return;
                }
                let items: Vec<Item> = locs
                    .into_iter()
                    .map(|(path, l, c)| {
                        let pos = self.lsp_pos_for(&path, l, c);
                        let preview = self.line_text(&path, pos.line);
                        let rel = self.rel_path(&path);
                        Item {
                            label: format!("{rel}:{}", pos.line + 1),
                            detail: preview.trim().chars().take(80).collect(),
                            action: Action::Open(path, Some(pos)),
                        }
                    })
                    .collect();
                let mut p = Palette::new(Mode::Locations, "");
                p.title = format!(
                    "{} {}",
                    items.len(),
                    if is_def { "definitions" } else { "references" }
                );
                p.set_items(items);
                self.modal = Some(Modal::Palette(p));
            }
            Pending::DocSymbols { buf } => {
                let Some(Modal::Palette(p)) = &self.modal else {
                    return;
                };
                if p.mode != Mode::Symbols {
                    return;
                }
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                let Ok(Some(resp)) =
                    serde_json::from_value::<Option<lt::DocumentSymbolResponse>>(result)
                else {
                    if let Some(p) = self.palette() {
                        p.loading = false;
                    }
                    return;
                };
                let lines = &self.buffers[bi].lines;
                let mut items = vec![];
                match resp {
                    lt::DocumentSymbolResponse::Flat(v) => {
                        for s in v {
                            let pos = from_lsp(
                                lines,
                                s.location.range.start.line,
                                s.location.range.start.character,
                            );
                            items.push(Item {
                                label: s.name,
                                detail: format!(
                                    "{} {}",
                                    kind_name(kind_num(&s.kind)),
                                    s.container_name.unwrap_or_default()
                                ),
                                action: Action::Goto(pos),
                            });
                        }
                    }
                    lt::DocumentSymbolResponse::Nested(v) => {
                        fn walk(
                            v: Vec<lt::DocumentSymbol>,
                            depth: usize,
                            lines: &[String],
                            out: &mut Vec<Item>,
                        ) {
                            for s in v {
                                let r = s.selection_range.start;
                                out.push(Item {
                                    label: format!("{}{}", "  ".repeat(depth), s.name),
                                    // Only the name and its kind, like Sublime (no signature).
                                    detail: kind_name(kind_num(&s.kind)).to_string(),
                                    action: Action::Goto(from_lsp(lines, r.line, r.character)),
                                });
                                if let Some(ch) = s.children {
                                    walk(ch, depth + 1, lines, out);
                                }
                            }
                        }
                        walk(v, 0, lines, &mut items);
                    }
                }
                items.sort_by_key(|i| match i.action {
                    Action::Goto(p) => p,
                    _ => Pos::default(),
                });
                if let Some(p) = self.palette() {
                    p.loading = false;
                    p.set_items(items);
                }
                self.palette_preview();
            }
            Pending::WorkspaceSymbols => {
                let Some(p) = self.palette() else { return };
                if p.mode != Mode::WorkspaceSymbols || p.ws_request != Some(req_id) {
                    return;
                }
                let Ok(Some(resp)) =
                    serde_json::from_value::<Option<lt::WorkspaceSymbolResponse>>(result)
                else {
                    p.loading = false;
                    return;
                };
                let mut raw: Vec<(String, String, PathBuf, u32, u32)> = vec![];
                match resp {
                    lt::WorkspaceSymbolResponse::Flat(v) => {
                        for s in v {
                            if let Some(path) = uri_to_path(s.location.uri.as_str()) {
                                let r = s.location.range.start;
                                raw.push((
                                    s.name,
                                    kind_name(kind_num(&s.kind)).into(),
                                    path,
                                    r.line,
                                    r.character,
                                ));
                            }
                        }
                    }
                    lt::WorkspaceSymbolResponse::Nested(v) => {
                        for s in v {
                            let (uri, r) = match s.location {
                                lt::OneOf::Left(l) => (l.uri, l.range.start),
                                lt::OneOf::Right(w) => (w.uri, lt::Position::default()),
                            };
                            if let Some(path) = uri_to_path(uri.as_str()) {
                                raw.push((
                                    s.name,
                                    kind_name(kind_num(&s.kind)).into(),
                                    path,
                                    r.line,
                                    r.character,
                                ));
                            }
                        }
                    }
                }
                let items: Vec<Item> = raw
                    .into_iter()
                    .map(|(name, kind, path, l, c)| {
                        let pos = self.lsp_pos_for(&path, l, c);
                        let rel = self.rel_path(&path);
                        Item {
                            label: name,
                            detail: format!("{kind}  {rel}:{}", pos.line + 1),
                            action: Action::Open(path, Some(pos)),
                        }
                    })
                    .collect();
                if let Some(p) = self.palette() {
                    p.loading = false;
                    p.set_items(items);
                }
            }
            Pending::Rename => match serde_json::from_value::<Option<lt::WorkspaceEdit>>(result) {
                Ok(Some(e)) => {
                    let (n, f) = self.apply_workspace_edit(e);
                    self.msg(format!(
                        "Renamed: {n} edit(s) in {f} file(s) — save to keep"
                    ));
                }
                _ => self.msg("Rename failed"),
            },
            Pending::Format { buf, save } => {
                let Some(bi) = self.buf_index(buf) else {
                    return;
                };
                let Ok(Some(edits)) = serde_json::from_value::<Option<Vec<lt::TextEdit>>>(result)
                else {
                    if save {
                        self.save_after_format(buf);
                    }
                    return;
                };
                let b = &mut self.buffers[bi];
                let es = edits
                    .into_iter()
                    .map(|e| {
                        (
                            from_lsp(&b.lines, e.range.start.line, e.range.start.character),
                            from_lsp(&b.lines, e.range.end.line, e.range.end.character),
                            e.new_text,
                        )
                    })
                    .collect::<Vec<_>>();
                let n = es.len();
                b.apply_edits(es);
                if save {
                    return self.save_after_format(buf);
                }
                self.msg(if n == 0 {
                    "Already formatted".to_string()
                } else {
                    format!("Formatted ({n} edits)")
                });
            }
            Pending::CodeActions { buf } => {
                let mut list: Vec<Value> = result
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|a| a.get("disabled").is_none())
                    .collect();
                // The server's preferred fix first.
                list.sort_by_key(|a| !a.get("isPreferred").and_then(Value::as_bool).unwrap_or(false));
                let mut items: Vec<Item> = list
                    .iter()
                    .enumerate()
                    .map(|(i, a)| Item {
                        label: a
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("(untitled)")
                            .to_string(),
                        detail: a
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        action: Action::CodeAction(i),
                    })
                    .collect();
                self.code_actions = list.into_iter().map(|a| (cid, buf, a)).collect();
                items.extend(self.editor_actions(cid));
                self.show_code_actions(items);
            }
            Pending::ResolveAction { buf } => {
                if result.is_object() {
                    self.apply_code_action(cid, buf, result, false);
                }
            }
            Pending::Other => {}
        }
    }

    /// Ask the server to format the current file; with `save`, it's saved afterwards.
    /// False if there's no server that can format it.
    pub(super) fn request_format(&mut self, save: bool) -> bool {
        self.sync_lsp();
        let Some(b) = self.buf() else { return false };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            return false;
        };
        let id = b.id;
        let tab = b.indent_unit == "\t";
        let size = if tab { 4 } else { b.indent_unit.len() };
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return false;
        };
        if !c.alive || (save && !(c.ready && c.has_cap("documentFormattingProvider"))) {
            return false;
        }
        c.request(
            "textDocument/formatting",
            json!({"textDocument": {"uri": path_to_uri(&path)},
                   "options": {"tabSize": size, "insertSpaces": !tab}}),
            Pending::Format { buf: id, save },
        );
        true
    }

    /// Format on save: the edits are in (or formatting failed), save the file now.
    pub(super) fn save_after_format(&mut self, buf: usize) {
        if !self.format_saving.contains(&buf) {
            return;
        }
        let Some(bi) = self.buf_index(buf) else {
            self.format_saving.remove(&buf);
            return;
        };
        let (cur, group) = (self.cur, self.group);
        self.cur = bi;
        self.group = self.buffers[bi].group;
        self.save_current();
        self.cur = cur;
        self.group = group;
    }

    /// Cmd+.: ask the server what it can do at the cursor (quick fixes for the
    /// diagnostics there, refactorings of the selection).
    pub(super) fn request_code_actions(&mut self) {
        self.sync_lsp();
        let Some(b) = self.buf() else { return };
        let (Some(cid), Some(path)) = (b.lsp_client, b.path.clone()) else {
            return self.msg("No language server for this file");
        };
        let (a, z) = b.selection().unwrap_or((b.cursor, b.cursor));
        let range = json!({"start": to_lsp(&b.lines[a.line], a), "end": to_lsp(&b.lines[z.line], z)});
        let id = b.id;
        let diags: Vec<Value> = self
            .diagnostics
            .get(&path)
            .map(|ds| {
                ds.iter()
                    .filter(|d| d.start.line <= z.line && d.end.line >= a.line)
                    .map(|d| d.raw.clone())
                    .collect()
            })
            .unwrap_or_default();
        let Some(c) = self.lsp.clients.get_mut(&cid) else {
            return;
        };
        if c.ready && !c.has_cap("codeActionProvider") {
            // Nothing to ask: just the editor's own actions.
            self.code_actions.clear();
            let items = self.editor_actions(cid);
            return self.show_code_actions(items);
        }
        c.request(
            "textDocument/codeAction",
            json!({"textDocument": {"uri": path_to_uri(&path)}, "range": range,
                   "context": {"diagnostics": diags, "triggerKind": 1}}),
            Pending::CodeActions { buf: id },
        );
    }

    /// After the server's actions, the editor's own for the symbol at the cursor (the ones
    /// this server can do), like VS Code's context menu: rename, go to definition, …
    fn editor_actions(&self, cid: usize) -> Vec<Item> {
        use crate::commands::Command::*;
        let c = self.lsp.clients.get(&cid);
        [
            (Rename, "renameProvider"),
            (GotoDefinition, "definitionProvider"),
            (FindReferences, "referencesProvider"),
            (Hover, "hoverProvider"),
            (Format, "documentFormattingProvider"),
        ]
        .into_iter()
        // Until the server says what it can do, offer them all.
        .filter(|(_, cap)| c.is_none_or(|c| !c.ready || c.has_cap(cap)))
        .map(|(cmd, _)| Item {
            label: crate::commands::name(cmd)
                .trim_start_matches("LSP: ")
                .to_string(),
            detail: self.keymap.shortcut(cmd),
            action: Action::Command(cmd),
        })
        .collect()
    }

    fn show_code_actions(&mut self, items: Vec<Item>) {
        if items.is_empty() {
            return self.msg("No code actions here");
        }
        let mut p = Palette::new(Mode::CodeActions, "");
        p.title = match self.code_actions.len() {
            0 => "Actions".to_string(),
            1 => "1 code action".to_string(),
            n => format!("{n} code actions"),
        };
        p.set_items(items);
        self.modal = Some(Modal::Palette(p));
    }

    /// The code action picked from the list.
    pub(super) fn run_code_action(&mut self, i: usize) {
        let Some((cid, buf, a)) = self.code_actions.get(i).cloned() else {
            return;
        };
        self.code_actions.clear();
        self.apply_code_action(cid, buf, a, true);
    }

    /// Apply an action's edit and run its command. An action without its edit yet is
    /// resolved first (servers fill it in lazily).
    fn apply_code_action(&mut self, cid: usize, buf: usize, a: Value, may_resolve: bool) {
        let title = a.get("title").and_then(Value::as_str).unwrap_or("").to_string();
        // A bare Command: { title, command: "name", arguments }.
        if a.get("command").is_some_and(Value::is_string) {
            return self.execute_command(cid, &a);
        }
        if a.get("edit").is_none()
            && may_resolve
            && let Some(c) = self.lsp.clients.get_mut(&cid)
            && c.caps
                .pointer("/codeActionProvider/resolveProvider")
                .and_then(Value::as_bool)
                == Some(true)
        {
            c.request("codeAction/resolve", a, Pending::ResolveAction { buf });
            return;
        }
        let mut done = false;
        if let Some(edit) = a.get("edit").cloned()
            && let Ok(e) = serde_json::from_value::<lt::WorkspaceEdit>(edit)
        {
            self.apply_workspace_edit(e);
            done = true;
        }
        if let Some(cmd) = a.get("command").filter(|c| c.is_object()).cloned() {
            self.execute_command(cid, &cmd);
            done = true;
        }
        self.msg(if done {
            title.clone()
        } else {
            format!("{title}: the server sent nothing to apply")
        });
    }

    /// workspace/executeCommand; the server usually answers with workspace/applyEdit.
    fn execute_command(&mut self, cid: usize, cmd: &Value) {
        let Some(name) = cmd.get("command").and_then(Value::as_str) else {
            return;
        };
        let args = cmd.get("arguments").cloned().unwrap_or(json!([]));
        if let Some(c) = self.lsp.clients.get_mut(&cid) {
            c.request(
                "workspace/executeCommand",
                json!({"command": name, "arguments": args}),
                Pending::Other,
            );
        }
    }

    pub(super) fn lsp_pos_for(&self, path: &Path, l: u32, c: u32) -> Pos {
        if let Some(b) = self
            .buffers
            .iter()
            .find(|b| b.path.as_deref() == Some(path))
        {
            return from_lsp(&b.lines, l, c);
        }
        Pos::new(l as usize, c as usize)
    }

    pub(super) fn line_text(&self, path: &Path, line: usize) -> String {
        if let Some(b) = self
            .buffers
            .iter()
            .find(|b| b.path.as_deref() == Some(path))
        {
            return b.lines.get(line).cloned().unwrap_or_default();
        }
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| s.lines().nth(line).map(String::from))
            .unwrap_or_default()
    }

    pub(super) fn rel_path(&self, path: &Path) -> String {
        self.rel(path)
    }

    pub(super) fn apply_workspace_edit(&mut self, edit: lt::WorkspaceEdit) -> (usize, usize) {
        let mut per_file: Vec<(PathBuf, Vec<lt::TextEdit>)> = vec![];
        if let Some(changes) = edit.changes {
            for (uri, edits) in changes {
                if let Some(p) = uri_to_path(uri.as_str()) {
                    per_file.push((p, edits));
                }
            }
        }
        let push_tde = |tde: lt::TextDocumentEdit, out: &mut Vec<(PathBuf, Vec<lt::TextEdit>)>| {
            if let Some(p) = uri_to_path(tde.text_document.uri.as_str()) {
                let edits = tde
                    .edits
                    .into_iter()
                    .map(|e| match e {
                        lt::OneOf::Left(t) => t,
                        lt::OneOf::Right(a) => a.text_edit,
                    })
                    .collect();
                out.push((p, edits));
            }
        };
        match edit.document_changes {
            Some(lt::DocumentChanges::Edits(v)) => {
                for t in v {
                    push_tde(t, &mut per_file);
                }
            }
            Some(lt::DocumentChanges::Operations(ops)) => {
                for op in ops {
                    if let lt::DocumentChangeOperation::Edit(t) = op {
                        push_tde(t, &mut per_file);
                    }
                }
            }
            None => {}
        }
        let (mut n, files) = (0, per_file.len());
        let (cur, focus, group) = (self.cur, self.focus, self.group);
        for (path, edits) in per_file {
            let Some(i) = self.open_file(&path, false) else {
                continue;
            };
            let b = &mut self.buffers[i];
            let es: Vec<_> = edits
                .into_iter()
                .map(|e| {
                    (
                        from_lsp(&b.lines, e.range.start.line, e.range.start.character),
                        from_lsp(&b.lines, e.range.end.line, e.range.end.character),
                        e.new_text,
                    )
                })
                .collect();
            n += es.len();
            b.apply_edits(es);
        }
        self.cur = cur.min(self.buffers.len().saturating_sub(1));
        self.group = group;
        self.focus = focus;
        (n, files)
    }

    pub fn lsp_status(&self) -> String {
        let Some(b) = self.buf() else {
            return String::new();
        };
        match b.lsp_client.and_then(|c| self.lsp.clients.get(&c)) {
            Some(c) if !c.alive => format!("{} (dead)", c.server),
            Some(c) if !c.ready => format!("{} starting…", c.server),
            Some(c) => match &c.progress {
                Some(p) => format!("{} · {}", c.server, p.chars().take(40).collect::<String>()),
                None => c.server.clone(),
            },
            None => String::new(),
        }
    }

    pub(super) fn close_lsp_doc(&mut self, i: usize) {
        let b = &mut self.buffers[i];
        if let (Some(cid), Some(path)) = (b.lsp_client.take(), &b.path)
            && let Some(c) = self.lsp.clients.get_mut(&cid)
        {
            c.notify(
                "textDocument/didClose",
                json!({"textDocument": {"uri": path_to_uri(path)}}),
            );
        }
    }
}
