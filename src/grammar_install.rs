//! Installing tree-sitter grammars at runtime, so only a core set is compiled into farsub.
//!
//! A recipe (`grammars/<name>.toml`, from runtime/, the config folder or a package) names a
//! grammar's source: a crates.io crate (downloaded as its source tarball) or a local folder.
//! Installing compiles its `src/parser.c` (+ `scanner.c` / `scanner.cc`) into a shared
//! library with the system C compiler — like Helix — into `<data>/grammars/<name>/`.
//! [`load`] then loads it; libraries stay loaded for the life of the process.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tree_sitter::Language;

use crate::app::{AppEvent, EventTx};
use crate::installer::InstallEvent;

/// `grammars/<name>.toml`.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub struct GrammarRecipe {
    /// Grammar name (defaults to the file name).
    #[serde(default)]
    pub name: String,
    /// crates.io package holding the grammar source.
    #[serde(default, rename = "crate")]
    pub krate: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// A local folder with the source instead of a crate.
    #[serde(default)]
    pub source: Option<String>,
    /// Folder inside the source that holds `src/parser.c` (default: the root).
    #[serde(default)]
    pub path: Option<String>,
    /// Exported language function (default `tree_sitter_<name>`).
    #[serde(default)]
    pub symbol: Option<String>,
}

impl GrammarRecipe {
    pub fn symbol(&self) -> String {
        self.symbol
            .clone()
            .unwrap_or_else(|| format!("tree_sitter_{}", self.name))
    }

    /// Where it comes from, for the UI.
    pub fn source_desc(&self) -> String {
        match (&self.krate, &self.version, &self.source) {
            (Some(c), Some(v), _) => format!("{c} {v}"),
            (_, _, Some(s)) => s.clone(),
            _ => "?".into(),
        }
    }

    /// File name of the compiled library; includes the version so an update loads as a new
    /// library (loaded ones are never unloaded).
    fn lib_file(&self) -> String {
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else if cfg!(windows) {
            "dll"
        } else {
            "so"
        };
        let v = match &self.version {
            Some(v) => v.clone(),
            // Local builds get a fresh name each time, so a rebuild loads as a new library.
            None => format!(
                "local-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            ),
        };
        format!("{}-{v}.{ext}", self.name)
    }
}

pub fn grammars_dir(data: &Path) -> PathBuf {
    data.join("grammars")
}

/// `<data>/grammars/<name>/grammar.toml`: what an install produced, so installed grammars
/// load without needing their recipe (grammars installed from a file have none).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Installed {
    pub name: String,
    pub symbol: String,
    /// Library file name in the grammar's folder.
    pub library: String,
    /// Where it came from (crate + version, or a path).
    pub source: String,
}

/// Every installed grammar: (record, its library path).
pub fn installed(data: &Path) -> Vec<(Installed, PathBuf)> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(grammars_dir(data)) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let Ok(text) = std::fs::read_to_string(d.join("grammar.toml")) else {
            continue;
        };
        let Ok(rec) = toml::from_str::<Installed>(&text) else {
            continue;
        };
        let lib = d.join(&rec.library);
        if lib.is_file() {
            out.push((rec, lib));
        }
    }
    out
}

/// The installed library for grammar `name`, if any.
pub fn installed_lib(data: &Path, name: &str) -> Option<PathBuf> {
    installed(data)
        .into_iter()
        .find(|(r, _)| r.name == name)
        .map(|(_, p)| p)
}

/// The highlight query that came with an installed grammar's source, if any.
pub fn installed_query(data: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(grammars_dir(data).join(name).join("queries/highlights.scm")).ok()
}

pub fn uninstall(data: &Path, name: &str) -> std::io::Result<()> {
    let d = grammars_dir(data).join(name);
    if d.exists() {
        std::fs::remove_dir_all(d)
    } else {
        Ok(())
    }
}

/// Install in the background; events arrive as `AppEvent::Grammar(name, ..)`.
pub fn install(data: PathBuf, r: GrammarRecipe, tx: EventTx) {
    std::thread::spawn(move || {
        let name = r.name.clone();
        let log = |m: String| {
            tx.send(AppEvent::Grammar(name.clone(), InstallEvent::Log(m)));
        };
        let res = run_install(&data, &r, &log);
        if let Err(e) = &res {
            log(format!("error: {e}"));
        }
        tx.send(AppEvent::Grammar(name.clone(), InstallEvent::Done(res)));
    });
}

