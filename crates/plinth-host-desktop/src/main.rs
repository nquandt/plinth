#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! `plinth-host <app.plnt | app.wasm>`: runs one app in a window. It is the
//! runner that a user installs, and the stub of a single-file export
//! (SPEC.md §10.3): with a `.plnt` payload at its end, it runs that app.

use anyhow::{Context as _, Result};
use plinth_host_desktop::{HostApp, init_logging, run, run_payload};
use std::path::PathBuf;

fn main() -> Result<()> {
    init_logging();
    let exe = std::env::current_exe()?;
    if let Some(plnt) = plinth_package::read_payload(&exe)? {
        // With the updater (`docs/UPDATES.md` §5, §6, §8).
        return run_payload(plnt, &exe);
    }
    let path = std::env::args().nth(1).map(PathBuf::from).context("usage: plinth-host <app.plnt | app.wasm>")?;
    run(HostApp::load(&path)?, None)
}
