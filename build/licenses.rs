//! Licenses and attribution (Help › Licenses): every crate compiled into this build and
//! the bundled data that carries its own notice, with the license texts. Generated from
//! `cargo metadata` for the target being built, so it follows the dependencies by itself.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;
use std::process::Command;

/// Notices for bundled data that aren't named LICENSE*/COPYING*/NOTICE*.
const EXTRA_FILES: &[(&str, &[&str])] = &[(
    "epaint_default_fonts",
    &[
        "fonts/OFL.txt",
        "fonts/UFL.txt",
        "fonts/Hack-Regular.txt",
        "fonts/emoji-icon-font-mit-license.txt",
    ],
)];

/// Build-only dependencies whose data still ends up in the binary (two-face: the fallback
/// syntaxes are dumped from it in build.rs, and the bundled themes were converted from its
/// theme set).
const EMBEDDED_BUILD_DEPS: &[&str] = &["two-face"];

/// Something with license texts: a crate or a piece of bundled data.
struct Item {
    name: String,
    version: String,
    license: String,
    /// (file name, text)
    texts: Vec<(String, String)>,
}

pub fn generate(manifest_dir: &Path, runtime: &Path, target: &str) -> String {
    let meta = metadata(manifest_dir, target);
    let crates = crates_in_binary(&meta);
    let mut data = bundled_data(runtime);
    data.sort_by(|a, b| a.name.cmp(&b.name));

    // Each distinct license text once, numbered, with what it covers.
    let mut texts: Vec<(String, String, Vec<String>)> = vec![]; // (title, text, covers)
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut refs = |item: &Item| -> Vec<usize> {
        let mut out = vec![];
        for (file, text) in &item.texts {
            let text = license_terms(text);
            // Copies of the full Apache text differ only in the header (URL, a comma).
            let body = match (text.trim_start().starts_with("Apache License"), text.find("1. Definitions")) {
                (true, Some(i)) if text.ends_with("END OF TERMS AND CONDITIONS") => &text[i..],
                _ => text.as_str(),
            };
            let key = body.split_whitespace().collect::<Vec<_>>().join(" ");
            if key.is_empty() {
                continue;
            }
            let n = *index.entry(key).or_insert_with(|| {
                texts.push((title_of(&text), text.trim_end().to_string(), vec![]));
                texts.len() - 1
            });
            let who = if item.version.is_empty() {
                format!("{} ({file})", item.name)
            } else {
                format!("{} {} ({file})", item.name, item.version)
            };
            texts[n].2.push(who);
            if !out.contains(&n) {
                out.push(n);
            }
        }
        out
    };
    let data_refs: Vec<Vec<usize>> = data.iter().map(&mut refs).collect();
    let crate_refs: Vec<Vec<usize>> = crates.iter().map(&mut refs).collect();
    // Number the texts grouped by title (all the MIT ones together, and so on).
    let mut order: Vec<usize> = (0..texts.len()).collect();
    order.sort_by(|&a, &b| texts[a].0.cmp(&texts[b].0).then(a.cmp(&b)));
    let mut number = vec![0; texts.len()];
    for (k, &i) in order.iter().enumerate() {
        number[i] = k + 1;
    }
    let cite = |r: &[usize]| -> String {
        if r.is_empty() {
            return "—".into();
        }
        let mut v: Vec<usize> = r.iter().map(|&i| number[i]).collect();
        v.sort();
        v.iter()
            .map(|n| format!("[{n}]"))
            .collect::<Vec<_>>()
            .join(" ")
    };

    let mut md = String::new();
    let _ = writeln!(
        md,
        "# Licenses and attribution\n\n\
         farsub is built on the work of many open-source projects. This lists the third-party\n\
         code and data compiled into this build ({target}), the license each is under, and\n\
         the license texts, each text once, followed by what it covers. Numbers in brackets\n\
         point to those texts.\n\n\
         Language servers and tree-sitter grammars that farsub downloads for you aren't part\n\
         of this build; they come from their own authors under their own licenses.\n"
    );
    let _ = writeln!(md, "## Bundled data\n");
    for (it, r) in data.iter().zip(&data_refs) {
        let _ = writeln!(md, "- {} — {} {}", it.name, it.license, cite(r));
    }
    let _ = writeln!(md, "\n## Rust crates ({})\n", crates.len());
    let _ = writeln!(md, "| Crate | Version | License | Text |\n| --- | --- | --- | --- |");
    for (it, r) in crates.iter().zip(&crate_refs) {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} |",
            it.name,
            it.version,
            it.license,
            cite(r)
        );
    }
    if crate_refs.iter().any(|r| r.is_empty()) {
        let _ = writeln!(
            md,
            "\n— the crate ships no license file; its license is the one declared in the\n\
             License column, whose text appears below."
        );
    }
    let _ = writeln!(md, "\n## License texts");
    for &i in &order {
        let (title, text, covers) = &texts[i];
        let _ = writeln!(md, "\n### [{}] {title}\n", number[i]);
        md.push_str(&wrap(&format!("Covers: {}.", covers.join(", ")), 92));
        let _ = writeln!(md, "\n````text\n{text}\n````");
    }
    md
}

