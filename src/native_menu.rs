//! The macOS system menu bar, built from the same menu data as the in-window menu bar.
//! Choosing an item (or pressing its shortcut) sends the command id back to the app.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::app::{App, AppEvent, EventTx};
use crate::commands::{self, Command};
use crate::keymap::Spec;
use crate::keys::KeyCode;

pub struct NativeMenu {
    _menu: Menu,
    items: Vec<(MenuItem, Command, bool)>,
    _repeats: Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,
}

/// Held-down shortcuts skip the menu bar. macOS runs every Cmd key press through the
/// menus first; a menu item's shortcut flashes its menu title and only then reaches the
/// app, which slows key repeat to a crawl (holding Cmd+X to cut line after line lagged).
/// The first press still goes through the menu (the flash shows what ran); repeats go
/// straight to the window, like any other key, and the app's key bindings run them.
fn route_repeats_to_window() -> Option<objc2::rc::Retained<objc2::runtime::AnyObject>> {
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
    let mtm = objc2::MainThreadMarker::new()?;
    let block = block2::RcBlock::new(move |ev: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands the monitor a valid event, on the main thread.
        let e = unsafe { ev.as_ref() };
        if e.isARepeat()
            && e.modifierFlags().contains(NSEventModifierFlags::Command)
            && let Some(w) = e.window(mtm)
        {
            w.sendEvent(e);
            return std::ptr::null_mut();
        }
        ev.as_ptr()
    });
    // SAFETY: the block returns the event it was given or null.
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block) }
}

fn code(k: KeyCode) -> Option<Code> {
    let name = match k {
        KeyCode::Char(c) if c.is_ascii_lowercase() => format!("Key{}", c.to_ascii_uppercase()),
        KeyCode::Char(c) if c.is_ascii_digit() => format!("Digit{c}"),
        KeyCode::Char(c) => match c {
            ',' => "Comma",
            '.' => "Period",
            '/' => "Slash",
            '[' => "BracketLeft",
            ']' => "BracketRight",
            '-' => "Minus",
            '=' => "Equal",
            ';' => "Semicolon",
            '\'' => "Quote",
            '`' => "Backquote",
            '\\' => "Backslash",
            ' ' => "Space",
            _ => return None,
        }
        .to_string(),
        KeyCode::Enter => "Enter".into(),
        KeyCode::Esc => "Escape".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::Backspace => "Backspace".into(),
        KeyCode::Delete => "Delete".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Left => "ArrowLeft".into(),
        KeyCode::Right => "ArrowRight".into(),
        KeyCode::Up => "ArrowUp".into(),
        KeyCode::Down => "ArrowDown".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Menu => return None,
    };
    name.parse().ok()
}

fn accelerator(s: &Spec) -> Option<Accelerator> {
    let mut m = Modifiers::empty();
    if s.ctrl {
        m |= if s.literal_ctrl {
            Modifiers::CONTROL
        } else {
            Modifiers::META
        };
    }
    if s.shift {
        m |= Modifiers::SHIFT;
    }
    if s.alt {
        m |= Modifiers::ALT;
    }
    Some(Accelerator::new(m, code(s.code)?))
}

impl NativeMenu {
    pub fn install(app: &App, tx: EventTx) -> Option<NativeMenu> {
        let menu = Menu::new();
        let mut items = vec![];
        let accels = crate::menu::native_accelerators(&app.menus, &app.keymap);
        let mut item = |cmd: Command, label: &str| {
            let acc = accels.get(&cmd).and_then(accelerator);
            let enabled = app.enabled(cmd);
            let mi = MenuItem::with_id(commands::id(cmd), label, enabled, acc);
            items.push((mi.clone(), cmd, enabled));
            mi
        };
        // Application menu, macOS style.
        let app_menu = Submenu::new("FarSub", true);
        let about = PredefinedMenuItem::about(
            None,
            Some(AboutMetadata {
                name: Some("FarSub".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Default::default()
            }),
        );
        let settings = item(Command::OpenSettings, "Settings…");
        let servers = item(Command::ManageServers, "Language Servers…");
        let quit = item(Command::Quit, "Quit FarSub");
        app_menu
            .append_items(&[
                &about,
                &PredefinedMenuItem::separator(),
                &settings,
                &servers,
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::services(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &PredefinedMenuItem::separator(),
                &quit,
            ])
            .ok()?;
        menu.append(&app_menu).ok()?;
        let mut help = None;
        for m in &app.menus {
            let sub = Submenu::new(&m.title, true);
            for it in &m.items {
                match it {
                    Some(i) if matches!(i.cmd, Command::Quit | Command::OpenSettings) => {}
                    Some(i) => {
                        sub.append(&item(i.cmd, &i.label)).ok()?;
                    }
                    None => {
                        sub.append(&PredefinedMenuItem::separator()).ok()?;
                    }
                }
            }
            menu.append(&sub).ok()?;
            if m.title == "Help" {
                help = Some(sub);
            }
        }
        let window = Submenu::new("Window", true);
        window
            .append_items(&[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::fullscreen(None),
                &PredefinedMenuItem::separator(),
                &PredefinedMenuItem::bring_all_to_front(None),
            ])
            .ok()?;
        menu.append(&window).ok()?;
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        if let Some(h) = help {
            h.set_as_help_menu_for_nsapp();
        }
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            tx.send(AppEvent::Menu(e.id.0.clone()));
        }));
        Some(NativeMenu {
            _menu: menu,
            items,
            _repeats: route_repeats_to_window(),
        })
    }

    /// Grey out items that can't run right now (only touches items whose state changed).
    pub fn sync(&mut self, app: &App) {
        for (mi, cmd, enabled) in &mut self.items {
            let now = app.enabled(*cmd);
            if now != *enabled {
                mi.set_enabled(now);
                *enabled = now;
            }
        }
    }
}
