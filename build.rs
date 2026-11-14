//! Embed every file under runtime/ (default languages, servers, keymap, menus) so the binary
//! is self-contained; user files in the config folder override them at runtime.
//!
//! Also precompiles the bundled grammars: two-face's set plus runtime/syntaxes/*.sublime-syntax,
//! dumped once here so startup only deserializes them.

use std::fmt::Write;
use std::path::Path;

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p.extension().is_some_and(|x| x == "toml") {
            out.push(
                p.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    println!("cargo:rerun-if-changed=runtime");
    let mut files = vec![];
    walk(&root, &root, &mut files);
    let mut src = String::from("pub const RUNTIME_FILES: &[(&str, &str)] = &[\n");
    for f in files {
        let abs = root.join(&f);
        writeln!(
            src,
            "    ({f:?}, include_str!({:?})),",
            abs.to_string_lossy()
        )
        .unwrap();
    }
    src.push_str("];\n");
    let out_dir = Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    std::fs::write(out_dir.join("runtime_files.rs"), src).unwrap();
    bundle_syntaxes(&root.join("syntaxes"), &out_dir.join("syntaxes.packdump"));
}

fn bundle_syntaxes(dir: &Path, out: &Path) {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p: &std::path::PathBuf| p.extension().is_some_and(|x| x == "sublime-syntax"));
    files.sort();
    let mut b = two_face::syntax::extra_newlines().into_builder();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let def = syntect::parsing::SyntaxDefinition::load_from_str(&text, true, None)
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        b.add(def);
    }
    syntect::dumps::dump_to_uncompressed_file(&b.build(), out).unwrap();
}
