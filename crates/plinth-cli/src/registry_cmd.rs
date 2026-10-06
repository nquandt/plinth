//! `plinth registry build|serve` (`docs/REGISTRY.md` §8): a thin CLI over
//! `plinth-registry`'s generator and static HTTP server.

use anyhow::{Context as _, Result};
use std::path::Path;

pub fn build(folder: &Path, with_core: bool) -> Result<()> {
    let report = plinth_registry::build::build(folder, &plinth_registry::build::Options { with_core })?;
    println!("built the registry at {}", folder.display());
    for (id, version) in &report.added {
        println!("  added {id}@{version}");
    }
    for (id, version) in &report.unchanged {
        println!("  unchanged {id}@{version}");
    }
    Ok(())
}

/// Serves `folder` as a static registry on `127.0.0.1:<port>`. Blocks
/// forever (`Ctrl+C` to stop).
pub fn serve(folder: &Path, port: u16) -> Result<()> {
    let listener = plinth_registry::serve::bind(port)?;
    let addr = listener.local_addr()?;
    println!("serving {} on http://{addr}", folder.display());
    plinth_registry::serve::accept_loop(listener, folder).with_context(|| format!("serve {}", folder.display()))
}
