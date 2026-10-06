//! Front-end tests: the examples check without errors, and rejected features
//! give their stable codes.

use plinth_compiler::driver::{DiskFs, MemFs, frontend};
use std::path::PathBuf;

fn example(name: &str) -> DiskFs {
    DiskFs { root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name) }
}

fn render(f: &plinth_compiler::driver::Frontend) -> String {
    f.diags.iter().map(|d| f.sources.render(d)).collect::<Vec<_>>().join("\n")
}

#[test]
fn examples_check_clean() {
    for name in ["counter", "todo", "timer", "gallery"] {
        let f = frontend(&example(name));
        assert!(f.diags.is_empty(), "{name}:\n{}", render(&f));
        assert!(f.program.is_some());
    }
}

fn codes(main: &str) -> Vec<&'static str> {
    let fs = MemFs::default().with("app/main.tsx", main);
    let f = frontend(&fs);
    eprintln!("{}", render(&f));
    f.diags.iter().map(|d| d.code).collect()
}

const APP: &str = "\nexport default app({ screens: { home: { title: \"Home\", component: Home } } });\n";

fn with_app(body: &str) -> String {
    format!("import {{ app, Screen }} from \"plinth:ui\";\n{body}\nfunction Home() {{ return <Screen title=\"Home\" />; }}{APP}")
}

#[test]
fn rejected_features() {
    assert_eq!(codes(&with_app("import _ from \"lodash\";")), ["PL1001"]);
    assert_eq!(codes(&with_app("let x: any = 1;")), ["PL2001"]);
    assert_eq!(codes(&with_app("const a = 1 == 2;")), ["PL2002"]);
    // Basic classes and single inheritance are supported; see
    // `tests/lang.rs`'s "Classes" section for the behavior and other class
    // diagnostics. `implements` is still rejected.
    assert_eq!(codes(&with_app("interface I {}\nclass A implements I {}")), ["PL2003"]);
    assert_eq!(codes(&with_app("var v = 1;")), ["PL2014"]);
    assert_eq!(codes(&with_app("const d = document;")), ["PL3002"]);
    assert_eq!(codes(&with_app("const n: number = \"x\";")), ["PL3001"]);
    assert_eq!(codes(&with_app("const c = 1; function f() { c = 2; }")), ["PL3008"]);
    assert_eq!(codes(&with_app("for (const k in {}) {}")), ["PL2007"]);
    assert_eq!(codes(&with_app("function f(x: string | null) { return x!; }")), ["PL2005"]);
}

#[test]
fn jsx_errors() {
    let main = "import { app, Screen, Button } from \"plinth:ui\";\n\
                function Home() { return <Screen title=\"Home\"><Button label=\"x\" color=\"red\" /></Screen>; }"
        .to_owned()
        + APP;
    // Unknown prop, and the missing required onPress.
    assert_eq!(codes(&main), ["PL4002", "PL4003"]);
}

#[test]
fn checkbox_and_text_area() {
    // Good usage: both controls check clean.
    let good = "import { app, Screen, Checkbox, TextArea, signal } from \"plinth:ui\";\n\
                function Home() {\n  const on = signal(false);\n  const notes = signal(\"\");\n  return <Screen title=\"Home\">\n    <Checkbox label=\"Notify\" value={on} />\n    <TextArea label=\"Notes\" value={notes} placeholder=\"...\" />\n  </Screen>;\n}"
        .to_owned()
        + APP;
    assert_eq!(codes(&good), Vec::<&str>::new());

    // Bad usage: missing required value, and an unknown prop.
    let bad = "import { app, Screen, Checkbox, TextArea } from \"plinth:ui\";\n\
               function Home() { return <Screen title=\"Home\"><Checkbox label=\"x\" /><TextArea label=\"y\" value=\"\" rows={4} /></Screen>; }"
        .to_owned()
        + APP;
    assert_eq!(codes(&bad), ["PL4003", "PL4002"]);
}

