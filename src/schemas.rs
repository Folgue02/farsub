//! JSON Schemas for FarSub's own TOML files (settings, key bindings, projects, languages,
//! servers), generated from the structs that read them so they can't drift. The TOML
//! language server (Taplo) uses them to check those files, complete their keys and show
//! each setting's explanation on hover.

use std::path::{Path, PathBuf};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Map, Value};

use crate::config::{
    Config, EditorConfig, FileConfig, KeymapFile, LanguageConfig, ServerConfig,
};
use crate::project::FolderDef;

/// A `.farsub-project` file (read by `project.rs`, which keeps it as a table).
#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
#[allow(dead_code)]
struct ProjectFile {
    /// The project's folders, shown in the sidebar.
    #[serde(default)]
    folders: Vec<FolderDef>,
    /// Editor settings for this project only, over your settings (same names as
    /// Preferences: Settings).
    #[serde(default)]
    settings: Option<EditorConfig>,
    /// Languages for this project only (same format as languages/<name>.toml).
    #[serde(default)]
    languages: Vec<LanguageConfig>,
    /// Language servers for this project only (same format as servers/<name>.toml).
    #[serde(default)]
    servers: Vec<ServerConfig>,
}

/// `command` in a key binding: one of the command ids (or "" / "none" to unbind).
pub fn command_id(_: &mut SchemaGenerator) -> Schema {
    let mut ids: Vec<Value> = crate::commands::COMMANDS
        .iter()
        .map(|&(c, _, _)| Value::String(crate::commands::id(c)))
        .collect();
    ids.push("".into());
    ids.push("none".into());
    json_schema!({
        "type": "string",
        "description": "Command id (all are listed in Preferences: Key Bindings); \"\" or \"none\" unbinds the keys.",
        "enum": ids,
    })
}

pub fn key_context(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "description": "Where the binding applies: \"global\" (default), \"editor\" or \"sidebar\".",
        "enum": ["global", "editor", "sidebar"],
    })
}

pub fn platforms(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "description": "Only on these systems.",
        "items": {"type": "string", "enum": ["macos", "windows", "linux"]},
    })
}

/// `Option<T>` comes out as "T or null": make it just T (and drop `"default": null`).
fn drop_null(v: &mut Value) {
    match v {
        Value::Object(o) => {
            if o.get("default").is_some_and(Value::is_null) {
                o.remove("default");
            }
            if let Some(Value::Array(types)) = o.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let t = types.remove(0);
                    o.insert("type".into(), t);
                }
            }
            let is_null = |x: &Value| x.get("type").is_some_and(|t| t == "null");
            if let Some(Value::Array(any)) = o.get("anyOf")
                && any.len() == 2
                && any.iter().any(is_null)
            {
                let Some(Value::Array(any)) = o.remove("anyOf") else { unreachable!() };
                if let Some(Value::Object(inner)) = any.into_iter().find(|x| !is_null(x)) {
                    for (k, x) in inner {
                        o.entry(k).or_insert(x);
                    }
                }
            }
            o.values_mut().for_each(drop_null);
        }
        Value::Array(a) => a.iter_mut().for_each(drop_null),
        _ => {}
    }
}

/// Each schema's file name and contents.
pub fn all() -> Vec<(&'static str, Value)> {
    fn of<T: JsonSchema>() -> Value {
        // Draft 7, which TOML tools know best; TOML has no null, so an optional key is
        // just its type (not "this or null", which makes errors vague).
        let mut v = schemars::generate::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<T>()
            .to_value();
        drop_null(&mut v);
        v
    }
    vec![
        ("settings.json", of::<FileConfig>()),
        ("keymap.json", of::<KeymapFile>()),
        ("project.json", of::<ProjectFile>()),
        ("language.json", of::<LanguageConfig>()),
        ("server.json", of::<ServerConfig>()),
    ]
}

/// Write the schemas into `<data>/schemas/` (rewritten each time, so they match this
/// version of FarSub) and return which files use which: Taplo's
/// `schema.associations`, a document-URI regex → schema URL.
pub fn associations(config: &Config, defaults: &[(PathBuf, &str)]) -> Map<String, Value> {
    let mut out = Map::new();
    let dir = schemas_dir(config);
    if std::fs::create_dir_all(&dir).is_err() {
        return out;
    }
    for (name, schema) in all() {
        let text = serde_json::to_string_pretty(&schema).unwrap_or_default();
        if std::fs::write(dir.join(name), text).is_err() {
            return out;
        }
    }
    let url = |name: &str| Value::String(crate::lsp::path_to_uri(&dir.join(name)));
    let exact = |p: &Path| format!("^{}$", regex::escape(&crate::lsp::path_to_uri(p)));
    if let Some(cfg) = &config.path {
        out.insert(exact(cfg), url("settings.json"));
        out.insert(exact(&crate::config::keymap_path(cfg)), url("keymap.json"));
        if let Some(d) = cfg.parent() {
            let folder = |sub: &str| {
                let base = crate::lsp::path_to_uri(&d.join(sub));
                format!("^{}/[^/]+\\.toml$", regex::escape(&base))
            };
            out.insert(folder("languages"), url("language.json"));
            out.insert(folder("servers"), url("server.json"));
        }
    }
    for (p, name) in defaults {
        out.insert(exact(p), url(name));
    }
    out.insert(
        format!("\\.{}$", crate::project::PROJECT_EXT),
        url("project.json"),
    );
    out
}

fn schemas_dir(config: &Config) -> PathBuf {
    config
        .data_dir
        .clone()
        .unwrap_or_else(std::env::temp_dir)
        .join("schemas")
}

/// The config files Taplo looks for at a project root.
pub const TAPLO_CONFIG_FILES: [&str; 2] = [".taplo.toml", "taplo.toml"];

