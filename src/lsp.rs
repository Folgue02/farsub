//! Minimal LSP client: JSON-RPC over stdio, one server per (language, project root).
//! Reader/writer threads keep the UI thread from ever blocking on a server.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;

use serde_json::{Value, json};

use crate::app::{AppEvent, EventTx};
use crate::config::{LanguageConfig, ServerConfig};
use crate::editor::{Pos, char_len};

#[derive(Clone, Debug)]
pub enum Pending {
    Initialize,
    Completion { buf: usize, pos: Pos },
    Hover { buf: usize, pos: Pos, mouse: bool },
    Definition,
    References,
    DocSymbols { buf: usize },
    WorkspaceSymbols,
    Rename,
    /// `save`: the file is saved once the edits are in (format on save).
    Format { buf: usize, save: bool },
    CodeActions { buf: usize },
    ResolveAction { buf: usize },
    Other,
}

pub struct Client {
    #[allow(dead_code)]
    pub lang: String,
    /// Server config name (e.g. "pyright").
    pub server: String,
    pub cmd: String,
    pub root: PathBuf,
    writer: Sender<String>,
    child: Child,
    next_id: i64,
    pub pending: HashMap<i64, Pending>,
    pub ready: bool,
    pub alive: bool,
    queue: Vec<String>,
    pub caps: Value,
    pub progress: Option<String>,
    /// textDocumentSync kind: 1 = full, 2 = incremental.
    pub sync_kind: u8,
    pub triggers: Vec<String>,
    /// Full path of the executable, process id, start time, recent stderr lines.
    pub path: PathBuf,
    pub pid: u32,
    pub started: std::time::Instant,
    pub stderr: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
    /// Why the server failed to initialize, if it did.
    pub init_error: Option<String>,
    /// Answers for `workspace/configuration`, keyed by section.
    pub settings: Value,
}

