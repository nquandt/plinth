//! `plinth:locale` formatting (core 1.13, `docs/I18N.md` §3.7, §4 step 2):
//! `formatNumber`, `formatDate`, `pluralCategory`, `formatList` and
//! `compare` through a real core, with ICU4X in the runner. The cases are in
//! `tests/locale_cases.txt`, which the web test (`web/test/run-locale.mjs`)
//! also reads: the two hosts must give the same text, or the file records a
//! known difference. Set `PLINTH_ENGINE=wasmi` to run them on wasmi.

use plinth_protocol::{ControlKind, NodeId};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::Tree;
use std::collections::BTreeMap;

const LOCALES: &[&str] = &[
    "en-US", "de-DE", "fr-FR", "ar-EG", "ja-JP", "hi-IN", "pl-PL", "ru-RU",
];

/// One case of `locale_cases.txt`: the expression and, for each locale, the
/// text on the desktop.
struct Case {
    name: String,
    expr: String,
    desktop: BTreeMap<String, String>,
}

/// `\u{hex}` and `\\` in a text of the case file.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix('\\') {
            out.push('\\');
            rest = r;
        } else if let Some(r) = rest.strip_prefix("u{") {
            let end = r.find('}').expect("a closing brace after \\u{");
            let cp = u32::from_str_radix(&r[..end], 16).expect("a hex code point");
            out.push(char::from_u32(cp).expect("a valid code point"));
            rest = &r[end + 1..];
        } else {
            panic!("an unknown escape in {s:?}");
        }
    }
    out.push_str(rest);
    out
}

fn read_cases() -> Vec<Case> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/locale_cases.txt");
    let text = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
    let mut cases: Vec<Case> = Vec::new();
    let mut want_expr = false;
    // For each case: the locales with a host line, and the locales with a `known` line.
    let mut host_lines: Vec<(String, String)> = Vec::new();
    let mut known: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("=== case ") {
            cases.push(Case {
                name: name.to_owned(),
                expr: String::new(),
                desktop: BTreeMap::new(),
            });
            want_expr = true;
            continue;
        }
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let case = cases
            .last_mut()
            .unwrap_or_else(|| panic!("a line outside a case: {line}"));
        if want_expr {
            case.expr = line.to_owned();
            want_expr = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("known ") {
            let (loc, reason) = rest
                .split_once(": ")
                .unwrap_or_else(|| panic!("`known <locale>: <reason>`: {line}"));
            assert!(
                !reason.trim().is_empty(),
                "case {}: a known difference needs a reason",
                case.name
            );
            known.push((case.name.clone(), loc.to_owned()));
            continue;
        }
        let (head, value) = line.split_once(' ').unwrap_or((line, ""));
        let (loc, host) = match head.split_once('@') {
            Some((loc, host)) => {
                assert!(
                    host == "desktop" || host == "web",
                    "case {}: an unknown host {host}",
                    case.name
                );
                host_lines.push((case.name.clone(), loc.to_owned()));
                (loc, Some(host))
            }
            None => (head, None),
        };
        assert!(
            LOCALES.contains(&loc),
            "case {}: an unknown locale {loc}",
            case.name
        );
        if host != Some("web") {
            assert!(
                case.desktop
                    .insert(loc.to_owned(), unescape(value))
                    .is_none(),
                "case {}: two lines for {loc}",
                case.name
            );
        }
    }
    for (case, loc) in &host_lines {
        assert!(
            known.contains(&(case.clone(), loc.clone())),
            "case {case}: {loc} has host lines but no `known` line"
        );
    }
    for c in &cases {
        assert!(!c.expr.is_empty(), "case {}: no expression", c.name);
        for loc in LOCALES {
            assert!(
                c.desktop.contains_key(*loc),
                "case {}: no text for {loc}",
                c.name
            );
        }
    }
    cases
}

/// The app: one `Text` for each case, in the order of the file.
fn app_source(cases: &[Case]) -> String {
    let texts: String = cases
        .iter()
        .map(|c| format!("      <Text>{{{}}}</Text>\n", c.expr))
        .collect();
    format!(
        r#"import {{ app, Screen, Text }} from "plinth:ui";
import {{ formatNumber, formatDate, pluralCategory, formatList, compare }} from "plinth:locale";

function Home() {{
  return (
    <Screen title="Home">
{texts}    </Screen>
  );
}}

export default app({{ screens: {{ home: {{ title: "Home", component: Home }} }} }});
"#
    )
}

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
    fn start(component: &[u8], tag: &str) -> App {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(component, Limits::default()).unwrap();
        guest.set_locale(tag);
        guest.set_fake_timezone_offset(0);
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
}

