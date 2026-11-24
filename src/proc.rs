//! Starting helper programs (language servers, installers, compilers, git).

use std::process::Command;

/// On Windows FarSub is a windowed app, and every console program it starts gets a
/// console window of its own unless told not to: a terminal popping up for each language
/// server, install step or compile. This keeps them hidden; elsewhere it does nothing.
pub fn hidden(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}