#[test]
fn image_control() {
    // Good usage: `src` names a real asset and `alt` is not empty.
    let fs = MemFs::default()
        .with(
            "app/main.tsx",
            &("import { app, Screen, Image } from \"plinth:ui\";\n\
               function Home() { return <Screen title=\"Home\"><Image src=\"logo.png\" alt=\"Company logo\" aspect=\"wide\" /></Screen>; }"
                .to_owned()
                + APP),
        )
        .with("assets/logo.png", "");
    let f = frontend(&fs);
    assert!(f.diags.is_empty(), "{}", render(&f));
    assert!(f.program.is_some());

    // Missing asset: a stable code with a help that lists the assets.
    let missing_main = "import { app, Screen, Image } from \"plinth:ui\";\n\
                         function Home() { return <Screen title=\"Home\"><Image src=\"missing.png\" alt=\"x\" /></Screen>; }"
        .to_owned()
        + APP;
    assert_eq!(codes(&missing_main), ["PL4008"]);

    // Empty `alt`: a stable code (SPEC.md §6.3 a11y).
    let fs = MemFs::default()
        .with(
            "app/main.tsx",
            &("import { app, Screen, Image } from \"plinth:ui\";\n\
               function Home() { return <Screen title=\"Home\"><Image src=\"logo.png\" alt=\"\" /></Screen>; }"
                .to_owned()
                + APP),
        )
        .with("assets/logo.png", "");
    let f = frontend(&fs);
    assert_eq!(f.diags.iter().map(|d| d.code).collect::<Vec<_>>(), ["PL4009"]);
}

#[test]
fn icon_control() {
    // Good usage: a name from the runtime icon set, decorative (no label).
    let good = "import { app, Screen, Icon } from \"plinth:ui\";\n\
                function Home() { return <Screen title=\"Home\"><Icon name=\"star\" tone=\"muted\" /></Screen>; }"
        .to_owned()
        + APP;
    let f = frontend(&MemFs::default().with("app/main.tsx", &good));
    assert!(f.diags.is_empty(), "{}", render(&f));
    assert!(f.program.is_some());

    // With a label, it is still accepted (the label makes it accessible,
    // not visible to the type checker as anything special).
    let labeled = "import { app, Screen, Icon } from \"plinth:ui\";\n\
                   function Home() { return <Screen title=\"Home\"><Icon name=\"star\" label=\"Favorite\" /></Screen>; }"
        .to_owned()
        + APP;
    let f = frontend(&MemFs::default().with("app/main.tsx", &labeled));
    assert!(f.diags.is_empty(), "{}", render(&f));

    // An unknown icon name is rejected.
    let bad = "import { app, Screen, Icon } from \"plinth:ui\";\n\
               function Home() { return <Screen title=\"Home\"><Icon name=\"not-a-real-icon\" /></Screen>; }"
        .to_owned()
        + APP;
    let f = frontend(&MemFs::default().with("app/main.tsx", &bad));
    assert!(!f.diags.is_empty());
}

#[test]
fn date_picker_control() {
    // Good usage: a signal value, each mode.
    let good = "import { app, Screen, DatePicker, signal } from \"plinth:ui\";\n\
                function Home() {\n  const due = signal(\"2026-10-06\");\n  const at = signal(\"14:30\");\n  return <Screen title=\"Home\">\n    \
                <DatePicker label=\"Due\" value={due} />\n    \
                <DatePicker label=\"At\" value={at} mode=\"time\" />\n  </Screen>;\n}"
        .to_owned()
        + APP;
    let f = frontend(&MemFs::default().with("app/main.tsx", &good));
    assert!(f.diags.is_empty(), "{}", render(&f));
    assert!(f.program.is_some());

    // An unknown `mode` is rejected (SPEC.md §6.3: mode is "date" | "time"
    // | "datetime").
    let bad = "import { app, Screen, DatePicker, signal } from \"plinth:ui\";\n\
               function Home() {\n  const due = signal(\"\");\n  return <Screen title=\"Home\"><DatePicker label=\"Due\" value={due} mode=\"year\" /></Screen>;\n}"
        .to_owned()
        + APP;
    let f = frontend(&MemFs::default().with("app/main.tsx", &bad));
    assert!(!f.diags.is_empty());
}