/// Taplo only checks `*.toml` files unless its config file says otherwise; without one,
/// a `.farsub-project` gets just "this document has been excluded". For roots with no
/// Taplo config of their own, FarSub writes one that includes project files too and
/// points Taplo at it. `None`: the root has its own (which wins), or it couldn't be
/// written.
pub fn taplo_config(config: &Config, root: &Path) -> Option<PathBuf> {
    if TAPLO_CONFIG_FILES.iter().any(|n| root.join(n).is_file()) {
        return None;
    }
    let dir = schemas_dir(config);
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("taplo.toml");
    let text = format!(
        "# Written by FarSub for folders without a .taplo.toml of their own: Taplo checks\n\
         # only *.toml files by default, and FarSub's project files are TOML too.\n\
         include = [\"**/*.toml\", \"**/*.{}\"]\n",
        crate::project::PROJECT_EXT
    );
    std::fs::write(&path, text).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator(name: &str) -> jsonschema::Validator {
        let schema = all().into_iter().find(|(n, _)| *n == name).unwrap().1;
        jsonschema::validator_for(&schema).unwrap()
    }

    fn check(v: &jsonschema::Validator, what: &str, text: &str) {
        let value: toml::Value = toml::from_str(text).unwrap_or_else(|e| panic!("{what}: {e}"));
        let json = serde_json::to_value(value).unwrap();
        let errs: Vec<String> = v.iter_errors(&json).map(|e| format!("{e} at {}", e.instance_path())).collect();
        assert!(errs.is_empty(), "{what}: {errs:?}");
    }

    /// Everything FarSub ships passes its own schema, and typos don't.
    #[test]
    fn shipped_files_match_their_schemas() {
        let settings = validator("settings.json");
        check(&settings, "Default Settings", crate::config::DEFAULT_SETTINGS);
        check(&settings, "settings template", crate::config::TEMPLATE);
        let keymap = validator("keymap.json");
        check(&keymap, "Default Key Bindings", &crate::config::default_keymap_reference());
        check(&keymap, "keymap template", crate::config::KEYMAP_TEMPLATE);
        let project = validator("project.json");
        check(&project, "example project", crate::project::EXAMPLE_PROJECT);
        let (lang, server) = (validator("language.json"), validator("server.json"));
        for (name, text) in crate::config::RUNTIME_FILES {
            if name.starts_with("languages/") {
                check(&lang, name, text);
            } else if name.starts_with("servers/") {
                check(&server, name, text);
            }
        }
        let bad = |v: &jsonschema::Validator, text: &str| {
            let value: toml::Value = toml::from_str(text).unwrap();
            !v.is_valid(&serde_json::to_value(value).unwrap())
        };
        assert!(bad(&settings, "[editor]\nfont_sise = 12\n"));
        assert!(bad(&settings, "[editor]\nfont_size = \"big\"\n"));
        assert!(bad(&settings, "[editor]\ncaret_style = \"wobbly\"\n"));
        assert!(bad(&keymap, "[[keys]]\nkeys = \"Cmd+K\"\ncommand = \"no_such_command\"\n"));
        assert!(bad(&project, "[[folders]]\npaht = \".\"\n"));
        assert!(bad(&project, "[settings]\ntab_sise = 2\n"));
    }

    /// Roots without their own Taplo config get FarSub's, which includes project files.
    #[test]
    fn taplo_config_only_where_the_root_has_none() {
        let d = std::env::temp_dir().join(format!("farsub-taplocfg-{}", std::process::id()));
        std::fs::create_dir_all(d.join("plain")).unwrap();
        std::fs::create_dir_all(d.join("own")).unwrap();
        std::fs::write(d.join("own/.taplo.toml"), "").unwrap();
        let mut c = Config::default();
        c.data_dir = Some(d.join("data"));
        let f = taplo_config(&c, &d.join("plain")).expect("written");
        let text = std::fs::read_to_string(&f).unwrap();
        let t: toml::Table = toml::from_str(&text).unwrap();
        let include: Vec<&str> = (t["include"].as_array().unwrap().iter())
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(include, ["**/*.toml", "**/*.farsub-project"]);
        assert_eq!(taplo_config(&c, &d.join("own")), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn associations_cover_farsubs_files() {
        let d = std::env::temp_dir().join(format!("farsub-schemas-{}", std::process::id()));
        let mut c = Config::default();
        c.path = Some(d.join("My Config/config.toml"));
        c.data_dir = Some(d.join("data"));
        let a = associations(&c, &[(d.join("data/Default Settings.toml"), "settings.json")]);
        let target = |p: &Path| {
            let uri = crate::lsp::path_to_uri(p);
            let hits: Vec<&str> = a
                .iter()
                .filter(|(re, _)| regex::Regex::new(re).unwrap().is_match(&uri))
                .map(|(_, v)| v.as_str().unwrap().rsplit('/').next().unwrap())
                .collect();
            hits.join(",")
        };
        assert_eq!(target(&d.join("My Config/config.toml")), "settings.json");
        assert_eq!(target(&d.join("data/Default Settings.toml")), "settings.json");
        assert_eq!(target(&d.join("My Config/keymap.toml")), "keymap.json");
        assert_eq!(target(&d.join("My Config/languages/zig.toml")), "language.json");
        assert_eq!(target(&d.join("My Config/servers/zls.toml")), "server.json");
        assert_eq!(target(&d.join("src/App.farsub-project")), "project.json");
        assert_eq!(target(&d.join("src/Cargo.toml")), "");
        assert_eq!(target(&d.join("other/config.toml")), "");
        assert!(d.join("data/schemas/settings.json").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }
}
