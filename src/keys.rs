//! Key normalization. Cmd (SUPER, when the terminal reports it via the kitty keyboard
//! protocol) and Ctrl are both treated as the "primary" modifier, so Cmd+P and Ctrl+P
//! do the same thing.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    /// Ctrl or Cmd.
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd specifically (for macOS-only motions like Cmd+Left = line start).
    pub sup: bool,
}

impl Key {
    pub fn from_event(ev: &KeyEvent) -> Key {
        let m = ev.modifiers;
        let sup = m.contains(KeyModifiers::SUPER);
        let ctrl = m.contains(KeyModifiers::CONTROL) || sup;
        let alt = m.contains(KeyModifiers::ALT);
        let mut shift = m.contains(KeyModifiers::SHIFT);
        let code = match ev.code {
            KeyCode::BackTab => {
                shift = true;
                KeyCode::Tab
            }
            KeyCode::Char(c) => {
                let mut c = c;
                if (ctrl || alt) && c.is_ascii_uppercase() {
                    shift = true;
                    c = c.to_ascii_lowercase();
                }
                if ctrl {
                    // Legacy terminals encode Ctrl+/ as 0x1F and Ctrl+] as 0x1D.
                    c = match c {
                        '7' | '_' => '/',
                        '5' => ']',
                        '4' => '\\',
                        other => other,
                    };
                }
                KeyCode::Char(c)
            }
            other => other,
        };
        Key {
            code,
            ctrl,
            alt,
            shift,
            sup,
        }
    }

    /// A plain printable character (to be inserted as text).
    pub fn text_char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }
}
