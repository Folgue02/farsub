//! Backend-independent key representation. Cmd and Ctrl are both treated as the "primary"
//! modifier (`ctrl`), so every shortcut works with either; `sup` is set only for Cmd so
//! macOS motions like Cmd+Left (line start) can be told apart from Ctrl+Left (word left).

use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    /// Ctrl or Cmd.
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd specifically.
    pub sup: bool,
}

impl Key {
    pub fn new(code: KeyCode) -> Key {
        Key {
            code,
            ctrl: false,
            alt: false,
            shift: false,
            sup: false,
        }
    }

    #[allow(dead_code)]
    pub fn ctrl(c: char) -> Key {
        Key {
            ctrl: true,
            ..Key::new(KeyCode::Char(c))
        }
    }

    pub fn from_egui(key: egui::Key, m: egui::Modifiers) -> Option<Key> {
        use egui::Key as E;
        let code = match key {
            E::ArrowDown => KeyCode::Down,
            E::ArrowUp => KeyCode::Up,
            E::ArrowLeft => KeyCode::Left,
            E::ArrowRight => KeyCode::Right,
            E::Escape => KeyCode::Esc,
            E::Tab => KeyCode::Tab,
            E::Backspace => KeyCode::Backspace,
            E::Enter => KeyCode::Enter,
            E::Insert => KeyCode::Insert,
            E::Delete => KeyCode::Delete,
            E::Home => KeyCode::Home,
            E::End => KeyCode::End,
            E::PageUp => KeyCode::PageUp,
            E::PageDown => KeyCode::PageDown,
            E::Space => KeyCode::Char(' '),
            E::Colon => KeyCode::Char(':'),
            E::Comma => KeyCode::Char(','),
            E::Backslash => KeyCode::Char('\\'),
            E::Slash | E::Questionmark => KeyCode::Char('/'),
            E::Pipe => KeyCode::Char('|'),
            E::Exclamationmark => KeyCode::Char('!'),
            E::OpenBracket | E::OpenCurlyBracket => KeyCode::Char('['),
            E::CloseBracket | E::CloseCurlyBracket => KeyCode::Char(']'),
            E::Backtick => KeyCode::Char('`'),
            E::Minus => KeyCode::Char('-'),
            E::Period => KeyCode::Char('.'),
            E::Plus | E::Equals => KeyCode::Char('='),
            E::Semicolon => KeyCode::Char(';'),
            E::Quote => KeyCode::Char('\''),
            E::F1 => KeyCode::F(1),
            E::F2 => KeyCode::F(2),
            E::F3 => KeyCode::F(3),
            E::F4 => KeyCode::F(4),
            E::F5 => KeyCode::F(5),
            E::F6 => KeyCode::F(6),
            E::F7 => KeyCode::F(7),
            E::F8 => KeyCode::F(8),
            E::F9 => KeyCode::F(9),
            E::F10 => KeyCode::F(10),
            E::F11 => KeyCode::F(11),
            E::F12 => KeyCode::F(12),
            other => {
                let name = other.name();
                let mut cs = name.chars();
                match (cs.next(), cs.next()) {
                    (Some(c), None) if c.is_ascii_alphanumeric() => {
                        KeyCode::Char(c.to_ascii_lowercase())
                    }
                    _ => return None,
                }
            }
        };
        Some(Key {
            code,
            ctrl: m.ctrl || m.mac_cmd || m.command,
            alt: m.alt,
            shift: m.shift,
            sup: m.mac_cmd,
        })
    }

    /// Keys that produce text are delivered through text events instead, unless a
    /// shortcut modifier is held. (Alt+letter is tried as a shortcut first; if nothing
    /// handles it, the text event it generates is inserted as usual.)
    pub fn is_text_key(&self) -> bool {
        matches!(self.code, KeyCode::Char(_)) && !self.ctrl && !self.alt
    }
}
