//! Renders the `counter` example headless and checks the AccessKit roles and
//! names the host produces for its controls (SPEC.md §6.1 item 3, §9.3).

use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, px, size};
use plinth_compiler::driver::DiskFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::PathBuf;

struct WasmGuest {
    guest: Guest,
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> anyhow::Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

fn build(name: &str) -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    let fs = DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("{name} has errors:\n{}", diags.join("\n")))
}

/// Collects `(role, label)` for every node in the tree, labels taken from
/// `Node::label` falling back to `Node::value` (text content).
fn nodes(tree: &gpui::accesskit::TreeUpdate) -> Vec<(gpui::accesskit::Role, String)> {
    tree.nodes
        .iter()
        .map(|(_, n)| (n.role(), n.label().or(n.value()).unwrap_or("").to_owned()))
        .collect()
}

#[test]
fn counter_screen_has_expected_a11y_roles() {
    let artifact = build("counter");

    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
            .map_err(|e| log::error!("no headless renderer: {e:#}"))
            .ok()
    });
    cx.update(plinth_ui::init);

    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let commits = guest.init(&[]).unwrap();
    let port = Box::new(WasmGuest { guest, _runner: runner });
    let window =
        cx.open_window(size(px(900.), px(760.)), move |_, cx| cx.new(|cx| PlinthRoot::new(port, commits, "teal", cx))).unwrap();

    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true))).unwrap();
    cx.run_until_parked();

    let tree = cx.update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned())).unwrap();
    let tree = tree.expect("a11y tree was built");
    let found = nodes(&tree);

    let has = |role: gpui::accesskit::Role, name: &str| found.iter().any(|(r, l)| *r == role && l.contains(name));

    assert!(has(gpui::accesskit::Role::Heading, "Counter"), "screen title should be a heading: {found:#?}");
    assert!(has(gpui::accesskit::Role::Button, "Increment"), "Increment button should have role Button: {found:#?}");
    assert!(has(gpui::accesskit::Role::Button, "Decrement"), "Decrement button should have role Button: {found:#?}");
}