fn metadata(dir: &Path, target: &str) -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let run = |offline: bool| {
        let mut c = Command::new(&cargo);
        c.current_dir(dir).args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            target,
        ]);
        if offline {
            c.arg("--offline");
        }
        c.output().ok().filter(|o| o.status.success())
    };
    let out = run(true)
        .or_else(|| run(false))
        .expect("`cargo metadata` failed: it lists the crates for Help › Licenses");
    serde_json::from_slice(&out.stdout).expect("cargo metadata: bad JSON")
}

/// The crates linked into the binary: normal dependencies of farsub, transitively, for
/// this target. Proc-macros (and what only they use) run at compile time and are left out.
fn crates_in_binary(meta: &Value) -> Vec<Item> {
    let pkgs: HashMap<&str, &Value> = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p))
        .collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["id"].as_str().unwrap(), n))
        .collect();
    let root = meta["resolve"]["root"].as_str().expect("no root package");
    let is_proc_macro = |p: &Value| {
        p["targets"].as_array().is_some_and(|ts| {
            ts.iter().any(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "proc-macro"))
            })
        })
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let Some(node) = nodes.get(id) else { continue };
        for d in node["deps"].as_array().into_iter().flatten() {
            let dep = d["pkg"].as_str().unwrap();
            let p = pkgs[dep];
            let kinds = d["dep_kinds"].as_array().cloned().unwrap_or_default();
            let normal = kinds.iter().any(|k| k["kind"].is_null());
            let embedded = id == root
                && kinds.iter().any(|k| k["kind"] == "build")
                && EMBEDDED_BUILD_DEPS.contains(&p["name"].as_str().unwrap());
            if (normal || embedded) && !is_proc_macro(p) && seen.insert(dep) {
                // An embedded build dependency counts on its own, not with its dependencies.
                if normal {
                    stack.push(dep);
                }
            }
        }
    }
    let mut out: Vec<Item> = seen
        .iter()
        .map(|id| {
            let p = pkgs[id];
            let name = p["name"].as_str().unwrap().to_string();
            let dir = Path::new(p["manifest_path"].as_str().unwrap())
                .parent()
                .unwrap()
                .to_path_buf();
            let mut files: Vec<String> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|f| {
                    let u = f.to_uppercase();
                    ["LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE", "COPYRIGHT"]
                        .iter()
                        .any(|s| u.starts_with(s))
                })
                .collect();
            if let Some(f) = p["license_file"].as_str()
                && !files.iter().any(|x| x == f)
            {
                files.push(f.to_string());
            }
            for (n, extra) in EXTRA_FILES {
                if *n == name {
                    files.extend(extra.iter().map(|s| s.to_string()));
                }
            }
            // Under "A OR B" farsub uses the permissive option: a GPL text in a dual-licensed
            // crate would only suggest farsub is under GPL. SPDX files are metadata.
            let license = p["license"].as_str().unwrap_or("");
            files.retain(|f| {
                let u = f.to_uppercase();
                !u.ends_with(".SPDX") && !(license.contains(" OR ") && u.contains("GPL"))
            });
            files.sort();
            let texts = files
                .into_iter()
                .filter_map(|f| {
                    let bytes = std::fs::read(dir.join(&f)).ok()?;
                    Some((f, String::from_utf8_lossy(&bytes).into_owned()))
                })
                .collect();
            Item {
                name,
                version: p["version"].as_str().unwrap_or("").to_string(),
                license: p["license"]
                    .as_str()
                    .unwrap_or("see license file")
                    .to_string(),
                texts,
            }
        })
        .collect();
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    out
}