impl Client {
    fn spawn(
        id: usize,
        lang: &LanguageConfig,
        server: &ServerConfig,
        cmd: &Path,
        root: &Path,
        install_dir: Option<&Path>,
        tx: EventTx,
    ) -> io::Result<Client> {
        let server = server.clone();
        let mut vars: Vec<(&str, String)> = vec![
            ("root", root.display().to_string()),
            ("root_id", root_id(root)),
        ];
        if let Some(h) = crate::paths::home_dir() {
            vars.push(("home", h.display().to_string()));
        }
        if let Some(d) = install_dir {
            vars.push(("install_dir", d.display().to_string()));
        }
        let to_json = |v: &Option<toml::Value>| {
            v.as_ref()
                .and_then(|v| serde_json::to_value(v).ok())
                .and_then(|v| substitute(v, &vars))
                .unwrap_or(Value::Null)
        };
        let init_opts = to_json(&server.initialization_options);
        let settings = to_json(&server.settings);
        let args: Vec<String> = server
            .args
            .iter()
            .filter_map(|a| subst_str(a, &vars))
            .collect();
        let env: Vec<(String, String)> = server
            .env
            .iter()
            .filter_map(|(k, v)| subst_str(v, &vars).map(|v| (k.clone(), v)))
            .collect();
        crate::log::info(
            &format!("lsp:{}", server.name),
            format!(
                "starting {} {} in {}",
                cmd.display(),
                args.join(" "),
                root.display()
            ),
        );
        let mut child = Command::new(cmd)
            .args(&args)
            // For launchers that keep data per project (installer.rs `{root_id}`).
            .env("FARSUB_ROOT_ID", root_id(root))
            .envs(env)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Keep the last stderr lines for LSP Info (servers often explain failures there).
        let stderr_buf =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        if let Some(err) = child.stderr.take() {
            let buf = stderr_buf.clone();
            let source = format!("lsp:{}", server.name);
            thread::spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    crate::log::debug(&source, &line);
                    let mut b = buf.lock().unwrap();
                    b.push_back(line);
                    if b.len() > 200 {
                        b.pop_front();
                    }
                }
            });
        }
        let pid = child.id();
        let mut stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (wtx, wrx) = mpsc::channel::<String>();
        // Set FARSUB_LSP_LOG=/path/to/file to trace all LSP traffic.
        let log = std::env::var_os("FARSUB_LSP_LOG").and_then(|p| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .ok()
        });
        let log = std::sync::Arc::new(std::sync::Mutex::new(log));
        let log_r = log.clone();
        thread::spawn(move || {
            for body in wrx {
                if let Some(f) = log.lock().unwrap().as_mut() {
                    let _ = writeln!(f, "--> {body}");
                }
                let msg = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
                if stdin.write_all(msg.as_bytes()).is_err() || stdin.flush().is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                match read_message(&mut r) {
                    Ok(Some(v)) => {
                        if let Some(f) = log_r.lock().unwrap().as_mut() {
                            let _ = writeln!(f, "<-- {v}");
                        }
                        if !tx.send(AppEvent::Lsp(id, v)) {
                            return;
                        }
                    }
                    _ => {
                        let _ = tx.send(AppEvent::LspExit(id));
                        return;
                    }
                }
            }
        });
        let mut c = Client {
            lang: lang.name.clone(),
            server: server.name.clone(),
            cmd: cmd
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            root: root.to_path_buf(),
            writer: wtx,
            child,
            next_id: 1,
            pending: HashMap::new(),
            ready: false,
            alive: true,
            queue: vec![],
            caps: Value::Null,
            progress: None,
            sync_kind: 1,
            triggers: vec![],
            settings,
            path: cmd.to_path_buf(),
            pid,
            started: std::time::Instant::now(),
            stderr: stderr_buf,
            init_error: None,
        };
        let mut params = initialize_params(root);
        if !init_opts.is_null() {
            params["initializationOptions"] = init_opts;
        }
        c.request("initialize", params, Pending::Initialize);
        Ok(c)
    }

    fn raw_send(&mut self, v: Value, force: bool) {
        let s = v.to_string();
        if self.ready || force {
            let _ = self.writer.send(s);
        } else {
            self.queue.push(s);
        }
    }

    pub fn request(&mut self, method: &str, params: Value, p: Pending) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, p);
        let force = method == "initialize";
        self.raw_send(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            force,
        );
        id
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.raw_send(
            json!({"jsonrpc":"2.0","method":method,"params":params}),
            false,
        );
    }

    pub fn respond(&mut self, id: Value, result: Value) {
        self.raw_send(json!({"jsonrpc":"2.0","id":id,"result":result}), true);
    }

    pub fn on_initialized(&mut self, caps: Value) {
        self.caps = caps;
        self.ready = true;
        let sync = self.caps.get("textDocumentSync");
        self.sync_kind = sync
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.get("change").and_then(|c| c.as_u64()))
            })
            .unwrap_or(1) as u8;
        self.triggers = self.trigger_chars();
        self.raw_send(
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
            true,
        );
        for s in std::mem::take(&mut self.queue) {
            let _ = self.writer.send(s);
        }
    }

    /// Value for a `workspace/configuration` section like "rust-analyzer" or "python.analysis".
    pub fn setting(&self, section: Option<&str>) -> Value {
        let Some(section) = section else {
            return self.settings.clone();
        };
        let mut v = &self.settings;
        for part in section.split('.') {
            match v.get(part) {
                Some(x) => v = x,
                None => return Value::Null,
            }
        }
        v.clone()
    }

    pub fn has_cap(&self, name: &str) -> bool {
        match self.caps.get(name) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(_) => true,
        }
    }

    pub fn trigger_chars(&self) -> Vec<String> {
        self.caps
            .pointer("/completionProvider/triggerCharacters")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The process's exit code, if it has exited.
    pub fn exit_code(&mut self) -> Option<i32> {
        self.child.try_wait().ok().flatten().and_then(|s| s.code())
    }

    pub fn shutdown(&mut self) {
        if self.alive {
            self.request("shutdown", Value::Null, Pending::Other);
            self.notify("exit", Value::Null);
            self.alive = false;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let t = line.trim_end();
        if t.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some(v) = t.strip_prefix("Content-Length:") {
            len = v.trim().parse().ok();
        }
    }
    let mut buf = vec![0u8; len.unwrap_or(0)];
    r.read_exact(&mut buf)?;
    Ok(Some(serde_json::from_slice(&buf).unwrap_or(Value::Null)))
}

fn initialize_params(root: &Path) -> Value {
    let uri = path_to_uri(root);
    let name = root.file_name().unwrap_or_default().to_string_lossy();
    json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "farsub", "version": env!("CARGO_PKG_VERSION")},
        "rootUri": uri,
        "rootPath": root.to_string_lossy(),
        "workspaceFolders": [{"uri": uri, "name": name}],
        "capabilities": {
            "general": {"positionEncodings": ["utf-16"]},
            "workspace": {
                "configuration": true,
                "workspaceFolders": true,
                "applyEdit": true,
                "workspaceEdit": {"documentChanges": true},
                "symbol": {},
                "executeCommand": {}
            },
            "textDocument": {
                "synchronization": {"didSave": true},
                "completion": {
                    "completionItem": {"snippetSupport": false, "documentationFormat": ["plaintext", "markdown"]},
                    "contextSupport": true
                },
                "hover": {"contentFormat": ["plaintext", "markdown"]},
                "definition": {"linkSupport": true},
                "references": {},
                "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
                "rename": {},
                "formatting": {},
                "codeAction": {
                    "codeActionLiteralSupport": {"codeActionKind": {"valueSet": [
                        "", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                        "refactor.rewrite", "source", "source.organizeImports", "source.fixAll"
                    ]}},
                    "isPreferredSupport": true,
                    "disabledSupport": true,
                    "dataSupport": true,
                    "resolveSupport": {"properties": ["edit"]}
                },
                "publishDiagnostics": {}
            },
            "window": {"workDoneProgress": true}
        }
    })
}

