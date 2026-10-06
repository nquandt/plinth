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

/// Serves `folder` as a static registry on `127.0.0.1:<port>`; with `web`,
/// also the web App Hub (`docs/web-hub.md`). Blocks forever (`Ctrl+C` to
/// stop).
pub fn serve(folder: &Path, port: u16, web: bool) -> Result<()> {
    let listener = plinth_registry::serve::bind(port)?;
    let addr = listener.local_addr()?;
    println!("serving {} on http://{addr}", folder.display());
    if web {
        println!("web App Hub: http://{addr}/ (Ctrl+C to stop)");
    }
    // The Hub keys that the web App Hub trusts: the same variable as the
    // desktop host (`docs/HUB.md` §4.1), comma-separated key ids.
    let trusted_keys: Vec<String> = std::env::var("PLINTH_HUB_TRUSTED_KEYS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
        .collect();
    if web && !trusted_keys.is_empty() {
        println!("trusted Hub keys: {}", trusted_keys.join(", "));
    }
    let options = plinth_registry::serve::Options { web, trusted_keys };
    plinth_registry::serve::accept_loop_with(listener, folder, options).with_context(|| format!("serve {}", folder.display()))
}
