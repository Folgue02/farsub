//! Integrated terminal (VS Code style): a shell on a pseudo-terminal, parsed by `vt100`.
//! A reader thread feeds the emulator; the UI draws `screen()` and sends keys as bytes.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::app::EventTx;
use crate::keys::{Key, KeyCode};

/// Captures OSC window titles set by the shell/programs.
#[derive(Default)]
pub struct TermCallbacks {
    pub title: String,
    /// Answers to the program's status requests, sent back to it after each read.
    reply: Vec<u8>,
}

impl vt100::Callbacks for TermCallbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = String::from_utf8_lossy(title).into_owned();
    }

    /// Device status requests, which vt100 leaves to us. Windows' pseudo-console asks
    /// for the cursor position (`ESC[6n`) when it starts and waits for the answer before
    /// the shell shows anything; programs like fzf ask too.
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if c != 'n' || i1.is_some() {
            return;
        }
        match params.first().and_then(|p| p.first()) {
            Some(6) => {
                let (row, col) = screen.cursor_position();
                self.reply
                    .extend(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
            }
            Some(5) => self.reply.extend(b"\x1b[0n"),
            _ => {}
        }
    }
}

type Parser = vt100::Parser<TermCallbacks>;

/// A selection in screen coordinates: (row, col) anchor and head.
pub type TermSel = ((u16, u16), (u16, u16));

pub struct Terminal {
    pub id: usize,
    /// "zsh", "bash", … (the shell's name; programs can set a title too).
    pub shell: String,
    /// The folder it started in.
    pub cwd: std::path::PathBuf,
    parser: Arc<Mutex<Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    pub exited: Arc<AtomicBool>,
    /// The shell's exit code once it has exited.
    pub exit_code: Arc<Mutex<Option<u32>>>,
    pub size: (u16, u16),
    pub sel: Option<TermSel>,
}

const SCROLLBACK: usize = 5000;

fn nz(n: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(n.max(1)).unwrap()
}

