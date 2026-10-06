//! `plinth-host <app.plnt | app.wasm>`: runs one app in a window.

use anyhow::{Context as _, Result};
use plinth_host_desktop::{HostApp, init_logging, run};
use std::path::PathBuf;

fn main() -> Result<()> {
    init_logging();
    let path = std::env::args().nth(1).map(PathBuf::from).context("usage: plinth-host <app.plnt | app.wasm>")?;
    run(HostApp::load(&path)?, None)
}
