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
pub mod split;
pub mod tir;
pub mod types;

use driver::{FileSystem, Frontend};

/// The result of a successful build.
pub struct Artifact {
    /// The app module: only the app code, the `app.wasm` of a `.plnt`
    /// (SPEC.md §10.1). It names its runtime build in a custom section.
    pub app: Vec<u8>,
    /// The runtime ABI that `app` needs (`split::abi_id`), for example
    /// `plinth-abi/1`.
    pub runtime: String,
    /// The app linked into the runtime, as a `plinth:app` component. The dev
    /// host and the tests run it directly.
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
    let mut front = driver::frontend_with_capabilities(fs, capabilities);
    let Some(mut program) = front.program.take() else {
        return Ok((front, None));
    };
    let main = lower::lower(&mut program);
    let rt = link::runtime();
    let layout = link::layout(rt)?;
    let app_layout = split::app_layout();
    let code = codegen::generate(&program, &app_layout.layout, main);
    let runtime = split::abi_id();
    let app = split::encode_app(&app_layout, &code)?;
    // Link the app module the same way a host does, so each build checks
    // the load path too.
    let loaded = split::load_app(rt, &layout, &app)?;
    let core = link::link(rt, &layout, &loaded)?;
    let component = link::componentize(&core)?;
    let accent = program.accent.clone();
    Ok((front, Some(Artifact { app, runtime, core_size: core.len(), component, accent })))
}
