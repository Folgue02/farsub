//! Embed every file under runtime/ (languages, servers, keymap, menus, themes, highlight
//! queries) so the binary is self-contained; user files in the config folder and packages
//! override them at runtime. (The tree-sitter grammars themselves are crates.)

use std::fmt::Write;
use std::path::Path;

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if p.extension().is_some_and(|x| x == "toml" || x == "scm") {
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
    // The target triple, for compiling grammars at runtime (grammar_install.rs).
    println!(
        "cargo:rustc-env=FARSUB_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