impl Terminal {
    pub fn spawn(
        id: usize,
        cwd: &Path,
        rows: u16,
        cols: u16,
        shell: Option<&str>,
        tx: EventTx,
    ) -> Result<Terminal, String> {
        let (rows, cols) = (rows.max(2), cols.max(10));
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        // Windows: PowerShell by default (PowerShell 7 when installed), not cmd.exe.
        let windows_default = (shell.is_none() && cfg!(windows)).then(|| {
            crate::lsp::find_server("pwsh")
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "powershell.exe".into())
        });
        let shell = shell.or(windows_default.as_deref());
        let mut cmd = match shell {
            Some(sh) => CommandBuilder::new(sh),
            None => CommandBuilder::new_default_prog(),
        };
        if cwd.is_dir() {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "farsub");
        let shell = shell
            .map(String::from)
            .or_else(|| std::env::var("SHELL").ok())
            .and_then(|s| {
                Path::new(&s)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell".into()
                } else {
                    "sh".into()
                }
            });
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        let killer = child.clone_killer();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
        let parser = Arc::new(Mutex::new(Parser::new_with_callbacks(
            nz(rows),
            nz(cols),
            SCROLLBACK,
            TermCallbacks::default(),
        )));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(Mutex::new(None));
        // Wait on the shell process itself: reaching EOF on the PTY isn't reliable on every
        // platform, the process exiting is.
        {
            let exited = exited.clone();
            let exit_code = exit_code.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let code = child.wait().map(|s| s.exit_code()).unwrap_or(1);
                *exit_code.lock().unwrap() = Some(code);
                exited.store(true, Ordering::SeqCst);
                tx.wake();
            });
        }
        {
            let parser = parser.clone();
            let exited = exited.clone();
            let writer = writer.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let reply = {
                                let mut p = parser.lock().unwrap();
                                p.process(&buf[..n]);
                                std::mem::take(&mut p.callbacks_mut().reply)
                            };
                            if !reply.is_empty() {
                                let mut w = writer.lock().unwrap();
                                let _ = w.write_all(&reply);
                                let _ = w.flush();
                            }
                            tx.wake();
                        }
                    }
                }
                exited.store(true, Ordering::SeqCst);
                tx.wake();
            });
        }
        crate::log::info("terminal", format!("started {shell} in {}", cwd.display()));
        Ok(Terminal {
            id,
            shell,
            cwd: cwd.to_path_buf(),
            parser,
            writer,
            master: pair.master,
            killer,
            exited,
            exit_code,
            size: (rows, cols),
            sel: None,
        })
    }

    pub fn is_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn title(&self) -> String {
        let t = self.parser.lock().unwrap().callbacks().title.clone();
        if t.trim().is_empty() {
            self.shell.clone()
        } else {
            t
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if self.is_exited() {
            return;
        }
        // Typing jumps back to the live screen, like other terminals.
        self.parser.lock().unwrap().screen_mut().set_scrollback(0);
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self.parser.lock().unwrap().screen().bracketed_paste();
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = (rows.max(2), cols.max(10));
        if (rows, cols) == self.size {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.parser.lock().unwrap().set_size(nz(rows), nz(cols));
        self.sel = None;
    }

    /// Scroll back through history (positive = older lines).
    pub fn scroll(&mut self, lines: isize) {
        let mut p = self.parser.lock().unwrap();
        let s = p.screen().scrollback() as isize;
        p.screen_mut().set_scrollback((s + lines).max(0) as usize);
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(self.parser.lock().unwrap().screen())
    }

    pub fn application_cursor(&self) -> bool {
        self.parser.lock().unwrap().screen().application_cursor()
    }

    pub fn selection_text(&self) -> Option<String> {
        let ((r0, c0), (r1, c1)) = self.sel?;
        let (a, b) = if (r0, c0) <= (r1, c1) {
            ((r0, c0), (r1, c1))
        } else {
            ((r1, c1), (r0, c0))
        };
        let t = self.with_screen(|s| s.contents_between(a.0, a.1, b.0, b.1 + 1));
        (!t.is_empty()).then_some(t)
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Bytes a key sends to the shell (xterm conventions). `None` for keys with no meaning.
pub fn key_bytes(k: Key, app_cursor: bool) -> Option<Vec<u8>> {
    let mods = 1 + (k.shift as u8) + 2 * (k.alt as u8) + 4 * ((k.ctrl && !k.sup) as u8);
    let csi = |final_: char| -> Vec<u8> {
        if mods > 1 {
            format!("\x1b[1;{mods}{final_}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_}").into_bytes()
        } else {
            format!("\x1b[{final_}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if mods > 1 {
            format!("\x1b[{n};{mods}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    Some(match k.code {
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab if k.shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Backspace if k.alt || (k.ctrl && !k.sup) => b"\x17".to_vec(),
        KeyCode::Backspace => b"\x7f".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => csi('A'),
        KeyCode::Down => csi('B'),
        KeyCode::Right if k.alt && mods == 3 => b"\x1bf".to_vec(),
        KeyCode::Left if k.alt && mods == 3 => b"\x1bb".to_vec(),
        KeyCode::Right => csi('C'),
        KeyCode::Left => csi('D'),
        KeyCode::Home => csi('H'),
        KeyCode::End => csi('F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        KeyCode::F(n) => {
            let code = match n {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => return None,
            };
            tilde(code)
        }
        // Ctrl+letter → control character (Ctrl+C = 0x03 …); Alt+char → ESC prefix.
        KeyCode::Char(c) if k.ctrl && !k.sup => {
            let b = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                ' ' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '6' => 0x1e,
                '/' | '-' | '7' => 0x1f,
                _ => return None,
            };
            vec![b]
        }
        KeyCode::Char(c) if k.alt => {
            let mut v = vec![0x1b];
            let mut s = [0u8; 4];
            v.extend_from_slice(c.encode_utf8(&mut s).as_bytes());
            v
        }
        _ => return None,
    })
}

/// An xterm 256-color palette entry.
pub fn palette(i: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0x1e, 0x1e, 0x1e),
        (0xf4, 0x47, 0x47),
        (0x6a, 0x99, 0x55),
        (0xd7, 0xba, 0x7d),
        (0x56, 0x9c, 0xd6),
        (0xc5, 0x86, 0xc0),
        (0x4e, 0xc9, 0xb0),
        (0xd4, 0xd4, 0xd4),
        (0x80, 0x80, 0x80),
        (0xf4, 0x87, 0x71),
        (0xb5, 0xce, 0xa8),
        (0xf9, 0xe2, 0x7d),
        (0x9c, 0xdc, 0xfe),
        (0xd6, 0x9d, 0xd8),
        (0x7f, 0xe0, 0xd0),
        (0xff, 0xff, 0xff),
    ];
    match i {
        0..=15 => BASE[i as usize],
        16..=231 => {
            let i = i - 16;
            let v = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            (v(i / 36), v((i / 6) % 6), v(i % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            (g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_to_bytes() {
        let k = |code, ctrl, alt, shift| Key {
            code,
            ctrl,
            alt,
            shift,
            sup: false,
        };
        assert_eq!(
            key_bytes(k(KeyCode::Char('c'), true, false, false), false),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes(k(KeyCode::Up, false, false, false), false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes(k(KeyCode::Up, false, false, false), true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(k(KeyCode::Left, true, false, false), false),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            key_bytes(k(KeyCode::Left, false, true, false), false),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(
            key_bytes(k(KeyCode::Enter, false, false, false), false),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            key_bytes(k(KeyCode::Char('x'), false, true, false), false),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            key_bytes(k(KeyCode::F(5), false, false, false), false),
            Some(b"\x1b[15~".to_vec())
        );
        // Cmd (sup) combos are app shortcuts, never control bytes.
        assert_eq!(
            key_bytes(
                Key {
                    code: KeyCode::Char('c'),
                    ctrl: true,
                    alt: false,
                    shift: false,
                    sup: true
                },
                false
            ),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn runs_a_shell() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut t = Terminal::spawn(
            1,
            &std::env::temp_dir(),
            10,
            60,
            Some("/bin/sh"),
            EventTx::new(tx),
        )
        .unwrap();
        t.write(b"echo farsub-term-$((40+2))\r");
        let ok = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            t.with_screen(|s| s.contents()).contains("farsub-term-42")
        });
        assert!(ok, "{}", t.with_screen(|s| s.contents()));
        t.resize(20, 100);
        assert_eq!(t.with_screen(|s| s.size()).1.get(), 100);
        t.write(b"exit\r");
        assert!((0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            t.is_exited()
        }));
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    /// Cursor-position and status requests get answered (Windows' pseudo-console waits
    /// for the first one before the shell appears).
    #[test]
    fn answers_device_status_requests() {
        let mut p = Parser::new_with_callbacks(nz(24), nz(80), 0, TermCallbacks::default());
        p.process(b"\r\nab\x1b[6n");
        assert_eq!(std::mem::take(&mut p.callbacks_mut().reply), b"\x1b[2;3R");
        p.process(b"\x1b[5n");
        assert_eq!(std::mem::take(&mut p.callbacks_mut().reply), b"\x1b[0n");
        p.process(b"\x1b[?6n\x1b[2J");
        assert!(p.callbacks().reply.is_empty());
    }
}