/// Data in runtime/ (or dumped from two-face) that carries its own notice.
fn bundled_data(runtime: &Path) -> Vec<Item> {
    let mut out = vec![];
    // Themes taken from elsewhere keep their source and license in a leading comment.
    let mut themes: Vec<_> = std::fs::read_dir(runtime.join("themes"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    themes.sort();
    for p in themes {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let head: Vec<&str> = text
            .lines()
            .take_while(|l| l.starts_with('#'))
            .map(|l| {
                let l = l.trim_start_matches('#');
                l.strip_prefix(' ').unwrap_or(l)
            })
            .collect();
        if !head.iter().any(|l| l.contains("Copyright")) {
            continue;
        }
        let name = text
            .lines()
            .find_map(|l| l.strip_prefix("name = "))
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|| p.file_stem().unwrap().to_string_lossy().into_owned());
        out.push(Item {
            name: format!("Color theme \"{name}\""),
            version: String::new(),
            license: license_of(&head.join("\n")),
            texts: vec![(
                format!("themes/{}", p.file_name().unwrap().to_string_lossy()),
                head.join("\n"),
            )],
        });
    }
    // Fonts in runtime/fonts/: each <name>.ttf comes with <name>-LICENSE.txt (its source
    // in the first lines, then the license).
    let mut fonts: Vec<_> = std::fs::read_dir(runtime.join("fonts"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with("-LICENSE.txt"))
        .collect();
    fonts.sort();
    for p in fonts {
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let file = p.file_name().unwrap().to_string_lossy().into_owned();
        let font = file.trim_end_matches("-LICENSE.txt");
        let license = if text.contains("Attribution 4.0 International") {
            "CC-BY-4.0".to_string()
        } else {
            license_of(&text)
        };
        // Named by the notice's first words ("Codicons icon font (codicon.ttf), …").
        let title = text.lines().next().and_then(|l| l.split(" (").next());
        out.push(Item {
            name: title.map_or_else(|| format!("Font {font}.ttf"), str::to_string),
            version: String::new(),
            license,
            texts: vec![(format!("fonts/{file}"), text)],
        });
    }
    // two-face (bat's set): the fallback syntaxes, and the themes most bundled themes
    // were converted from.
    let ack = two_face::acknowledgement::listing();
    for (kind, list) in [("Syntax", ack.for_syntaxes()), ("Color scheme", ack.for_themes())] {
        for l in list {
            let path = l.rel_path.to_string_lossy().replace('\\', "/");
            out.push(Item {
                name: format!("{kind} from bat: {path}"),
                version: String::new(),
                license: license_of(&l.text),
                texts: vec![(path, l.text.clone())],
            });
        }
    }
    out
}

fn license_of(text: &str) -> String {
    let t = title_of(text);
    if t == "Other notices" {
        "see notice".into()
    } else {
        t
    }
}

/// The license's terms: an Apache-2.0 text up to "END OF TERMS AND CONDITIONS" (the
/// appendix after it only says how to apply the license, and its wording varies between
/// copies, which would list the same license dozens of times).
fn license_terms(text: &str) -> String {
    let t = text.trim();
    if title_of(t) == "Apache License 2.0"
        && let Some(i) = t.find("END OF TERMS AND CONDITIONS")
    {
        return t[..i + "END OF TERMS AND CONDITIONS".len()].to_string();
    }
    t.to_string()
}

/// A short name for a license text, from its wording.
fn title_of(text: &str) -> String {
    let t = text.to_lowercase();
    let has = |s: &str| t.contains(s);
    let name = if has("apache license") && has("version 2.0") {
        "Apache License 2.0"
    } else if has("permission is hereby granted, free of charge") {
        "MIT"
    } else if has("sil open font license") {
        "SIL Open Font License 1.1"
    } else if has("ubuntu font licence") {
        "Ubuntu Font Licence 1.0"
    } else if has("mozilla public license") {
        "Mozilla Public License 2.0"
    } else if has("boost software license") {
        "Boost Software License 1.0"
    } else if has("this is free and unencumbered software") {
        "Unlicense"
    } else if has("unicode") && has("data files and any associated documentation") {
        "Unicode License"
    } else if has("provided 'as-is'") || has("provided \"as-is\"") {
        "zlib"
    } else if has("redistribution and use in source and binary forms") {
        "BSD"
    } else if has("permission to use, copy, modify, and/or distribute")
        || has("permission to use, copy, modify, and distribute")
    {
        "ISC"
    } else if has("permission to copy, use, modify, sell and distribute") {
        "Sublime Text package license"
    } else if has("community data license agreement") && has("permissive") {
        "Community Data License Agreement – Permissive 2.0"
    } else if has("creative commons") && has("cc0") {
        "Creative Commons CC0 1.0"
    } else if has("dual-licensed under") || has("licensed under either of") {
        "Licensing notes"
    } else if has("public domain") {
        "Public domain"
    } else {
        "Other notices"
    };
    name.to_string()
}

/// Wrap at `width` columns, breaking after ", ".
fn wrap(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in s.split_inclusive(' ') {
        if line.len() + word.len() > width && !line.is_empty() {
            out.push_str(line.trim_end());
            out.push('\n');
            line.clear();
        }
        line.push_str(word);
    }
    out.push_str(line.trim_end());
    out.push('\n');
    out
}
