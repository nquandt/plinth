//! An app can set the values of the color tokens in the `[theme]` table of
//! `plinth.toml` (`docs/ui.md`, "Theming an app"). Renders `examples/themed`
//! on the desktop host: the window takes the brand colors in light and dark,
//! high contrast ignores them, and an override that fails the contrast guard
//! is ignored with the host theme as the result. Writes
//! `target/shots/theme-*.png` for a visual check.

use gpui::{AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, WindowHandle, px, size};
use plinth_compiler::driver::DiskFs;
use plinth_package::ProjectConfig;
use plinth_package::theme::{ThemeColor, ThemeOverride};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot, SystemSettings, Tokens};
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

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/themed")
}

fn compile() -> Vec<u8> {
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root: root() }, &[]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("examples/themed has errors:\n{}", diags.join("\n"))).component
}

fn theme() -> ThemeOverride {
    let text = std::fs::read_to_string(root().join("plinth.toml")).unwrap();
    ProjectConfig::parse(&text).unwrap().theme.expect("examples/themed has a [theme]")
}

fn headless() -> HeadlessAppContext {
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
        gpui_wgpu::WgpuHeadlessRenderer::new()
            .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
            .map_err(|e| log::error!("no headless renderer: {e:#}"))
            .ok()
    });
    cx.update(plinth_ui::init);
    cx
}

fn open(cx: &mut HeadlessAppContext, component: &[u8], theme: Option<&ThemeOverride>) -> (WindowHandle<PlinthRoot>, bool) {
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(component, Limits::default()).unwrap();
    let commits = guest.init(&[]).unwrap();
    let port = Box::new(WasmGuest { guest, _runner: runner });
    let applied = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = applied.clone();
    let theme = theme.cloned();
    let window = cx
        .open_window(size(px(900.), px(760.)), move |_, cx| {
            cx.new(|cx| {
                let mut root = PlinthRoot::new(port, commits, "teal", cx);
                flag.set(root.set_theme(theme.as_ref()));
                root
            })
        })
        .unwrap();
    cx.run_until_parked();
    (window, applied.get())
}

struct Shot {
    width: u32,
    raw: Vec<u8>,
}

impl Shot {
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.raw[i], self.raw[i + 1], self.raw[i + 2]]
    }

    fn share_of(&self, color: [u8; 3]) -> f64 {
        let n = self.raw.chunks_exact(4).filter(|p| close([p[0], p[1], p[2]], color)).count();
        n as f64 / (self.raw.len() / 4) as f64
    }
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

fn as_rgb(color: gpui::Hsla) -> [u8; 3] {
    let c = gpui::hsla_to_rgba(color).color;
    [c.red, c.green, c.blue].map(|v| (v * 255.0).round() as u8)
}

fn shot(cx: &mut HeadlessAppContext, window: WindowHandle<PlinthRoot>, name: &str) -> Shot {
    cx.update_window(window.into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    })
    .unwrap();
    let image = cx.capture_screenshot(window.into()).expect("render");
    let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&shots).unwrap();
    image.save(shots.join(format!("theme-{name}.png"))).expect("write the screenshot");
    Shot { width: image.width(), raw: image.into_raw() }
}

fn set(cx: &mut HeadlessAppContext, settings: SystemSettings) {
    cx.update(|cx| plinth_host_desktop::system_settings::apply(settings, cx));
    cx.run_until_parked();
}

fn close_window(cx: &mut HeadlessAppContext, window: WindowHandle<PlinthRoot>) {
    cx.update(|cx| window.update(cx, |_, window, _| window.remove_window())).unwrap();
    cx.run_until_parked();
}

