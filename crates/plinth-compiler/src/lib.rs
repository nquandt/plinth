//! The Plinth TS compiler (SPEC.md §5).

pub mod ast;
pub mod codegen;
pub mod check;
pub mod controls;
pub mod diag;
pub mod driver;
pub mod link;
pub mod lower;
pub mod parse;
pub mod rt_abi;
pub mod tir;
pub mod types;

use driver::{FileSystem, Frontend};

/// The result of a successful build.
pub struct Artifact {
    /// The `app.wasm` component.
    pub component: Vec<u8>,
    /// The size of the linked core module.
    pub core_size: usize,
    /// The accent color from `app({ accent })`.
    pub accent: Option<String>,
}

/// Compiles a project into an app component, with no declared
/// capabilities (SPEC.md §11). The artifact is `None` when the front end
/// reports errors.
pub fn compile(fs: &dyn FileSystem) -> anyhow::Result<(Frontend, Option<Artifact>)> {
    compile_with_capabilities(fs, &[])
}

/// Compiles a project into an app component. `capabilities` are the
/// capability names declared in the project's `plinth.toml` (SPEC.md
/// §11); a host API call that needs a capability not in this list is a
/// compile error. The artifact is `None` when the front end reports
/// errors.
pub fn compile_with_capabilities(fs: &dyn FileSystem, capabilities: &[String]) -> anyhow::Result<(Frontend, Option<Artifact>)> {
    compile_ex(fs, capabilities, false)
}

/// Like `compile_with_capabilities`, with `dev` selecting hot reload
/// support (SPEC.md §13): module-level signals get a stable key and
/// shape, and the artifact answers hot-reload snapshot requests. `plinth
/// dev` passes `true`; `plinth build`/`plinth check` pass `false`, so a
/// release artifact never contains that code (HANDOFF.md §7, size policy).
pub fn compile_ex(fs: &dyn FileSystem, capabilities: &[String], dev: bool) -> anyhow::Result<(Frontend, Option<Artifact>)> {
    let mut front = driver::frontend_with_capabilities(fs, capabilities);
    let Some(mut program) = front.program.take() else {
        return Ok((front, None));
    };
    let main = lower::lower(&mut program, dev);
    let rt = if dev { link::runtime_dev() } else { link::runtime() };
    let extra = if dev { rt_abi::DEV_FUNCTIONS } else { &[] };
    let layout = link::layout_with(rt, extra)?;
    let app = codegen::generate(&program, &layout, main);
    let core = link::link(rt, &layout, &app)?;
    let component = link::componentize(&core)?;
    let accent = program.accent.clone();
    Ok((front, Some(Artifact { core_size: core.len(), component, accent })))
}
