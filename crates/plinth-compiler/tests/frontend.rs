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
    for name in ["counter", "todo"] {
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
    assert_eq!(codes(&with_app("class A {}")), ["PL2003"]);
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
fn missing_app() {
    assert_eq!(codes("const x = 1;"), ["PL1006"]);
}