// ---------- helpers ----------

/// Replace `${name}` placeholders; `None` if one can't be resolved (the value is dropped).
fn subst_str(s: &str, vars: &[(&str, String)]) -> Option<String> {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    (!out.contains("${")).then_some(out)
}

/// `subst_str` over a JSON tree; unresolvable strings drop their key / array item.
fn substitute(v: Value, vars: &[(&str, String)]) -> Option<Value> {
    Some(match v {
        Value::String(s) => Value::String(subst_str(&s, vars)?),
        Value::Array(a) => {
            Value::Array(a.into_iter().filter_map(|x| substitute(x, vars)).collect())
        }
        Value::Object(o) => Value::Object(
            o.into_iter()
                .filter_map(|(k, x)| substitute(x, vars).map(|x| (k, x)))
                .filter(|(_, x)| !matches!(x, Value::Object(m) if m.is_empty()))
                .collect(),
        ),
        other => other,
    })
}

#[cfg(test)]
mod subst_tests {
    use super::*;

    #[test]
    fn placeholders() {
        let vars = [("install_dir", "/opt/x".to_string())];
        let v = serde_json::json!({"hostInfo": "farsub", "tsserver": {"fallbackPath": "${install_dir}/lib"}, "list": ["a", "${nope}"]});
        assert_eq!(
            substitute(v.clone(), &vars).unwrap(),
            serde_json::json!({"hostInfo": "farsub", "tsserver": {"fallbackPath": "/opt/x/lib"}, "list": ["a"]})
        );
        // Without an install folder the setting disappears instead of sending "${install_dir}".
        assert_eq!(
            substitute(v, &[]).unwrap(),
            serde_json::json!({"hostInfo": "farsub", "list": ["a"]})
        );
    }
}

pub fn path_to_uri(p: &Path) -> String {
    url::Url::from_file_path(p)
        .map(|u| u.to_string())
        .unwrap_or_default()
}

pub fn uri_to_path(s: &str) -> Option<PathBuf> {
    url::Url::parse(s).ok()?.to_file_path().ok()
}