#[test]
fn slider_number_picker_progress_badge() {
    // Good usage: all five check clean.
    let good = "import { app, Screen, Slider, NumberField, Picker, Progress, Badge, signal } from \"plinth:ui\";\n\
                function Home() {\n  const v = signal(10);\n  const n = signal(1);\n  const t = signal(\"a\");\n  const opts = [\"a\"];\n  return <Screen title=\"Home\">\n    \
                <Slider label=\"V\" value={v} min={0} max={100} />\n    \
                <NumberField label=\"N\" value={n} />\n    \
                <Picker label=\"T\" value={t} options={[\"a\", \"b\", \"c\"]} />\n    \
                <Picker label=\"U\" value={t} options={opts} />\n    \
                <Progress label=\"P\" value={0.5} />\n    \
                <Badge label=\"new\" />\n  \
                </Screen>;\n}"
        .to_owned()
        + APP;
    assert_eq!(codes(&good), Vec::<&str>::new());

    // Bad usage: missing required `min`/`max` on Slider, and a non-string-array `options`.
    let bad = "import { app, Screen, Slider, Picker, signal } from \"plinth:ui\";\n\
               function Home() {\n  const v = signal(10);\n  const t = signal(\"a\");\n  const opts = [1, 2];\n  return <Screen title=\"Home\">\n    \
               <Slider label=\"V\" value={v} />\n    \
               <Picker label=\"T\" value={t} options={opts} />\n  \
               </Screen>;\n}"
        .to_owned()
        + APP;
    assert_eq!(codes(&bad), ["PL4003", "PL4003", "PL3001"]);

    // A computed value cannot bind both ways.
    let computed = "import { app, Screen, Slider, signal, computed } from \"plinth:ui\";\n\
                     function Home() {\n  const v = signal(10);\n  const c = computed(() => v() * 2);\n  return <Screen title=\"Home\"><Slider label=\"V\" value={c} min={0} max={100} /></Screen>;\n}"
        .to_owned()
        + APP;
    assert_eq!(codes(&computed), ["PL4005"]);
}

#[test]
fn missing_app() {
    assert_eq!(codes("const x = 1;"), ["PL1006"]);
}

/// The names exported by `std/*.d.ts` (what the editor sees) must be the
/// names the compiler knows (SPEC.md §4.7).
#[test]
fn std_typings_match() {
    use plinth_compiler::check::stdlib::{CLIPBOARD_NAMES, CORE_NAMES, DIALOG_NAMES, HUB_NAMES, STORE_NAMES, TIME_NAMES, UI_NAMES};
    let std_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../std");
    let exports = |file: &str| -> Vec<String> {
        let text = std::fs::read_to_string(std_dir.join(file)).unwrap();
        let mut names: Vec<String> = text
            .lines()
            .filter_map(|l| {
                let rest = l.trim_start().strip_prefix("export ")?;
                let rest = rest.strip_prefix("declare ").unwrap_or(rest);
                let rest = ["function ", "const ", "interface ", "type "].iter().find_map(|k| rest.strip_prefix(k))?;
                Some(rest.split(|c: char| !c.is_alphanumeric() && c != '_').next()?.to_owned())
            })
            .collect();
        names.sort();
        names
    };
    let sorted = |names: &[&str]| {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    };
    assert_eq!(exports("ui.d.ts"), sorted(UI_NAMES), "std/ui.d.ts and check::stdlib::UI_NAMES differ");
    assert_eq!(exports("core.d.ts"), sorted(CORE_NAMES), "std/core.d.ts and check::stdlib::CORE_NAMES differ");
    assert_eq!(exports("time.d.ts"), sorted(TIME_NAMES), "std/time.d.ts and check::stdlib::TIME_NAMES differ");
    assert_eq!(exports("store.d.ts"), sorted(STORE_NAMES), "std/store.d.ts and check::stdlib::STORE_NAMES differ");
    assert_eq!(exports("clipboard.d.ts"), sorted(CLIPBOARD_NAMES), "std/clipboard.d.ts and check::stdlib::CLIPBOARD_NAMES differ");
    assert_eq!(exports("dialog.d.ts"), sorted(DIALOG_NAMES), "std/dialog.d.ts and check::stdlib::DIALOG_NAMES differ");
    assert_eq!(exports("hub.d.ts"), sorted(HUB_NAMES), "std/hub.d.ts and check::stdlib::HUB_NAMES differ");
    for c in plinth_compiler::controls::CONTROLS {
        assert!(UI_NAMES.contains(&c.name), "control {} is not in UI_NAMES", c.name);
    }
}
