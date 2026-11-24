//! The tree-sitter grammars compiled into FarSub, by name: a core of common languages.
//! Others are installed at runtime from recipes in `grammars/*.toml` (see
//! grammar_install.rs). A language picks one with
//! `grammar = "…"` in its TOML (default: the language's own name). Each grammar's highlight
//! query is data: `runtime/queries/<name>/highlights.scm`, overridable from packages and the
//! config folder.

use tree_sitter::Language;

/// (grammar name, language) for every bundled grammar.
pub fn builtin() -> Vec<(&'static str, Language)> {
    vec![
        ("bash", tree_sitter_bash::LANGUAGE.into()),
        ("c", tree_sitter_c::LANGUAGE.into()),
        ("cpp", tree_sitter_cpp::LANGUAGE.into()),
        ("css", tree_sitter_css::LANGUAGE.into()),
        ("go", tree_sitter_go::LANGUAGE.into()),
        ("html", tree_sitter_html::LANGUAGE.into()),
        ("ini", tree_sitter_ini::LANGUAGE.into()),
        ("java", tree_sitter_java::LANGUAGE.into()),
        ("javascript", tree_sitter_javascript::LANGUAGE.into()),
        ("json", tree_sitter_json::LANGUAGE.into()),
        ("lua", tree_sitter_lua::LANGUAGE.into()),
        ("make", tree_sitter_make::LANGUAGE.into()),
        ("markdown", tree_sitter_md::LANGUAGE.into()),
        ("php", tree_sitter_php::LANGUAGE_PHP.into()),
        ("python", tree_sitter_python::LANGUAGE.into()),
        ("ruby", tree_sitter_ruby::LANGUAGE.into()),
        ("rust", tree_sitter_rust::LANGUAGE.into()),
        ("toml", tree_sitter_toml_ng::LANGUAGE.into()),
        ("tsx", tree_sitter_typescript::LANGUAGE_TSX.into()),
        (
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ),
        ("xml", tree_sitter_xml::LANGUAGE_XML.into()),
        ("yaml", tree_sitter_yaml::LANGUAGE.into()),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_grammar_loads_into_the_parser() {
        for (name, lang) in super::builtin() {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&lang)
                .unwrap_or_else(|e| panic!("{name}: incompatible grammar: {e}"));
        }
    }
}
