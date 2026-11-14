//! Key bindings, built at runtime from runtime/keymap.toml plus the user's keymap.toml.
//! Key handling, menu shortcuts, the command palette's hints and the keymap viewer all
//! read from the same `Keymap`.
//!
//! Specs look like "Cmd+Shift+P". `Cmd` is the primary modifier (Cmd on macOS, Ctrl
//! elsewhere; both always work); `Ctrl` is written where Sublime uses Ctrl everywhere (it
//! matches the same way); plus `Shift` and `Alt`.

use crate::commands::{self, Command};
use crate::config::{Config, KeyDoc};
use crate::keys::{Key, KeyCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    /// Works wherever focus is (checked after the focused pane's own bindings).
    Global,
    Editor,
    Sidebar,
}

impl Ctx {
    fn parse(s: Option<&str>) -> Option<Ctx> {
        match s.unwrap_or("global") {
            "global" => Some(Ctx::Global),
            "editor" => Some(Ctx::Editor),
            "sidebar" => Some(Ctx::Sidebar),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub code: KeyCode,
    /// Cmd (macOS) / Ctrl, or literal Ctrl.
    pub ctrl: bool,
    /// Written as "Ctrl" (so native menus use Control, not Command, on macOS).
    pub literal_ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub keys: String,
    pub spec: Spec,
    pub cmd: Command,
    pub ctx: Ctx,
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
    pub docs: Vec<KeyDoc>,
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

impl Keymap {
    /// Build from config. Later entries (the user's keymap.toml) override earlier ones for
    /// the same key + context; `command = ""` / `"none"` unbinds.
    pub fn from_config(c: &Config) -> (Keymap, Vec<String>) {
        let mut km = Keymap {
            bindings: vec![],
            docs: c.docs.clone(),
        };
        let mut errs = vec![];
        for kb in &c.keys {
            if !kb.platforms.is_empty() && !kb.platforms.iter().any(|p| p == platform()) {
                continue;
            }
            let Some(spec) = parse(&kb.keys) else {
                errs.push(format!("bad key \"{}\"", kb.keys));
                continue;
            };
            let Some(ctx) = Ctx::parse(kb.context.as_deref()) else {
                errs.push(format!("bad context for \"{}\"", kb.keys));
                continue;
            };
            km.bindings
                .retain(|b| !(b.ctx == ctx && same_key(&b.spec, &spec)));
            if kb.command.is_empty() || kb.command == "none" {
                continue;
            }
            match commands::from_id(&kb.command) {
                Some(cmd) => km.bindings.push(Binding {
                    keys: kb.keys.clone(),
                    spec,
                    cmd,
                    ctx,
                }),
                None => errs.push(format!(
                    "unknown command \"{}\" for {}",
                    kb.command, kb.keys
                )),
            }
        }
        (km, errs)
    }

    /// Binding for a key in one context (no Global fallback).
    pub fn lookup(&self, k: Key, ctx: Ctx) -> Option<Command> {
        self.bindings
            .iter()
            .find(|b| b.ctx == ctx && matches(&b.spec, &k))
            .map(|b| b.cmd)
    }

    /// Display strings of every key bound to a command.
    pub fn keys_for(&self, cmd: Command) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|b| b.cmd == cmd)
            .map(|b| display(&b.keys))
            .collect()
    }

    /// The primary shortcut for a command (menus, palette hints).
    pub fn shortcut(&self, cmd: Command) -> String {
        self.primary(cmd)
            .map(|b| display(&b.keys))
            .unwrap_or_default()
    }

    /// The binding menus show / emulate: a modifier or F-key shortcut (global/editor first).
    pub fn primary(&self, cmd: Command) -> Option<&Binding> {
        let chord = |b: &Binding| b.spec.ctrl || b.spec.alt || matches!(b.spec.code, KeyCode::F(_));
        let mine = || self.bindings.iter().filter(move |b| b.cmd == cmd);
        mine()
            .find(|b| chord(b) && b.ctx != Ctx::Sidebar)
            .or_else(|| mine().find(|b| chord(b)))
            .or_else(|| mine().next())
    }
}

/// Whether two bindings name the same key (a later one replaces an earlier one). On
/// macOS Cmd+X and Ctrl+X are different keys.
fn same_key(a: &Spec, b: &Spec) -> bool {
    a.code == b.code
        && a.ctrl == b.ctrl
        && a.shift == b.shift
        && a.alt == b.alt
        && (!cfg!(target_os = "macos") || a.literal_ctrl == b.literal_ctrl)
}

pub fn parse(spec: &str) -> Option<Spec> {
    let mut s = Spec {
        code: KeyCode::Esc,
        ctrl: false,
        literal_ctrl: false,
        shift: false,
        alt: false,
    };
    let parts: Vec<&str> = spec.split('+').collect();
    let (key, mods) = parts.split_last()?;
    for m in mods {
        match *m {
            "Cmd" => s.ctrl = true,
            "Ctrl" => {
                s.ctrl = true;
                s.literal_ctrl = true;
            }
            "Shift" => s.shift = true,
            "Alt" => s.alt = true,
            _ => return None,
        }
    }
    s.code = match *key {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Menu" => KeyCode::Menu,
        "Space" => KeyCode::Char(' '),
        k if k.len() > 1 && k.starts_with('F') => KeyCode::F(k[1..].parse().ok()?),
        k => {
            let mut cs = k.chars();
            let c = cs.next()?;
            if cs.next().is_some() {
                return None;
            }
            KeyCode::Char(c.to_ascii_lowercase())
        }
    };
    Some(s)
}

/// How a spec is shown on this platform.
pub fn display(spec: &str) -> String {
    let primary = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    spec.split('+')
        .map(|p| if p == "Cmd" { primary } else { p })
        .collect::<Vec<_>>()
        .join("+")
}

fn matches(s: &Spec, k: &Key) -> bool {
    // On macOS Command and Control are different keys: "Cmd" bindings need Command and
    // "Ctrl" bindings need Control (elsewhere both mean Control).
    if cfg!(target_os = "macos") && s.ctrl && s.literal_ctrl == k.sup {
        return false;
    }
    same_key(
        s,
        &Spec {
            code: k.code,
            ctrl: k.ctrl,
            // The Command/Control distinction was checked above.
            literal_ctrl: s.literal_ctrl,
            shift: k.shift,
            alt: k.alt,
        },
    )
}

/// The key a typed character corresponds to (for single-letter sidebar commands).
pub fn key_for_char(c: char) -> Key {
    Key {
        code: KeyCode::Char(c.to_ascii_lowercase()),
        ctrl: false,
        alt: false,
        shift: c.is_ascii_uppercase(),
        sup: false,
    }
}

/// The key event a binding stands for (used to replay native-menu shortcuts).
pub fn key_of(spec: &Spec) -> Key {
    Key {
        code: spec.code,
        ctrl: spec.ctrl,
        alt: spec.alt,
        shift: spec.shift,
        sup: spec.ctrl && !spec.literal_ctrl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Command::*;
    use std::collections::HashSet;

    #[test]
    fn defaults_parse_without_conflicts() {
        let c = Config::builtin();
        let (km, errs) = Keymap::from_config(&c);
        assert!(errs.is_empty(), "{errs:?}");
        // No key is bound twice in the same context in the shipped keymap.
        let mut seen = HashSet::new();
        for kb in &c.keys {
            if !kb.platforms.is_empty() && !kb.platforms.iter().any(|p| p == platform()) {
                continue;
            }
            let s = parse(&kb.keys).unwrap();
            // On macOS, Cmd+X and Ctrl+X are different keys.
            let lit = cfg!(target_os = "macos") && s.literal_ctrl;
            let key = (
                format!("{:?}{}{}{}{}", s.code, s.ctrl, s.shift, s.alt, lit),
                kb.context.clone(),
            );
            assert!(seen.insert(key), "{} bound twice", kb.keys);
        }
        let k = Key {
            code: KeyCode::Char('p'),
            ctrl: true,
            alt: false,
            shift: true,
            sup: true,
        };
        assert_eq!(km.lookup(k, Ctx::Global), Some(ShowCommands));
        assert_eq!(
            km.lookup(key_for_char('A'), Ctx::Sidebar),
            Some(SidebarNewFolder)
        );
        assert_eq!(
            km.lookup(key_for_char('a'), Ctx::Sidebar),
            Some(SidebarNewFile)
        );
        assert!(km.shortcut(Save).ends_with("+S"));
        assert_eq!(
            km.shortcut(SidebarCopy),
            display("Cmd+C"),
            "menus prefer modifier keys"
        );
        assert!(parse("Hyper+X").is_none());
    }

    #[test]
    fn user_overrides_and_unbinds() {
        let mut c = Config::builtin();
        c.merge_file(
            "keymap.toml",
            r#"
            [[keys]]
            keys = "Cmd+P"
            command = "show_keymap"
            [[keys]]
            keys = "F12"
            command = ""
            context = "editor"
            [[keys]]
            keys = "Cmd+K"
            command = "no_such_command"
            "#,
        )
        .unwrap();
        let (km, errs) = Keymap::from_config(&c);
        assert_eq!(errs.len(), 1, "{errs:?}");
        let k = Key {
            code: KeyCode::Char('p'),
            ctrl: true,
            alt: false,
            shift: false,
            // The Command key on macOS.
            sup: cfg!(target_os = "macos"),
        };
        assert_eq!(km.lookup(k, Ctx::Global), Some(ShowKeymap));
        assert_eq!(km.lookup(Key::new(KeyCode::F(12)), Ctx::Editor), None);
    }
}
