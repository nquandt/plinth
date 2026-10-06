//! The desktop host entry point.
//!
//! Usage: `plinth-host <app.wasm> [--accent <name>]`

use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions, actions, px, size};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::{GuestPort, PlinthRoot};
use std::path::PathBuf;

actions!(plinth_host, [Quit]);

struct WasmGuest {
    guest: Guest,
    // The runner owns the engine's epoch ticker, so it lives as long as the guest.
    _runner: Runner,
}

impl GuestPort for WasmGuest {
    fn dispatch(&mut self, events: &[u8]) -> Result<Vec<Vec<u8>>> {
        self.guest.on_event(events)
    }
}

struct Args {
    artifact: PathBuf,
    accent: String,
}

fn parse_args() -> Result<Args> {
    let mut artifact = None;
    let mut accent = "teal".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--accent" => accent = args.next().context("--accent needs a value")?,
            "-h" | "--help" => {
                println!("usage: plinth-host <app.wasm> [--accent <name>]");
                std::process::exit(0);
            }
            _ if artifact.is_none() => artifact = Some(PathBuf::from(arg)),
            _ => bail!("unexpected argument `{arg}`"),
        }
    }
    Ok(Args { artifact: artifact.context("usage: plinth-host <app.wasm> [--accent <name>]")?, accent })
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = parse_args()?;
    let bytes = std::fs::read(&args.artifact).with_context(|| format!("read {}", args.artifact.display()))?;

    let runner = Runner::new()?;
    let mut guest = runner.load(&bytes, Limits::default())?;
    let init = guest.init(&[]);
    let port = Box::new(WasmGuest { guest, _runner: runner });
    let title = args.artifact.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let accent = args.accent;

    gpui_platform::application().run(move |cx: &mut App| {
        plinth_ui::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1000.), px(720.)), cx);
        let options = WindowOptions::new()
            .window_bounds(Some(WindowBounds::Windowed(bounds)))
            .titlebar(Some(gpui::TitlebarOptions { title: Some(title.into()), ..Default::default() }));
        cx.open_window(options, move |_, cx| {
            cx.new(move |cx| match init {
                Ok(commits) => PlinthRoot::new(port, commits, accent, cx),
                Err(e) => PlinthRoot::stopped(port, format!("{e:#}"), accent),
            })
        })
        .expect("open the window");
        cx.activate(true);
    });
    Ok(())
}