fn check(cases: &[Case], texts: &[String], tag: &str) {
    assert_eq!(texts.len(), cases.len());
    let mut wrong = Vec::new();
    for (c, got) in cases.iter().zip(texts) {
        let want = &c.desktop[tag];
        if got != want {
            wrong.push(format!("{tag} {}: got {got:?}, want {want:?}", c.name));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_shared_cases_in_each_locale() {
    let cases = read_cases();
    assert!(cases.len() >= 50, "the shared file has the cases");
    let art = compile(&app_source(&cases));
    assert_eq!(art.runtime, "plinth-core/1.13");
    for tag in LOCALES {
        let mut app = App::start(&art.component, tag);
        check(&cases, &app.texts(), tag);
        assert!(app.guest.take_errors().is_empty());
    }
}

/// A locale change formats every text again (the calls read the locale
/// signal, as `locale()`).
#[test]
fn a_locale_change_formats_again() {
    let cases = read_cases();
    let art = compile(&app_source(&cases));
    let mut app = App::start(&art.component, "en-US");
    check(&cases, &app.texts(), "en-US");
    for tag in ["de-DE", "ar-EG", "ja-JP"] {
        let commits = app.guest.change_locale(tag).unwrap();
        app.apply(commits);
        check(&cases, &app.texts(), tag);
    }
}

/// Values from variables, the local time zone, and values that are not
/// finite.
#[test]
fn dynamic_values_and_the_time_zone() {
    let src = r#"
import { app, signal, Screen, Text, Button } from "plinth:ui";
import { formatNumber, formatDate, pluralCategory, formatList, compare } from "plinth:locale";
import type { PluralType, ListType } from "plinth:locale";

const code = signal("EUR");
const digits = signal(1);
const names = ["Zoë", "adam", "Émile", "bob"];

function Home() {
  const kind: PluralType = "ordinal";
  const list: ListType = "disjunction";
  const sorted = names.slice().sort((a, b) => compare(a, b));
  return (
    <Screen title="Home">
      <Text>{formatNumber(9.87, { style: "currency", currency: code() })}</Text>
      <Text>{formatNumber(3.14159, { maximumFractionDigits: digits() })}</Text>
      <Text>{formatDate(1791459845000, { timeStyle: "short" })}</Text>
      <Text>{pluralCategory(2, kind)}</Text>
      <Text>{formatList(sorted, list)}</Text>
      <Text>{`[${formatNumber(0 / 0)}][${formatDate(1 / 0)}][${pluralCategory(1 / 0)}]`}</Text>
      <Button label="Next" onPress={() => { code.set("JPY"); digits.set(3); }} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let art = compile(src);
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&art.component, Limits::default()).unwrap();
    guest.set_locale("en-US");
    guest.set_fake_timezone_offset(120);
    let mut app = App {
        guest,
        tree: Tree::new(),
    };
    let commits = app.guest.init(&[]).unwrap();
    app.apply(commits);
    assert_eq!(
        app.texts(),
        [
            "€9.87",
            "3.1",
            "1:44 PM",
            "two",
            "adam, bob, Émile, or Zoë",
            "[][][other]"
        ]
    );
    assert!(app.guest.take_errors().is_empty());
}

/// Wrong options are compile errors with a help text.
#[test]
fn wrong_options_are_errors() {
    let errors = |body: &str| -> String {
        let src = format!(
            r#"
import {{ app, Screen, Text }} from "plinth:ui";
import {{ formatNumber, formatDate, pluralCategory, formatList, compare }} from "plinth:locale";
const opts = {{ style: "percent" }};
function Home() {{
  return <Screen title="Home"><Text>{{{body}}}</Text></Screen>;
}}
export default app({{ screens: {{ home: {{ title: "Home", component: Home }} }} }});
"#
        );
        let fs = plinth_compiler::driver::MemFs::default().with("app/main.tsx", &src);
        let (front, artifact) =
            plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        assert!(artifact.is_none(), "{body} compiles");
        front
            .diags
            .iter()
            .map(|d| front.sources.render(d))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(errors(r#"formatNumber(1, { style: "unit" })"#).contains("PL"));
    assert!(errors(r#"formatNumber(1, { digits: 2 })"#)
        .contains("the options of `formatNumber` have no `digits` property"));
    assert!(errors("formatNumber(1, opts)")
        .contains("the options of `formatNumber` must be an object literal"));
    assert!(errors(r#"formatDate(0, { timeStyle: "full" })"#).contains("PL"));
    assert!(errors("formatNumber()").contains("`formatNumber` takes a number"));
    assert!(errors(r#"pluralCategory(1, "plural")"#).contains("PL"));
    assert!(errors(r#"formatList("a")"#).contains("PL"));
    assert!(errors(r#"`${compare("a")}`"#).contains("`compare` takes two strings"));
}
