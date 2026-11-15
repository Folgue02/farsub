//! Command-line interface, modeled on Sublime's `subl`.

use std::path::{Path, PathBuf};

use clap::Parser;

use crate::config::Config;
use crate::project::{Last, PROJECT_EXT};

#[derive(Parser, Debug, Default)]
#[command(
    name = "farsub",
    version,
    about = "A Sublime Text-style editor with LSP, projects and a keyboard-driven sidebar.",
    after_help = "Examples:
  farsub                      reopen the last project/folders (or the current folder)
  farsub .                    open the current folder
  farsub src/main.rs:42:5     open a file at line 42, column 5
  farsub app.farsub-project   open a project
  farsub -a ../lib            add a folder to the last session's project
  git log | farsub -          read stdin into a new buffer
  git config core.editor \"farsub -w\"   use as git's editor (waits for the window)
  farsub --install-server pyright"
)]
pub struct Args {
    /// Files (FILE[:LINE[:COL]]), folders or .farsub-project files to open; "-" reads stdin.
    pub paths: Vec<String>,

    /// Open a project file.
    #[arg(short, long, value_name = "FILE")]
    pub project: Option<PathBuf>,

    /// Don't restore the previous session.
    #[arg(short, long)]
    pub new: bool,

    /// Open an empty window: no folder, no project, no session (File › New Window).
    #[arg(long, conflicts_with_all = ["project", "add"])]
    pub empty: bool,

    /// Add the given folders to the last session's project instead of opening them alone.
    #[arg(short, long)]
    pub add: bool,

    /// Stay in the foreground until the window is closed (for $EDITOR / git).
    #[arg(short, long)]
    pub wait: bool,

    /// (internal) read the "-" buffer from this file instead of stdin.
    #[arg(long, hide = true, value_name = "FILE")]
    pub stdin_file: Option<PathBuf>,

    /// Use this config.toml (its folder holds languages/, servers/, keymap.toml, Packages/).
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Where installed servers, sessions and recent projects are kept.
    #[arg(long, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// Print the configured language servers and their status, then exit.
    #[arg(long)]
    pub list_servers: bool,

    /// Install a language server (like :MasonInstall), then exit. Repeatable.
    #[arg(long, value_name = "NAME")]
    pub install_server: Vec<String>,

    /// Uninstall a server installed by farsub, then exit. Repeatable.
    #[arg(long, value_name = "NAME")]
    pub uninstall_server: Vec<String>,

    /// Print the built-in and installable tree-sitter grammars, then exit.
    #[arg(long)]
    pub list_grammars: bool,

    /// Install a grammar, then exit: a recipe NAME (downloads the source) or a PATH to a
    /// source folder / archive. Compiles it with your C compiler. Repeatable.
    #[arg(long, value_name = "NAME|PATH")]
    pub install_grammar: Vec<String>,

    /// Install a package from a folder or archive (.zip, .tar.gz), then exit. Repeatable.
    #[arg(long, value_name = "PATH")]
    pub install_package: Vec<PathBuf>,

    /// Remove an installed grammar, then exit. Repeatable.
    #[arg(long, value_name = "NAME")]
    pub uninstall_grammar: Vec<String>,

