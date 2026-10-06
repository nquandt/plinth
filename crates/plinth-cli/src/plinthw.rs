#![windows_subsystem = "windows"]
//! `plinthw <arguments>`: runs `plinth <arguments>` (the `plinth` program in
//! the same folder) with no console window, and exits with its exit code.
//!
//! `plinth` is a console program, so Windows shows a console window for a
//! moment when a shortcut or a `plinth://` link starts it. This program is
//! for the Windows subsystem, so Windows shows no console for it, and it
//! starts `plinth` with `CREATE_NO_WINDOW` (`docs/HUB.md` §10). The app
//! window of `plinth hub open|run` opens as usual. On other platforms it
//! only forwards the call.

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let Ok(me) = std::env::current_exe() else { return ExitCode::FAILURE };
    let cli = me.with_file_name(format!("plinth{}", std::env::consts::EXE_SUFFIX));
    let mut command = Command::new(cli);
    command.args(std::env::args_os().skip(1));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(_) => ExitCode::FAILURE,
    }
}
