//! The theme follows the system settings on the desktop (SPEC.md §6.5,
//! PRODUCTION U7): dark mode, high contrast and the text size, from the
//! `SystemSettings` global that the host keeps up to date
//! (`plinth_host_desktop::system_settings`). Renders three examples in
//! each setting, checks the colors of the screenshots and that the layout
//! holds at 200 % text, and writes `target/shots/settings-*.png` for a
//! visual check. A change of the setting draws the open window again
//! without a restart.

use gpui::accesskit::{Rect, Role};
use gpui::{
    AppContext as _, HeadlessAppContext, PlatformHeadlessRenderer, WindowAppearance, WindowHandle,
    px, size,
};
use plinth_compiler::driver::DiskFs;
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot, SystemSettings, Tokens, contrast_ratio};
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

const EXAMPLES: [&str; 3] = ["settings-gallery", "todo", "calculator"];
const ACCENT: &str = "indigo";

fn compile(name: &str) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    let (front, artifact) =
        plinth_compiler::compile_with_capabilities(&DiskFs { root }, &[]).expect("compile");
    let diags: Vec<String> = front
        .diags
        .iter()
        .map(|d| front.sources.render(d))
        .collect();
    artifact
        .unwrap_or_else(|| panic!("examples/{name} has errors:\n{}", diags.join("\n")))
        .component
}

fn headless() -> HeadlessAppContext {
    let platform = gpui_platform::current_platform(true);
    let mut cx =
        HeadlessAppContext::with_platform(platform.text_system(), std::sync::Arc::new(()), || {
            gpui_wgpu::WgpuHeadlessRenderer::new()
                .map(|r| Box::new(r) as Box<dyn PlatformHeadlessRenderer>)
                .map_err(|e| log::error!("no headless renderer: {e:#}"))
                .ok()
        });
    cx.update(plinth_ui::init);
    cx
}

fn open(cx: &mut HeadlessAppContext, component: &[u8], w: f32, h: f32) -> WindowHandle<PlinthRoot> {
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(component, Limits::default()).unwrap();
    let commits = guest.init(&[]).unwrap();
    let port = Box::new(WasmGuest {
        guest,
        _runner: runner,
    });
    let window = cx
        .open_window(size(px(w), px(h)), move |_, cx| {
            cx.new(|cx| PlinthRoot::new(port, commits, ACCENT, cx))
        })
        .unwrap();
    cx.update(|cx| window.update(cx, |_, window, _| window.set_a11y_forced(true)))
        .unwrap();
    cx.run_until_parked();
    window
}

/// A screenshot as RGBA bytes.
struct Shot {
    width: u32,
    height: u32,
    raw: Vec<u8>,
}

impl Shot {
    fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.raw[i], self.raw[i + 1], self.raw[i + 2]]
    }
}

/// Draws a frame, as the platform does after a refresh (the headless
/// platform draws only when asked).
fn draw(cx: &mut HeadlessAppContext, window: WindowHandle<PlinthRoot>) {
    cx.update_window(window.into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    })
    .unwrap();
}

fn capture(cx: &mut HeadlessAppContext, window: WindowHandle<PlinthRoot>) -> Shot {
    draw(cx, window);
    let image = cx.capture_screenshot(window.into()).expect("render");
    Shot {
        width: image.width(),
        height: image.height(),
        raw: image.into_raw(),
    }
}

fn shot(cx: &mut HeadlessAppContext, window: WindowHandle<PlinthRoot>, name: &str) -> Shot {
    draw(cx, window);
    let image = cx.capture_screenshot(window.into()).expect("render");
    let shots = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&shots).unwrap();
    image
        .save(shots.join(format!("settings-{name}.png")))
        .expect("write the screenshot");
    Shot {
        width: image.width(),
        height: image.height(),
        raw: image.into_raw(),
    }
}

/// The color of the window background: the pixel at the bottom right
/// corner, which no example draws on.
fn background_pixel(image: &Shot) -> [u8; 3] {
    image.pixel(image.width - 3, image.height - 3)
}