pub fn to_lsp(line: &str, p: Pos) -> Value {
    let ch: usize = line.chars().take(p.col).map(|c| c.len_utf16()).sum();
    json!({"line": p.line, "character": ch})
}

pub fn from_lsp(lines: &[String], line: u32, character: u32) -> Pos {
    let l = (line as usize).min(lines.len().saturating_sub(1));
    let Some(s) = lines.get(l) else {
        return Pos::new(0, 0);
    };
    let mut u = 0usize;
    let mut col = 0usize;
    for c in s.chars() {
        if u >= character as usize {
            break;
        }
        u += c.len_utf16();
        col += 1;
    }
    if line as usize >= lines.len() {
        return Pos::new(l, char_len(s));
    }
    Pos::new(l, col)
}

fn which(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if p.is_absolute() || cmd.contains('/') || cmd.contains('\\') {
        return p.is_file().then(|| p.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    // GUI apps launched from Finder/Dock get a minimal PATH; check the usual places too.
    if let Some(home) = crate::paths::home_dir() {
        for d in [
            ".cargo/bin",
            ".local/bin",
            "go/bin",
            ".npm-global/bin",
            ".bun/bin",
            ".local/share/mise/shims",
            ".asdf/shims",
            ".dotnet/tools",
            "scoop/shims",
        ] {
            dirs.push(home.join(d));
        }
    }
    if cfg!(windows) {
        // npm's global folder; Program Files installs usually add themselves to PATH.
        if let Some(a) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(a).join("npm"));
        }
    } else {
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    // Windows runs `npm` as npm.cmd, `java` as java.exe…: try the executable extensions.
    let names: Vec<String> = if cfg!(windows) && p.extension().is_none() {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        exts.split(';')
            .filter(|e| !e.is_empty())
            .map(|e| format!("{cmd}{}", e.to_ascii_lowercase()))
            .collect()
    } else {
        vec![cmd.to_string()]
    };
    dirs.into_iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// A short id for a project folder (servers that keep per-project data, like jdtls).
pub fn root_id(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Recent lookups: screens like the server manager ask about every server each frame, and
/// a miss on macOS costs an `xcrun` process (~5 ms each).
static FIND_CACHE: std::sync::Mutex<
    Option<HashMap<String, (Option<PathBuf>, std::time::Instant)>>,
> = std::sync::Mutex::new(None);
const FIND_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// Forget cached lookups (after installing or uninstalling something).
pub fn clear_find_cache() {
    if let Ok(mut c) = FIND_CACHE.lock() {
        *c = None;
    }
}

/// An executable on PATH (or in the Xcode toolchain on macOS), cached for a few seconds.
pub fn find_server(cmd: &str) -> Option<PathBuf> {
    if let Ok(c) = FIND_CACHE.lock()
        && let Some((hit, at)) = c.as_ref().and_then(|m| m.get(cmd))
        && at.elapsed() < FIND_TTL
    {
        return hit.clone();
    }
    let found = find_server_uncached(cmd);
    if let Ok(mut c) = FIND_CACHE.lock() {
        c.get_or_insert_with(HashMap::new)
            .insert(cmd.to_string(), (found.clone(), std::time::Instant::now()));
    }
    found
}

fn find_server_uncached(cmd: &str) -> Option<PathBuf> {
    if let Some(p) = which(cmd) {
        return Some(p);
    }
    // macOS ships clangd / sourcekit-lsp inside the Xcode toolchain.
    if cfg!(target_os = "macos") {
        if let Ok(out) = Command::new("xcrun")
            .args(["--find", cmd])
            .stderr(Stdio::null())
            .output()
        {
            let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if out.status.success() && p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Nearest ancestor containing one of the language's root markers, else the git root,
/// else the file's directory.
pub fn find_root(file: &Path, markers: &[String]) -> PathBuf {
    let start = if file.is_dir() {
        file
    } else {
        file.parent().unwrap_or(file)
    };
    let mut git: Option<PathBuf> = None;
    // Markers with wildcards ("*.csproj") match any entry of the directory.
    let globs: Vec<globset::GlobMatcher> = markers
        .iter()
        .filter(|m| m.contains(['*', '?', '[']))
        .filter_map(|m| globset::Glob::new(m).ok())
        .map(|g| g.compile_matcher())
        .collect();
    let has_glob_match = |dir: &Path| {
        !globs.is_empty()
            && std::fs::read_dir(dir).is_ok_and(|rd| {
                rd.flatten()
                    .any(|e| globs.iter().any(|g| g.is_match(e.file_name())))
            })
    };
    for dir in start.ancestors() {
        if markers
            .iter()
            .any(|m| !m.contains(['*', '?', '[']) && dir.join(m).exists())
            || has_glob_match(dir)
        {
            return dir.to_path_buf();
        }
        if git.is_none() && dir.join(".git").exists() {
            git = Some(dir.to_path_buf());
        }
    }
    git.unwrap_or_else(|| start.to_path_buf())
}

pub struct Manager {
    pub clients: HashMap<usize, Client>,
    next: usize,
    keys: HashMap<(String, PathBuf), usize>,
    failed: HashSet<String>,
    tx: EventTx,
}

impl Manager {
    pub fn new(tx: EventTx) -> Self {
        Manager {
            clients: HashMap::new(),
            next: 0,
            keys: HashMap::new(),
            failed: HashSet::new(),
            tx,
        }
    }

    /// Get (or lazily start) `server` (already resolved to `cmd`) for a file.
    pub fn client_for(
        &mut self,
        lang: &LanguageConfig,
        server: &ServerConfig,
        cmd: &Path,
        install_dir: Option<&Path>,
        file: &Path,
    ) -> Result<usize, String> {
        let mut markers = server.root_markers.clone();
        markers.extend(lang.roots.iter().cloned());
        let root = find_root(file, &markers);
        let key = (server.name.clone(), root.clone());
        if let Some(&i) = self.keys.get(&key) {
            return match self.clients.get(&i) {
                Some(c) if c.alive => Ok(i),
                Some(c) => Err(format!("{} exited", c.cmd)),
                None => Err("server gone".into()),
            };
        }
        if self.failed.contains(&server.name) {
            return Err(format!("{} failed to start", server.name));
        }
        let id = self.next;
        self.next += 1;
        match Client::spawn(id, lang, server, cmd, &root, install_dir, self.tx.clone()) {
            Ok(c) => {
                self.clients.insert(id, c);
                self.keys.insert(key, id);
                Ok(id)
            }
            Err(e) => {
                self.failed.insert(server.name.clone());
                Err(format!("failed to start {}: {e}", cmd.display()))
            }
        }
    }

    /// Forget clients/failures for one server so it is started fresh next time.
    pub fn reset_server(&mut self, name: &str) {
        self.failed.remove(name);
        let ids: Vec<usize> = self
            .keys
            .iter()
            .filter(|(k, _)| k.0 == name)
            .map(|(_, v)| *v)
            .collect();
        self.keys.retain(|k, _| k.0 != name);
        for id in ids {
            if let Some(mut c) = self.clients.remove(&id) {
                c.shutdown();
            }
        }
    }

    pub fn restart(&mut self) {
        self.shutdown();
        self.clients.clear();
        self.keys.clear();
        self.failed.clear();
    }

    pub fn shutdown(&mut self) {
        for c in self.clients.values_mut() {
            c.shutdown();
        }
    }
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    fn root_markers_support_globs() {
        let d = std::env::temp_dir().join(format!("farsub-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("app/src")).unwrap();
        std::fs::write(d.join("app/App.csproj"), "").unwrap();
        std::fs::write(d.join("app/src/Main.cs"), "").unwrap();
        let file = d.join("app/src/Main.cs");
        assert_eq!(find_root(&file, &["*.csproj".into()]), d.join("app"));
        assert_eq!(find_root(&file, &["App.csproj".into()]), d.join("app"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
