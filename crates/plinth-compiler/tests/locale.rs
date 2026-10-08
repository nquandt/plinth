//! `plinth:locale` (core 1.13, `docs/I18N.md` §3.1, §4 step 1): `locale()`
//! and `direction()` through a real core, with a fixed locale on the runner
//! (`Guest::set_locale`), and the locale event (`lifecycle` kind `LOCALE`,
//! `Guest::change_locale`): the text, a computed value and an effect run
//! again. Set `PLINTH_ENGINE=wasmi` to run them on wasmi.

use plinth_protocol::{ControlKind, Event, NodeId, Writer};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::Tree;

const APP: &str = r#"
import { app, signal, computed, effect, Screen, Text } from "plinth:ui";
import { console } from "plinth:core";
import { locale, direction } from "plinth:locale";
import type { Direction } from "plinth:locale";

const runs = signal(0);

function Home() {
  const side = computed(() => (direction() === "rtl" ? "right" : "left"));
  effect(() => {
    const tag = locale();
    runs.update((n) => n + 1);
    console.log("locale " + tag);
  });
  const d: Direction = direction();
  return (
    <Screen title="Home">
      <Text>{`${locale()} ${direction()}`}</Text>
      <Text>{side()}</Text>
      <Text>{`first ${d}`}</Text>
      <Text>{`runs ${runs()}`}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

fn compile(src: &str) -> plinth_compiler::Artifact {
    let fs = plinth_compiler::driver::MemFs::default().with("app/main.tsx", src);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front
            .diags
            .iter()
            .map(|d| front.sources.render(d))
            .collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    })
}

struct App {
    guest: Guest,
    tree: Tree,
}

impl App {
    /// Starts `APP` with the locale fixed to `tag`.
    fn start(tag: &str) -> App {
        let art = compile(APP);
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(&art.component, Limits::default()).unwrap();
        guest.set_locale(tag);
        let mut app = App {
            guest,
            tree: Tree::new(),
        };
        let commits = app.guest.init(&[]).unwrap();
        app.apply(commits);
        app
    }

    fn apply(&mut self, commits: Vec<Vec<u8>>) {
        for commit in commits {
            assert!(self.tree.apply(&commit).unwrap().is_empty());
        }
    }

    /// The texts of the `Text` controls, in tree order.
    fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        stack.reverse();
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id).unwrap();
            if node.kind == Some(ControlKind::Text) {
                out.push(node.text.clone().unwrap_or_default());
            }
            stack.extend(node.children.iter().rev());
        }
        out
    }

    fn change(&mut self, tag: &str) {
        let commits = self.guest.change_locale(tag).unwrap();
        self.apply(commits);
    }
}

#[test]
fn locale_and_direction_for_each_locale() {
    for (tag, dir, side) in [
        ("en-US", "ltr", "left"),
        ("ar-EG", "rtl", "right"),
        ("he", "rtl", "right"),
        ("zh-Hans-CN", "ltr", "left"),
        ("fr", "ltr", "left"),
    ] {
        let mut app = App::start(tag);
        assert_eq!(
            app.texts(),
            [
                format!("{tag} {dir}"),
                side.to_owned(),
                format!("first {dir}"),
                "runs 1".to_owned()
            ],
            "{tag}"
        );
        assert_eq!(app.guest.take_logs(), [format!("locale {tag}")]);
        assert!(app.guest.take_errors().is_empty());
    }
}

#[test]
fn the_host_normalizes_the_tag() {
    let app = App::start("zh_hans_cn.UTF-8");
    assert_eq!(app.texts()[0], "zh-Hans-CN ltr");
}

#[test]
fn a_locale_change_runs_the_ui_again() {
    let mut app = App::start("en-US");
    assert_eq!(app.texts(), ["en-US ltr", "left", "first ltr", "runs 1"]);
    app.guest.take_logs();

    app.change("ar-EG");
    assert_eq!(
        app.texts(),
        ["ar-EG rtl", "right", "first ltr", "runs 2"],
        "the text, the computed value and the effect follow; a plain read does not"
    );
    assert_eq!(app.guest.take_logs(), ["locale ar-EG"]);

    // The same locale again sends no event.
    app.change("ar-EG");
    assert_eq!(app.texts()[3], "runs 2");
    assert!(app.guest.take_logs().is_empty());

    app.change("fr");
    assert_eq!(app.texts(), ["fr ltr", "left", "first ltr", "runs 3"]);
    assert!(app.guest.take_errors().is_empty());
}

/// The event that a host sends: a `lifecycle` event with kind `LOCALE` (4).
/// With a fixed locale that did not change, the app reads the same values
/// again and nothing changes on screen.
#[test]
fn the_locale_event_alone_is_harmless() {
    let mut app = App::start("he");
    let mut w = Writer::new();
    w.event(&Event::Lifecycle {
        kind: plinth_protocol::lifecycle_kind::LOCALE,
    });
    assert_eq!(
        w.as_bytes(),
        plinth_runner_wasmtime::locale_event().as_slice()
    );
    let commits = app.guest.on_event(w.as_bytes()).unwrap();
    app.apply(commits);
    assert_eq!(app.texts(), ["he rtl", "right", "first rtl", "runs 2"]);
}

/// A fixed locale is never replaced by the OS locale.
#[test]
fn a_fixed_locale_does_not_follow_the_os() {
    let mut app = App::start("ar-EG");
    assert!(!app.guest.refresh_locale());
    assert_eq!(app.texts()[0], "ar-EG rtl");
}

/// An app that does not import `plinth:locale` needs no new core function
/// because of it: it declares the same core as before core 1.13.
#[test]
fn only_an_app_that_uses_the_locale_needs_core_1_13() {
    assert_eq!(compile(APP).runtime, "plinth-core/1.13");
    const PLAIN: &str = r#"
import { app, signal, isActive, Screen, Text, Button } from "plinth:ui";
const count = signal(0);
function Home() {
  return (
    <Screen title="Home">
      <Text>{`${count()} ${isActive()}`}</Text>
      <Button label="Add" onPress={() => count.set(count() + 1)} />
    </Screen>
  );
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    assert_eq!(compile(PLAIN).runtime, "plinth-core/1.12");
}

/// `locale()` and `direction()` take no arguments, and the names must be
/// imported from `plinth:locale`.
#[test]
fn wrong_calls_are_errors() {
    let src = r#"
import { app, Screen, Text } from "plinth:ui";
import { locale } from "plinth:locale";
function Home() {
  return <Screen title="Home"><Text>{locale("en")}</Text></Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let fs = plinth_compiler::driver::MemFs::default().with("app/main.tsx", src);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    assert!(artifact.is_none());
    let text: Vec<String> = front
        .diags
        .iter()
        .map(|d| front.sources.render(d))
        .collect();
    assert!(
        text.join("\n").contains("`locale` takes no arguments"),
        "{text:?}"
    );
}