fn as_rgb(color: gpui::Hsla) -> [u8; 3] {
    let c = gpui::hsla_to_rgba(color).color;
    [c.red, c.green, c.blue].map(|v| (v * 255.0).round() as u8)
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

/// The share of pixels that are near `color`.
fn share_of(image: &Shot, color: [u8; 3]) -> f64 {
    let n = image
        .raw
        .chunks_exact(4)
        .filter(|p| close([p[0], p[1], p[2]], color))
        .count();
    n as f64 / f64::from(image.width * image.height)
}

fn labels(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<PlinthRoot>,
) -> Vec<(Role, String, Rect)> {
    let tree = cx
        .update(|cx| window.update(cx, |_, window, _| window.a11y_tree().cloned()))
        .unwrap()
        .unwrap();
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            Some((
                n.role(),
                n.label().or(n.value()).unwrap_or("").to_owned(),
                n.bounds()?,
            ))
        })
        .collect()
}

fn set(cx: &mut HeadlessAppContext, settings: SystemSettings) {
    cx.update(|cx| plinth_host_desktop::system_settings::apply(settings, cx));
    cx.run_until_parked();
}

#[test]
fn dark_and_high_contrast_paint_their_tokens() {
    let mut cx = headless();
    let base = SystemSettings::default();
    let cases = [
        ("light", base, Tokens::new(WindowAppearance::Light, ACCENT)),
        (
            "dark",
            SystemSettings {
                dark: Some(true),
                ..base
            },
            Tokens::new(WindowAppearance::Dark, ACCENT),
        ),
        (
            "hc-light",
            SystemSettings {
                high_contrast: true,
                ..base
            },
            Tokens::high_contrast(false, None, ACCENT),
        ),
        (
            "hc-dark",
            SystemSettings {
                dark: Some(true),
                high_contrast: true,
                ..base
            },
            Tokens::high_contrast(true, None, ACCENT),
        ),
    ];
    for example in EXAMPLES {
        let component = compile(example);
        for (name, settings, tokens) in cases {
            set(&mut cx, settings);
            let window = open(&mut cx, &component, 900., 760.);
            let image = shot(&mut cx, window, &format!("{example}-{name}"));
            let bg = background_pixel(&image);
            assert!(
                close(bg, as_rgb(tokens.background)),
                "{example} {name}: the background is {bg:?}, not {:?}",
                as_rgb(tokens.background)
            );
            if name.starts_with("hc") {
                // The borders and the text are in the text color: some of the
                // pixels are exactly it, and it is at least 7:1 on the background.
                assert!(
                    share_of(&image, as_rgb(tokens.text)) > 0.001,
                    "{example} {name}: no text-color pixels"
                );
                assert!(contrast_ratio(tokens.text, tokens.background) >= 7.0);
            }
            cx.update(|cx| window.update(cx, |_, window, _| window.remove_window()))
                .unwrap();
            cx.run_until_parked();
        }
    }
}