    /// Validate config, keymap, menus (and --project) and exit; non-zero on errors.
    #[arg(long)]
    pub check_config: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenFile {
    pub path: PathBuf,
    pub line: Option<usize>,
    pub col: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Project(PathBuf),
    Folders(Vec<PathBuf>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub target: Target,
    /// Restore the target's workspace (open files, unsaved changes).
    pub restore: bool,
    /// Extra folders to add (`--add`).
    pub add: Vec<PathBuf>,
    pub files: Vec<OpenFile>,
    pub stdin: bool,
}

fn abs(cwd: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    p.canonicalize()
        .unwrap_or_else(|_| crate::project::normalize(&p))
}

/// `src/a.rs:12:3` → (src/a.rs, 12, 3), only if the plain path doesn't exist.
pub fn split_location(cwd: &Path, s: &str) -> OpenFile {
    let plain = abs(cwd, s);
    if plain.exists() {
        return OpenFile {
            path: plain,
            line: None,
            col: None,
        };
    }
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();
    let num = |x: &str| x.parse::<usize>().ok().filter(|n| *n > 0);
    match parts.as_slice() {
        [c, l, f] if num(c).is_some() && num(l).is_some() && !f.is_empty() => OpenFile {
            path: abs(cwd, f),
            line: num(l),
            col: num(c),
        },
        [l, f] | [l, f, _] if num(l).is_some() && !f.is_empty() => {
            // "a.rs:12" (rsplitn gives [12, a.rs]); with 3 parts the first wasn't a number.
            let f = if parts.len() == 3 {
                s.rsplit_once(':').map(|x| x.0).unwrap_or(f)
            } else {
                f
            };
            OpenFile {
                path: abs(cwd, f),
                line: num(l),
                col: None,
            }
        }
        _ => OpenFile {
            path: plain,
            line: None,
            col: None,
        },
    }
}

/// Decide what to open from the arguments, the current folder and the last session.
pub fn plan(
    args: &Args,
    cwd: &Path,
    last: Option<&Last>,
    restore_session: bool,
) -> Result<Plan, String> {
    let mut folders = vec![];
    let mut files = vec![];
    let mut project = args
        .project
        .as_ref()
        .map(|p| abs(cwd, &p.to_string_lossy()));
    let mut stdin = false;
    for p in &args.paths {
        if p == "-" {
            stdin = true;
            continue;
        }
        let path = abs(cwd, p);
        if path.is_dir() {
            folders.push(path);
        } else if path.extension().is_some_and(|e| e == PROJECT_EXT) {
            if !path.is_file() {
                return Err(format!("no such project: {}", path.display()));
            }
            if project.is_some() {
                return Err("only one project can be opened".into());
            }
            project = Some(path);
        } else {
            files.push(split_location(cwd, p));
        }
    }
    if let Some(p) = &project
        && !p.is_file()
    {
        return Err(format!("no such project: {}", p.display()));
    }
    if args.empty {
        // A clean window; files given too still open in it.
        return Ok(Plan {
            target: Target::Folders(vec![]),
            restore: false,
            add: vec![],
            files,
            stdin,
        });
    }
    // The last session, if it still exists.
    let last = (!args.new && restore_session)
        .then_some(last)
        .flatten()
        .and_then(|l| match l {
            Last::Project(f) if f.is_file() => Some(Target::Project(f.clone())),
            Last::Folders(fs) if !fs.is_empty() && fs.iter().all(|f| f.is_dir()) => {
                Some(Target::Folders(fs.clone()))
            }
            _ => None,
        });
    let restore = !args.new;
    let plan = |target, add: Vec<PathBuf>| Plan {
        target,
        restore,
        add,
        files: files.clone(),
        stdin,
    };
    Ok(if let Some(p) = project {
        plan(Target::Project(p), folders)
    } else if !folders.is_empty() {
        match (args.add, last) {
            (true, Some(l)) => plan(l, folders),
            _ => plan(Target::Folders(folders), vec![]),
        }
    } else if let Some(l) = last {
        // Like `subl file`: open files in the previous session.
        plan(l, vec![])
    } else if let Some(f) = files.first() {
        let dir = f.path.parent().unwrap_or(cwd);
        let dir = if dir.is_dir() {
            dir.to_path_buf()
        } else {
            cwd.to_path_buf()
        };
        plan(
            Target::Folders(vec![crate::lsp::find_root(&dir, &[])]),
            vec![],
        )
    } else {
        plan(Target::Folders(vec![cwd.to_path_buf()]), vec![])
    })
}

/// Handle the non-GUI options. Returns `Some(exit code)` if one ran.
pub fn run_headless(args: &Args, config: &Config) -> Option<i32> {
    use crate::installer;
    if args.check_config {
        let mut bad = false;
        if let Some(e) = &config.error {
            eprintln!("config: {e}");
            bad = true;
        }
        let (km, errs) = crate::keymap::Keymap::from_config(config);
        let (menus, merrs) = crate::menu::build(&config.menus);
        for e in errs.iter().chain(&merrs) {
            eprintln!("{e}");
            bad = true;
        }
        for l in &config.languages {
            for s in &l.servers {
                if config.server(s).is_none() {
                    eprintln!("language {}: unknown server `{s}`", l.name);
                    bad = true;
                }
            }
            if let Some(r) = &l.first_line
                && regex::Regex::new(r).is_err()
            {
                eprintln!("language {}: bad first_line regex `{r}`", l.name);
                bad = true;
            }
        }
        if let Some(p) = &args.project {
            match crate::project::Project::load(p) {
                Ok(pr) => {
                    for f in &pr.folders {
                        if !f.path.is_dir() {
                            eprintln!("project: folder {} doesn't exist", f.path.display());
                            bad = true;
                        }
                    }
                    if let Err(e) = crate::project::apply_settings(&config.editor, &pr.settings) {
                        eprintln!("project settings: {e}");
                        bad = true;
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                    bad = true;
                }
            }
        }
        println!(
            "{} languages, {} servers, {} key bindings, {} menus{}",
            config.languages.len(),
            config.servers.len(),
            km.bindings.len(),
            menus.len(),
            config
                .path
                .as_ref()
                .map(|p| format!(" (config: {})", p.display()))
                .unwrap_or_default()
        );
        return Some(if bad { 1 } else { 0 });
    }
    let data = config.data_dir.clone();
    if args.list_servers {
        let mut servers = config.servers.clone();
        servers.sort_by(|a, b| a.name.cmp(&b.name));
        for s in servers {
            let status = match data
                .as_deref()
                .and_then(|d| installer::installed_bin(d, &s))
            {
                Some(b) => format!("installed  {}", b.display()),
                None => match crate::lsp::find_server(&s.command) {
                    Some(p) => format!("on PATH    {}", p.display()),
                    None => "missing".into(),
                },
            };
            let src = s
                .install
                .as_ref()
                .map(|i| i.source())
                .unwrap_or_else(|| "manual".into());
            println!(
                "{:<16} {:<28} {:<40} [{}]",
                s.name,
                config.languages_using(&s.name).join(","),
                status,
                src
            );
        }
        return Some(0);
    }
    if !args.install_package.is_empty() {
        let Some(dir) = config.packages_dir() else {
            eprintln!("no config folder for packages");
            return Some(1);
        };
        let mut code = 0;
        for p in &args.install_package {
            match crate::packages::install_from_path(&dir, p) {
                Ok(name) => println!("installed package {name}"),
                Err(e) => {
                    eprintln!("{}: {e}", p.display());
                    code = 1;
                }
            }
        }
        return Some(code);
    }
    if args.list_grammars {
        let mut rows: Vec<(String, String, String)> = crate::grammars::builtin()
            .into_iter()
            .map(|(n, _)| (n.to_string(), "built in".to_string(), String::new()))
            .collect();
        for r in &config.grammars {
            let status = match data
                .as_deref()
                .and_then(|d| crate::grammar_install::installed_lib(d, &r.name))
            {
                Some(p) => format!("installed  {}", p.display()),
                None => "not installed".into(),
            };
            rows.push((r.name.clone(), status, r.source_desc()));
        }
        rows.sort();
        for (name, status, src) in rows {
            let langs: Vec<&str> = config
                .languages
                .iter()
                .filter(|l| l.grammar_name() == name)
                .map(|l| l.name.as_str())
                .collect();
            println!("{name:<14} {:<22} {status:<60} {src}", langs.join(","));
        }
        return Some(0);
    }
    if !args.install_grammar.is_empty() || !args.uninstall_grammar.is_empty() {
        let Some(data) = data else {
            eprintln!("no data folder (set $HOME, $FARSUB_DATA or --data-dir)");
            return Some(1);
        };
        let mut code = 0;
        for name in &args.uninstall_grammar {
            match crate::grammar_install::uninstall(&data, name) {
                Ok(()) => println!("uninstalled {name}"),
                Err(e) => {
                    eprintln!("{name}: {e}");
                    code = 1;
                }
            }
        }
        for name in &args.install_grammar {
            // A path installs from that file or folder.
            if Path::new(name).exists() {
                println!("installing a grammar from {name}…");
                match crate::grammar_install::install_from_path(&data, Path::new(name), &|l| {
                    println!("  {l}")
                }) {
                    Ok((g, p)) => println!("{g}: {}", p.display()),
                    Err(e) => {
                        eprintln!("{name}: {e}");
                        code = 1;
                    }
                }
                continue;
            }
            let Some(r) = config.grammar_recipe(name) else {
                eprintln!("no recipe for `{name}` (see --list-grammars)");
                code = 1;
                continue;
            };
            println!("installing {name} ({})…", r.source_desc());
            match crate::grammar_install::run_install(&data, r, &|l| println!("  {l}")) {
                Ok(p) => println!("{name}: {}", p.display()),
                Err(e) => {
                    eprintln!("{name}: {e}");
                    code = 1;
                }
            }
        }
        return Some(code);
    }
    if !args.install_server.is_empty() || !args.uninstall_server.is_empty() {
        let Some(data) = data else {
            eprintln!("no data folder (set $HOME, $FARSUB_DATA or --data-dir)");
            return Some(1);
        };
        let mut code = 0;
        for name in &args.uninstall_server {
            match installer::uninstall(&data, name) {
                Ok(()) => println!("uninstalled {name}"),
                Err(e) => {
                    eprintln!("{name}: {e}");
                    code = 1;
                }
            }
        }
        for name in &args.install_server {
            let Some(s) = config.server(name) else {
                eprintln!("unknown server `{name}` (see --list-servers)");
                code = 1;
                continue;
            };
            println!("installing {name}…");
            match installer::run_install(&data, s, &|l| println!("  {l}")) {
                Ok(b) => println!("{name}: {}", b.display()),
                Err(e) => {
                    eprintln!("{name}: {e}");
                    code = 1;
                }
            }
        }
        return Some(code);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Args {
        Args::try_parse_from(std::iter::once("farsub").chain(v.iter().copied())).unwrap()
    }

    #[test]
    fn empty_window() {
        let d = std::env::temp_dir();
        let last = Last::Folders(vec![d.clone()]);
        let p = plan(&args(&["--empty"]), &d, Some(&last), true).unwrap();
        assert_eq!(p.target, Target::Folders(vec![]));
        assert!(!p.restore, "no session");
        assert!(p.files.is_empty());
        // Files still open in it.
        let p = plan(&args(&["--empty", "notes.txt"]), &d, Some(&last), true).unwrap();
        assert_eq!(p.target, Target::Folders(vec![]));
        assert_eq!(p.files.len(), 1);
        assert!(Args::try_parse_from(["farsub", "--empty", "-p", "x.farsub-project"]).is_err());
    }

    #[test]
    fn plans() {
        let d = std::env::temp_dir().join(format!("farsub-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("proj/src")).unwrap();
        std::fs::create_dir_all(d.join("lib")).unwrap();
        std::fs::write(d.join("proj/src/a.rs"), "x").unwrap();
        std::fs::write(
            d.join("proj/P.farsub-project"),
            "[[folders]]\npath = \".\"\n",
        )
        .unwrap();
        let d = d.canonicalize().unwrap();
        let cwd = d.join("proj");

        assert_eq!(
            split_location(&cwd, "src/a.rs:12:3"),
            OpenFile {
                path: cwd.join("src/a.rs"),
                line: Some(12),
                col: Some(3)
            }
        );
        assert_eq!(split_location(&cwd, "src/a.rs:7").line, Some(7));
        assert_eq!(split_location(&cwd, "src/a.rs").line, None);
        assert_eq!(split_location(&cwd, "new.txt").path, cwd.join("new.txt"));

        // No args, no session → current folder.
        let p = plan(&args(&[]), &cwd, None, true).unwrap();
        assert_eq!(p.target, Target::Folders(vec![cwd.clone()]));
        // No args with a session → restore it.
        let last = Last::Folders(vec![d.join("lib")]);
        assert_eq!(
            plan(&args(&[]), &cwd, Some(&last), true).unwrap().target,
            Target::Folders(vec![d.join("lib")])
        );
        // -n ignores the session and doesn't restore.
        let p = plan(&args(&["-n"]), &cwd, Some(&last), true).unwrap();
        assert_eq!(p.target, Target::Folders(vec![cwd.clone()]));
        assert!(!p.restore);
        // A folder replaces the session; --add adds to it.
        assert_eq!(
            plan(&args(&["."]), &cwd, Some(&last), true).unwrap().target,
            Target::Folders(vec![cwd.clone()])
        );
        let p = plan(&args(&["-a", "."]), &cwd, Some(&last), true).unwrap();
        assert_eq!(
            (p.target, p.add),
            (Target::Folders(vec![d.join("lib")]), vec![cwd.clone()])
        );
        // Project files, positional or with -p.
        assert_eq!(
            plan(&args(&["P.farsub-project"]), &cwd, None, true)
                .unwrap()
                .target,
            Target::Project(cwd.join("P.farsub-project"))
        );
        assert_eq!(
            plan(&args(&["-p", "P.farsub-project"]), &cwd, None, true)
                .unwrap()
                .target,
            Target::Project(cwd.join("P.farsub-project"))
        );
        assert!(plan(&args(&["missing.farsub-project"]), &cwd, None, true).is_err());
        // Files alone open in the last session, or in their project root.
        let p = plan(&args(&["src/a.rs:3", "-"]), &cwd, None, true).unwrap();
        assert!(p.stdin);
        assert_eq!(p.files[0].line, Some(3));
        assert_eq!(p.target, Target::Folders(vec![d.join("proj/src")]));
    }
}