/// Fetch the source, compile it and put the library in `<data>/grammars/<name>/`.
pub fn run_install(
    data: &Path,
    r: &GrammarRecipe,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let dir = grammars_dir(data).join(&r.name);
    let work = grammars_dir(data).join(format!(".{}-build", r.name));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let root = match (&r.source, &r.krate, &r.version) {
            (Some(local), _, _) => PathBuf::from(local),
            (None, Some(c), Some(v)) => {
                let url = format!("https://static.crates.io/crates/{c}/{c}-{v}.crate");
                log(format!("downloading {url}"));
                let archive = work.join(format!("{c}-{v}.tar.gz"));
                {
                    let mut reader = crate::installer::http_get(&url)?.into_reader();
                    let mut f = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
                    std::io::copy(&mut reader, &mut f)
                        .map_err(|e| format!("download failed: {e}"))?;
                }
                crate::installer::extract(&archive, &work, "")?;
                work.join(format!("{c}-{v}"))
            }
            _ => return Err("the recipe names no source (crate + version, or source)".into()),
        };
        let base = match &r.path {
            Some(p) => root.join(p),
            None => root.clone(),
        };
        let src = base.join("src");
        if !src.join("parser.c").is_file() {
            return Err(format!("{} has no src/parser.c", base.display()));
        }
        // A reinstall replaces the old files (a library still loaded this session keeps
        // working; on Windows a loaded one can't be deleted, which is fine).
        if dir.exists() {
            for e in std::fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .flatten()
            {
                let p = e.path();
                let _ = if p.is_dir() {
                    std::fs::remove_dir_all(&p)
                } else {
                    std::fs::remove_file(&p)
                };
            }
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let lib = dir.join(r.lib_file());
        log(format!("compiling {}", src.display()));
        compile(&src, &lib)?;
        // Keep the grammar's own highlight query (used when farsub ships none for it).
        for q in [
            base.join("queries/highlights.scm"),
            root.join("queries/highlights.scm"),
        ] {
            if q.is_file() {
                let qd = dir.join("queries");
                std::fs::create_dir_all(&qd).map_err(|e| e.to_string())?;
                std::fs::copy(&q, qd.join("highlights.scm")).map_err(|e| e.to_string())?;
                break;
            }
        }
        // The library must load and be a grammar the parser accepts.
        let lang = load(&lib, &r.symbol())?;
        tree_sitter::Parser::new()
            .set_language(&lang)
            .map_err(|e| format!("incompatible grammar: {e}"))?;
        let rec = Installed {
            name: r.name.clone(),
            symbol: r.symbol(),
            library: lib.file_name().unwrap().to_string_lossy().into_owned(),
            source: r.source_desc(),
        };
        std::fs::write(
            dir.join("grammar.toml"),
            toml::to_string(&rec).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        log(format!("installed {}", lib.display()));
        Ok(lib)
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Compile the grammar in `src` into the shared library `out`.
fn compile(src: &Path, out: &Path) -> Result<(), String> {
    let target = env!("FARSUB_TARGET");
    let cpp = src.join("scanner.cc").is_file();
    let mut build = cc::Build::new();
    build
        .target(target)
        .host(target)
        .opt_level(2)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .cpp(cpp);
    let tool = build.try_get_compiler().map_err(|e| {
        format!(
            "no C compiler found ({e}). Install the Xcode Command Line Tools (macOS), gcc/clang \
             (Linux) or Visual Studio Build Tools (Windows)"
        )
    })?;
    let mut cmd = tool.to_command();
    let parser = src.join("parser.c");
    let scanner_c = src.join("scanner.c");
    let scanner_cc = src.join("scanner.cc");
    if tool.is_like_msvc() {
        cmd.args(["/nologo", "/LD", "/O2", "/utf-8"]);
        cmd.arg(format!("/I{}", src.display()));
        cmd.arg("/Tc").arg(&parser);
        if scanner_c.is_file() {
            cmd.arg("/Tc").arg(&scanner_c);
        }
        if cpp {
            cmd.arg("/Tp").arg(&scanner_cc);
        }
        cmd.arg(format!("/Fe:{}", out.display()));
        cmd.current_dir(out.parent().unwrap_or(src));
    } else {
        cmd.args(["-shared", "-fPIC", "-O2", "-fno-exceptions", "-w"]);
        cmd.arg("-I").arg(src);
        // C files as C even when the tool is a C++ compiler (for scanner.cc).
        cmd.args(["-x", "c"]).arg(&parser);
        if scanner_c.is_file() {
            cmd.arg(&scanner_c);
        }
        if cpp {
            cmd.args(["-x", "c++"]).arg(&scanner_cc);
        }
        cmd.arg("-o").arg(out);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("running the compiler: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = err.lines().rev().take(12).collect();
        return Err(format!(
            "compiling failed:\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    Ok(())
}

/// Install a grammar from a file or folder on this computer: a source folder (or a
/// `.zip` / `.tar.gz` / `.tgz` / `.tar` / `.crate` archive of one) holding `src/parser.c`,
/// possibly in a subfolder. The name comes from its `tree-sitter.json`, else from the
/// language function `parser.c` exports. Returns (name, library).
pub fn install_from_path(
    data: &Path,
    path: &Path,
    log: &dyn Fn(String),
) -> Result<(String, PathBuf), String> {
    let tmp = grammars_dir(data).join(".from-file");
    let _ = std::fs::remove_dir_all(&tmp);
    let result = (|| {
        let root = if path.is_dir() {
            path.to_path_buf()
        } else {
            unpack(path, &tmp)?
        };
        let base = find_grammar_root(&root, 3)
            .ok_or_else(|| format!("no src/parser.c in {}", path.display()))?;
        let (name, symbol) = grammar_identity(&root, &base)?;
        log(format!("found the {name} grammar in {}", base.display()));
        let r = GrammarRecipe {
            name: name.clone(),
            source: Some(base.display().to_string()),
            symbol: Some(symbol),
            ..Default::default()
        };
        // Queries next to the source root (archives often keep them at the top).
        let lib = run_install(data, &r, log)?;
        let qd = grammars_dir(data).join(&name).join("queries");
        if !qd.join("highlights.scm").is_file() && root.join("queries/highlights.scm").is_file() {
            let _ = std::fs::create_dir_all(&qd);
            let _ = std::fs::copy(
                root.join("queries/highlights.scm"),
                qd.join("highlights.scm"),
            );
        }
        // Record where it really came from (the unpacked copy is temporary).
        let rec_path = grammars_dir(data).join(&name).join("grammar.toml");
        if let Ok(text) = std::fs::read_to_string(&rec_path)
            && let Ok(mut rec) = toml::from_str::<Installed>(&text)
        {
            rec.source = path.display().to_string();
            let _ = std::fs::write(&rec_path, toml::to_string(&rec).unwrap_or(text));
        }
        Ok((name, lib))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Unpack an archive into `dir`; returns its single top folder if it has one.
pub fn unpack(archive: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    // A .crate is a .tar.gz.
    let src = if name.ends_with(".crate") {
        let copy = dir.join("source.tar.gz");
        std::fs::copy(archive, &copy).map_err(|e| e.to_string())?;
        copy
    } else if [".zip", ".tar.gz", ".tgz", ".tar"]
        .iter()
        .any(|e| name.ends_with(e))
    {
        archive.to_path_buf()
    } else {
        return Err(format!(
            "{} isn't a folder or a .zip / .tar.gz / .tgz / .tar / .crate archive",
            archive.display()
        ));
    };
    let out = dir.join("unpacked");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    crate::installer::extract(&src, &out, "")?;
    let entries: Vec<PathBuf> = std::fs::read_dir(&out)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    Ok(match entries.as_slice() {
        [one] if one.is_dir() => one.clone(),
        _ => out,
    })
}

/// The folder holding `src/parser.c`: `root` itself or a subfolder (up to `depth` deep).
fn find_grammar_root(root: &Path, depth: usize) -> Option<PathBuf> {
    if root.join("src/parser.c").is_file() {
        return Some(root.to_path_buf());
    }
    if depth == 0 {
        return None;
    }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    subs.sort();
    subs.into_iter()
        .find_map(|s| find_grammar_root(&s, depth - 1))
}

/// (grammar name, exported symbol) for a grammar source.
fn grammar_identity(root: &Path, base: &Path) -> Result<(String, String), String> {
    let parser = std::fs::read_to_string(base.join("src/parser.c")).map_err(|e| e.to_string())?;
    // The language function: `tree_sitter_<name>(void)` that isn't a scanner hook.
    let symbol = parser
        .match_indices("tree_sitter_")
        .filter_map(|(i, _)| {
            let rest = &parser[i..];
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
            rest[end..]
                .starts_with("(void)")
                .then(|| rest[..end].to_string())
        })
        .find(|s| !s.contains("external_scanner"))
        .ok_or("can't find the grammar's language function in src/parser.c")?;
    let from_symbol = symbol.trim_start_matches("tree_sitter_").to_string();
    // tree-sitter.json names it explicitly (prefer the entry whose symbol matches).
    let json_name = [root.join("tree-sitter.json"), base.join("tree-sitter.json")]
        .iter()
        .find_map(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            let gs = v["grammars"].as_array()?.clone();
            gs.iter()
                .filter_map(|g| g["name"].as_str())
                .find(|n| *n == from_symbol)
                .or_else(|| gs.first().and_then(|g| g["name"].as_str()))
                .map(String::from)
        });
    let name = json_name
        .filter(|n| *n == from_symbol)
        .unwrap_or(from_symbol);
    Ok((name, symbol))
}

type Loaded = HashMap<PathBuf, (libloading::Library, Language)>;

fn loaded() -> &'static Mutex<Loaded> {
    static L: OnceLock<Mutex<Loaded>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load a compiled grammar library (once per process; it is never unloaded, since parsers
/// and trees keep pointing into it).
pub fn load(lib: &Path, symbol: &str) -> Result<Language, String> {
    let mut map = loaded().lock().map_err(|e| e.to_string())?;
    if let Some((_, lang)) = map.get(lib) {
        return Ok(lang.clone());
    }
    // SAFETY: a tree-sitter grammar library exports `const TSLanguage *<symbol>(void)`;
    // the library stays loaded for the rest of the process.
    let (library, lang) = unsafe {
        let library =
            libloading::Library::new(lib).map_err(|e| format!("{}: {e}", lib.display()))?;
        let f = *library
            .get::<unsafe extern "C" fn() -> *const ()>(symbol.as_bytes())
            .map_err(|e| format!("{}: no `{symbol}`: {e}", lib.display()))?;
        let lang = Language::new(tree_sitter_language::LanguageFn::from_raw(f));
        (library, lang)
    };
    map.insert(lib.to_path_buf(), (library, lang.clone()));
    Ok(lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grammar's source from the local cargo registry, if this machine has it.
    fn local_source(krate: &str) -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        let reg = PathBuf::from(home).join(".cargo/registry/src");
        for idx in std::fs::read_dir(reg).ok()?.flatten() {
            let mut hits: Vec<PathBuf> = std::fs::read_dir(idx.path())
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().is_some_and(|n| {
                        n.to_string_lossy()
                            .strip_prefix(&format!("{krate}-"))
                            .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
                    })
                })
                .collect();
            hits.sort();
            if let Some(h) = hits.pop() {
                return Some(h);
            }
        }
        None
    }

    #[test]
    fn compiles_loads_and_parses_a_grammar_from_source() {
        // Prisma (C only) and Vue (C++ scanner), compiled from local sources: no network.
        for (name, krate) in [("prisma", "tree-sitter-prisma"), ("vue", "tree-sitter-vue")] {
            let Some(src) = local_source(krate) else {
                eprintln!("{krate} source not in the cargo registry; skipping");
                continue;
            };
            let data =
                std::env::temp_dir().join(format!("farsub-gram-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&data);
            let r = GrammarRecipe {
                name: name.into(),
                source: Some(src.display().to_string()),
                ..Default::default()
            };
            let lib = run_install(&data, &r, &|m| eprintln!("{m}")).unwrap();
            assert_eq!(installed_lib(&data, name), Some(lib.clone()));
            let lang = load(&lib, &r.symbol()).unwrap();
            let mut p = tree_sitter::Parser::new();
            p.set_language(&lang).unwrap();
            let code = if name == "vue" {
                "<template><div>{{ msg }}</div></template>\n"
            } else {
                "model User {\n  id Int @id\n}\n"
            };
            let tree = p.parse(code, None).unwrap();
            assert!(
                !tree.root_node().has_error(),
                "{}",
                tree.root_node().to_sexp()
            );
            assert!(
                installed_query(&data, name).is_some(),
                "its own query is kept"
            );
            uninstall(&data, name).unwrap();
            assert!(installed_lib(&data, name).is_none());
            let _ = std::fs::remove_dir_all(&data);
        }
    }

    #[test]
    fn installs_from_an_archive_and_names_the_grammar() {
        let Some(src) = local_source("tree-sitter-prisma") else {
            eprintln!("tree-sitter-prisma source not in the cargo registry; skipping");
            return;
        };
        let data = std::env::temp_dir().join(format!("farsub-gram-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        // A .tar.gz of the source folder, as a user might download it.
        let archive = data.join("prisma-grammar.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(
                std::fs::File::create(&archive).unwrap(),
                flate2::Compression::fast(),
            );
            let mut t = tar::Builder::new(gz);
            t.append_dir_all("tree-sitter-prisma", &src).unwrap();
            t.into_inner().unwrap().finish().unwrap();
        }
        let (name, lib) = install_from_path(&data, &archive, &|m| eprintln!("{m}")).unwrap();
        assert_eq!(name, "prisma", "named from parser.c / tree-sitter.json");
        assert_eq!(installed_lib(&data, "prisma"), Some(lib.clone()));
        let rec = installed(&data);
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].0.symbol, "tree_sitter_prisma");
        assert!(rec[0].0.source.ends_with("prisma-grammar.tar.gz"));
        // Not a grammar: a clear error.
        let empty = data.join("nothing");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(
            install_from_path(&data, &empty, &|_| {})
                .unwrap_err()
                .contains("parser.c")
        );
        let _ = std::fs::remove_dir_all(&data);
    }
}