/// The window background is the brand background in light and dark, and
/// the brand accent is painted (the primary button).
#[test]
fn the_window_takes_the_brand_colors() {
    let component = compile();
    let theme = theme();
    let mut cx = headless();
    let base = SystemSettings::default();
    let resolved = theme.resolve(None).unwrap();
    for (name, dark) in [("light", false), ("dark", true)] {
        set(&mut cx, SystemSettings { dark: Some(dark), ..base });
        let (window, applied) = open(&mut cx, &component, Some(&theme));
        assert!(applied, "{name}: the theme applies");
        let image = shot(&mut cx, window, name);
        let tokens = Tokens::from_palette(dark, resolved.palette(dark));
        let corner = image.pixel(image.width - 3, (image.raw.len() as u32 / 4 / image.width) - 3);
        assert!(close(corner, as_rgb(tokens.background)), "{name}: the background is {corner:?}, not {:?}", as_rgb(tokens.background));
        assert!(image.share_of(as_rgb(tokens.accent)) > 0.001, "{name}: no accent pixels");
        assert!(image.share_of(as_rgb(tokens.text)) > 0.0001, "{name}: no text pixels");
        close_window(&mut cx, window);
    }
    // The host theme has other colors, so the test above means something.
    let host = Tokens::new(gpui::WindowAppearance::Light, "teal");
    assert!(!close(as_rgb(host.background), as_rgb(Tokens::from_palette(false, resolved.palette(false)).background)));
}

/// High contrast ignores the app theme: the host high-contrast tokens win.
#[test]
fn high_contrast_ignores_the_app_theme() {
    let component = compile();
    let theme = theme();
    let mut cx = headless();
    for (name, dark) in [("hc-light", false), ("hc-dark", true)] {
        set(&mut cx, SystemSettings { dark: Some(dark), high_contrast: true, ..SystemSettings::default() });
        let (window, applied) = open(&mut cx, &component, Some(&theme));
        assert!(applied, "the override is valid; high contrast ignores it when it draws");
        let image = shot(&mut cx, window, name);
        let hc = Tokens::high_contrast(dark, None, "teal");
        let corner = image.pixel(image.width - 3, (image.raw.len() as u32 / 4 / image.width) - 3);
        assert!(close(corner, as_rgb(hc.background)), "{name}: the background is {corner:?}, not {:?}", as_rgb(hc.background));
        assert_eq!(image.share_of([0xff, 0xf8, 0xee]), 0.0, "{name}: brand background pixels");
        close_window(&mut cx, window);
    }
}

/// The host-owned banner keeps the host colors inside a themed window.
#[test]
fn the_stopped_banner_keeps_the_host_theme() {
    let component = compile();
    let theme = theme();
    let mut cx = headless();
    set(&mut cx, SystemSettings::default());
    let runner = Runner::new().unwrap();
    let guest = runner.load(&component, Limits::default()).unwrap();
    let port = Box::new(WasmGuest { guest, _runner: runner });
    let window = cx
        .open_window(size(px(900.), px(760.)), move |_, cx| {
            cx.new(|cx| {
                let mut root = PlinthRoot::stopped(port, "the app stopped for this test".into(), "teal", cx);
                root.set_theme(Some(&theme));
                root
            })
        })
        .unwrap();
    cx.run_until_parked();
    let image = shot(&mut cx, window, "banner");
    let host = Tokens::new(gpui::WindowAppearance::Light, "teal");
    // The page has the brand background, and the banner has the host danger color, not the brand one.
    let corner = image.pixel(image.width - 3, (image.raw.len() as u32 / 4 / image.width) - 3);
    assert!(close(corner, [0xff, 0xf8, 0xee]), "the window takes the app theme: {corner:?}");
    assert!(image.share_of(as_rgb(host.danger)) > 0.005, "the banner has the host danger color");
    assert_eq!(image.share_of([0xa4, 0x22, 0x1a]), 0.0, "the brand danger color is not in the banner");
    close_window(&mut cx, window);
}

/// An override that fails the contrast guard is ignored; the window keeps
/// the host theme (the host never crashes on package data).
#[test]
fn a_failing_theme_is_ignored() {
    let component = compile();
    let mut bad = ThemeOverride::default();
    bad.0.insert("text".into(), ThemeColor::Same("#eeeeee".into()));
    let mut cx = headless();
    set(&mut cx, SystemSettings::default());
    let (window, applied) = open(&mut cx, &component, Some(&bad));
    assert!(!applied);
    let image = shot(&mut cx, window, "ignored");
    let host = Tokens::new(gpui::WindowAppearance::Light, "teal");
    let corner = image.pixel(image.width - 3, (image.raw.len() as u32 / 4 / image.width) - 3);
    assert!(close(corner, as_rgb(host.background)), "the background is {corner:?}");
    close_window(&mut cx, window);
}