/// 200 % text: the type scale doubles and the layout holds (every element
/// stays in the window width, the text does not overlap the next row).
#[test]
fn text_scale_200_percent_keeps_the_layout() {
    let mut cx = headless();
    for example in EXAMPLES {
        let component = compile(example);
        for (class, w) in [("compact", 400.0_f32), ("regular", 900.0)] {
            set(&mut cx, SystemSettings::default());
            // Tall enough that the bottom tab bar covers no content.
            let window = open(&mut cx, &component, w, 1800.);
            shot(&mut cx, window, &format!("{example}-{class}-100"));
            let normal = labels(&mut cx, window);
            let scale = f64::from(
                cx.update(|cx| window.update(cx, |_, window, _| window.scale_factor()))
                    .unwrap(),
            );

            set(
                &mut cx,
                SystemSettings {
                    text_scale: 2.0,
                    ..SystemSettings::default()
                },
            );
            draw(&mut cx, window);
            let rem = cx
                .update(|cx| window.update(cx, |_, window, _| window.rem_size()))
                .unwrap();
            assert_eq!(rem, px(32.), "{example}: the rem size is not 200 %");
            shot(&mut cx, window, &format!("{example}-{class}-200"));
            let large = labels(&mut cx, window);

            // The screen title is about twice as tall.
            let title = |found: &[(Role, String, Rect)]| {
                found
                    .iter()
                    .filter(|(r, l, _)| *r == Role::Heading && !l.is_empty())
                    .map(|(_, _, b)| b.y1 - b.y0)
                    .next()
            };
            if let (Some(a), Some(b)) = (title(&normal), title(&large)) {
                assert!(
                    b > a * 1.6,
                    "{example} {class}: the title is {a} then {b} px tall"
                );
            }
            // Nothing is wider than the window (no horizontal overflow).
            let width = f64::from(w) * scale;
            for (role, label, b) in &large {
                if matches!(
                    role,
                    Role::Window
                        | Role::ScrollView
                        | Role::GenericContainer
                        | Role::Group
                        | Role::List
                ) {
                    continue;
                }
                assert!(
                    b.x1 - b.x0 <= width + 1.0,
                    "{example} {class}: {role:?} {label:?} is wider than the window: {b:?}"
                );
            }
            // Buttons do not overlap each other.
            let buttons: Vec<&Rect> = large
                .iter()
                .filter(|(r, l, _)| *r == Role::Button && !l.is_empty())
                .map(|(_, _, b)| b)
                .collect();
            for (i, a) in buttons.iter().enumerate() {
                for b in &buttons[i + 1..] {
                    let overlap_x = a.x0.max(b.x0) < a.x1.min(b.x1) - 1.0;
                    let overlap_y = a.y0.max(b.y0) < a.y1.min(b.y1) - 1.0;
                    assert!(
                        !(overlap_x && overlap_y),
                        "{example} {class}: two buttons overlap at 200 %: {a:?} {b:?}"
                    );
                }
            }
            cx.update(|cx| window.update(cx, |_, window, _| window.remove_window()))
                .unwrap();
            cx.run_until_parked();
        }
    }
    set(&mut cx, SystemSettings::default());
}

/// A change of a setting draws the open window again with the new theme,
/// without a restart: the host stores the new `SystemSettings` and
/// refreshes each window (`system_settings::apply`).
#[test]
fn a_live_change_redraws_without_a_restart() {
    let mut cx = headless();
    let component = compile("todo");
    set(&mut cx, SystemSettings::default());
    let window = open(&mut cx, &component, 900., 760.);
    let light = background_pixel(&shot(&mut cx, window, "live-before"));
    assert!(close(
        light,
        as_rgb(Tokens::new(WindowAppearance::Light, ACCENT).background)
    ));

    // The OS turns dark mode on: the same window draws dark.
    let dark = SystemSettings {
        dark: Some(true),
        ..SystemSettings::default()
    };
    let changed = cx.update(|cx| plinth_host_desktop::system_settings::apply(dark, cx));
    assert!(changed);
    cx.run_until_parked();
    let after = capture(&mut cx, window);
    assert!(
        close(
            background_pixel(&after),
            as_rgb(Tokens::new(WindowAppearance::Dark, ACCENT).background)
        ),
        "the window did not draw dark"
    );

    // The same settings again: no change and no redraw.
    assert!(!cx.update(|cx| plinth_host_desktop::system_settings::apply(dark, cx)));

    // High contrast on, then off.
    let hc = SystemSettings {
        high_contrast: true,
        ..dark
    };
    set(&mut cx, hc);
    let image = capture(&mut cx, window);
    assert!(close(
        background_pixel(&image),
        as_rgb(Tokens::high_contrast(true, None, ACCENT).background)
    ));
    set(&mut cx, SystemSettings::default());
    let image = capture(&mut cx, window);
    assert!(
        close(background_pixel(&image), light),
        "the window did not go back to light"
    );
}
