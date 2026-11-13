//! Minimal LSP client: JSON-RPC over stdio, one server per (language, project root).
//! Reader/writer threads keep the UI thread from ever blocking on a server.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;

use serde_json::{Value, json};

use crate::app::AppEvent;
use crate::editor::{Pos, char_len};

#[derive(Clone, Debug)]
pub enum Pending {
    Initialize,
    Completion { buf: usize, pos: Pos },
    Hover { buf: usize, pos: Pos },
    Definition,
    References,
    DocSymbols { buf: usize },
    WorkspaceSymbols,
    Rename,
    Format { buf: usize },
    Other,
}

pub struct Client {
    #[allow(dead_code)]
    pub lang: String,
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
}

impl Client {
    fn spawn(
        id: usize,
        lang: &str,
        cmd: &Path,
        args: &[&str],
        root: &Path,
        tx: Sender<AppEvent>,
    ) -> io::Result<Client> {
        let mut child = Command::new(cmd)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
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
                        if tx.send(AppEvent::Lsp(id, v)).is_err() {
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
            lang: lang.to_string(),
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
        };
        let params = initialize_params(root);
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
        self.raw_send(
            json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
            true,
        );
        for s in std::mem::take(&mut self.queue) {
            let _ = self.writer.send(s);
        }
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
                "symbol": {}
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
                "publishDiagnostics": {}
            },
            "window": {"workDoneProgress": true}
        }
    })
}

// ---------- helpers ----------

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

pub fn lang_for_path(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "m" => "objective-c",
        "lua" => "lua",
        "zig" => "zig",
        "swift" => "swift",
        "rb" => "ruby",
        "java" => "java",
        "json" => "json",
        "toml" => "toml",
        "sh" | "bash" | "zsh" => "shellscript",
        "nix" => "nix",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "css" | "scss" => "css",
        "html" | "htm" => "html",
        "yaml" | "yml" => "yaml",
        _ => return None,
    })
}

fn servers(lang: &str) -> &'static [(&'static str, &'static [&'static str])] {
    match lang {
        "rust" => &[("rust-analyzer", &[])],
        "python" => &[
            ("pyright-langserver", &["--stdio"]),
            ("basedpyright-langserver", &["--stdio"]),
            ("pylsp", &[]),
            ("jedi-language-server", &[]),
        ],
        "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => {
            &[("typescript-language-server", &["--stdio"])]
        }
        "go" => &[("gopls", &[])],
        "c" | "cpp" | "objective-c" => &[("clangd", &[])],
        "lua" => &[("lua-language-server", &[])],
        "zig" => &[("zls", &[])],
        "swift" => &[("sourcekit-lsp", &[])],
        "ruby" => &[("ruby-lsp", &[]), ("solargraph", &["stdio"])],
        "java" => &[("jdtls", &[])],
        "json" => &[("vscode-json-language-server", &["--stdio"])],
        "toml" => &[("taplo", &["lsp", "stdio"])],
        "shellscript" => &[("bash-language-server", &["start"])],
        "nix" => &[("nil", &[]), ("nixd", &[])],
        "haskell" => &[("haskell-language-server-wrapper", &["--lsp"])],
        "elixir" => &[("elixir-ls", &[])],
        "css" => &[("vscode-css-language-server", &["--stdio"])],
        "html" => &[("vscode-html-language-server", &["--stdio"])],
        "yaml" => &[("yaml-language-server", &["--stdio"])],
        _ => &[],
    }
}

fn which(cmd: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for d in [".cargo/bin", ".local/bin", "go/bin", ".rustup/toolchains"] {
            dirs.push(home.join(d));
        }
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs.into_iter().map(|d| d.join(cmd)).find(|p| p.is_file())
}

fn find_server(lang: &str) -> Option<(PathBuf, &'static [&'static str])> {
    for (cmd, args) in servers(lang) {
        if let Some(p) = which(cmd) {
            return Some((p, args));
        }
    }
    // macOS ships clangd / sourcekit-lsp inside the Xcode toolchain.
    if cfg!(target_os = "macos") {
        for (cmd, args) in servers(lang) {
            if let Ok(out) = Command::new("xcrun")
                .args(["--find", cmd])
                .stderr(Stdio::null())
                .output()
            {
                let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
                if out.status.success() && p.is_file() {
                    return Some((p, args));
                }
            }
        }
    }
    None
}

const ROOT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "compile_commands.json",
    "CMakeLists.txt",
    "Package.swift",
    "build.zig",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "flake.nix",
    "mix.exs",
    "tsconfig.json",
];

pub fn find_root(file: &Path) -> PathBuf {
    let start = if file.is_dir() {
        file
    } else {
        file.parent().unwrap_or(file)
    };
    let mut marker_hit: Option<PathBuf> = None;
    for dir in start.ancestors() {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        if marker_hit.is_none() && ROOT_MARKERS.iter().any(|m| dir.join(m).exists()) {
            marker_hit = Some(dir.to_path_buf());
        }
    }
    marker_hit.unwrap_or_else(|| start.to_path_buf())
}

pub struct Manager {
    pub clients: HashMap<usize, Client>,
    next: usize,
    keys: HashMap<(String, PathBuf), usize>,
    failed: HashSet<String>,
    tx: Sender<AppEvent>,
}

impl Manager {
    pub fn new(tx: Sender<AppEvent>) -> Self {
        Manager {
            clients: HashMap::new(),
            next: 0,
            keys: HashMap::new(),
            failed: HashSet::new(),
            tx,
        }
    }

    /// Get (or lazily start) the server for a file. Returns `Err` with a reason if unavailable.
    pub fn client_for(&mut self, lang: &str, file: &Path) -> Result<usize, String> {
        let root = find_root(file);
        let key = (lang.to_string(), root.clone());
        if let Some(&i) = self.keys.get(&key) {
            return match self.clients.get(&i) {
                Some(c) if c.alive => Ok(i),
                Some(c) => Err(format!("{} exited", c.cmd)),
                None => Err("server gone".into()),
            };
        }
        if self.failed.contains(lang) {
            return Err(format!("no language server for {lang}"));
        }
        let Some((cmd, args)) = find_server(lang) else {
            self.failed.insert(lang.to_string());
            let names: Vec<&str> = servers(lang).iter().map(|s| s.0).collect();
            return Err(if names.is_empty() {
                format!("no language server configured for {lang}")
            } else {
                format!("install {} for LSP support", names.join(" or "))
            });
        };
        let id = self.next;
        self.next += 1;
        match Client::spawn(id, lang, &cmd, args, &root, self.tx.clone()) {
            Ok(c) => {
                self.clients.insert(id, c);
                self.keys.insert(key, id);
                Ok(id)
            }
            Err(e) => {
                self.failed.insert(lang.to_string());
                Err(format!("failed to start {}: {e}", cmd.display()))
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
