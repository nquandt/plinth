//! The test kit (`plinth test`, docs/TESTING.md): passing and failing
//! tests, the failure output, queries, the test clock, isolation, `--filter`,
//! exit codes, and that a package never contains test files.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn plinth() -> Command {
    Command::new(env!("CARGO_BIN_EXE_plinth"))
}

/// A new project in a temp folder with `files` (path, text).
fn project(files: &[(&str, &str)]) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "plinth-test-kit-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(dir.join("app")).unwrap();
    let mut toml = "id = \"dev.plinth.testkit\"\nname = \"Kit\"\nversion = \"0.1.0\"\npublisher = \"plinth\"\n".to_owned();
    for (path, text) in files {
        if *path == "caps" {
            toml.push_str(text);
            continue;
        }
        std::fs::write(dir.join(path), text).unwrap();
    }
    std::fs::write(dir.join("plinth.toml"), toml).unwrap();
    dir
}

fn run_tests(dir: &Path, extra: &[&str]) -> (Output, String) {
    let out = plinth()
        .arg("test")
        .arg(dir)
        .args(extra)
        .output()
        .expect("run plinth test");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out, text)
}

const COUNTER_APP: &str = r#"
import { app, signal, Screen, Section, Heading, Button, Text } from "plinth:ui";
import { setTimeout } from "plinth:time";

export function Counter() {
  const count = signal(0);
  const status = signal("idle");
  const later = () => {
    status.set("waiting");
    setTimeout(() => status.set("done"), 1000);
  };
  return (
    <Screen title="Counter">
      <Section title="Value">
        <Heading level={1}>{count()}</Heading>
        <Text>{status()}</Text>
      </Section>
      <Section title="Actions">
        <Button label="Increment" onPress={() => count.set(count() + 1)} />
        <Button label="Reset" onPress={() => count.set(0)} />
        <Button label="Later" onPress={later} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { counter: { title: "Counter", component: Counter } } });
"#;

#[test]
fn a_passing_test_prints_ok_and_exits_with_0() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, render, expect } from "plinth:test";
import { Counter } from "./main";

test("counts", async () => {
  const s = await render(<Counter />);
  expect(s.byRole("heading")).toHaveText("0");
  await s.press("Increment");
  expect(s.byRole("heading")).toHaveText("1");
  expect(await s.byRole("heading").text()).toBe("1");
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("  ok  counts ("), "{text}");
    assert!(text.contains("1 passed, 0 failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_failed_expectation_gives_the_message_the_line_and_the_screen() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, render, expect } from "plinth:test";
import { Counter } from "./main";

test("counts wrong", async () => {
  const s = await render(<Counter />);
  await s.press("Increment");
  expect(s.byRole("heading")).toHaveText("5");
});

test("adds wrong", async () => {
  expect(1 + 1).toBe(3);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FAIL  counts wrong"), "{text}");
    assert!(text.contains("expect(heading).toHaveText(\"5\")"), "{text}");
    assert!(text.contains("the text is \"1\""), "{text}");
    assert!(text.contains("at app/counter.test.tsx:7"), "{text}");
    // The screen as text, from the rendered component down.
    assert!(
        text.contains("the screen:")
            && text.contains("screen \"Counter\"")
            && text.contains("button \"Increment\""),
        "{text}"
    );
    assert!(
        !text.contains("plinth:test view"),
        "the view screen of the kit is not shown: {text}"
    );
    assert!(text.contains("FAIL  adds wrong"), "{text}");
    assert!(text.contains("expected 2 to be 3"), "{text}");
    assert!(text.contains("at app/counter.test.tsx:11"), "{text}");
    assert!(text.contains("0 passed, 2 failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_ambiguous_query_lists_the_matches() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, render } from "plinth:test";
import { Counter } from "./main";

test("presses a button", async () => {
  const s = await render(<Counter />);
  await s.byRole("button").press();
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("button matches 3 nodes"), "{text}");
    for b in ["Increment", "Reset", "Later"] {
        assert!(
            text.contains(&format!("button \"{b}\" in section \"Actions\"")),
            "{text}"
        );
    }
    assert!(text.contains("at app/counter.test.tsx:6"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_test_awaits_a_timer_on_the_test_clock() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, render, expect, tick, settle } from "plinth:test";
import { Counter } from "./main";

test("tick", async () => {
  const s = await render(<Counter />);
  await s.press("Later");
  expect(s.byText("waiting")).toExist();
  await tick(999);
  expect(s.byText("waiting")).toExist();
  await tick(1);
  expect(s.byText("done")).toExist();
});

test("settle", async () => {
  const s = await render(<Counter />);
  await s.press("Later");
  await settle();
  expect(s.byText("done")).toExist();
});

test("waits for ever", async () => {
  const s = await render(<Counter />);
  await new Promise<void>((resolve) => {});
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(text.contains("  ok  tick ("), "{text}");
    assert!(text.contains("  ok  settle ("), "{text}");
    assert!(text.contains("FAIL  waits for ever"), "{text}");
    assert!(text.contains("the test did not finish"), "{text}");
    assert_eq!(out.status.code(), Some(1), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn each_test_gets_a_new_instance_and_a_new_store() {
    let app = r#"
import { app, signal, Screen, Text, Button } from "plinth:ui";
import { kv } from "plinth:store";

const saved = kv.get("count");
const count = signal(saved === null ? 0 : parseInt(saved));

function Home() {
  const add = () => {
    count.set(count() + 1);
    kv.set("count", `${count()}`);
  };
  return (
    <Screen title="Home">
      <Text>{`count ${count()}`}</Text>
      <Button label="Add" onPress={add} />
    </Screen>
  );
}

function parseInt(s: string): number {
  return s === "1" ? 1 : s === "2" ? 2 : 0;
}

export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    let tests = r#"import { test, renderApp, expect } from "plinth:test";
import { kv } from "plinth:store";

test("first adds two", async () => {
  const s = await renderApp();
  await s.press("Add");
  await s.press("Add");
  expect(s.byText("count 2")).toExist();
  expect(kv.get("count")).toBe("2");
});

test("second starts at zero", async () => {
  const s = await renderApp();
  expect(s.byText("count 0")).toExist();
  expect(kv.get("count")).toBe(null);
});
"#;
    let dir = project(&[
        ("app/main.tsx", app),
        ("app/store.test.tsx", tests),
        (
            "caps",
            "\n[[capabilities]]\nname = \"store.kv\"\nrationale = \"Keep the count.\"\n",
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("2 passed, 0 failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn filter_runs_only_the_tests_whose_names_match() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, expect } from "plinth:test";

test("alpha passes", async () => {
  expect(1).toBe(1);
});

test("beta fails", async () => {
  expect(1).toBe(2);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &["--filter", "alpha"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("  ok  alpha passes"), "{text}");
    assert!(!text.contains("beta"), "{text}");
    assert!(
        text.contains("1 passed, 0 failed, 1 not run (--filter)"),
        "{text}"
    );
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("1 passed, 1 failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_compile_error_in_a_test_file_is_a_diagnostic() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, expect } from "plinth:test";

test("bad", async () => {
  const n: number = "text";
  expect(n).toBe(1);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("app/counter.test.tsx(4,"), "{text}");
    assert!(text.contains("1 error"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn only_test_files_can_import_the_kit() {
    let app = COUNTER_APP.replace(
        "import { setTimeout } from \"plinth:time\";",
        "import { setTimeout } from \"plinth:time\";\nimport { expect } from \"plinth:test\";",
    );
    let dir = project(&[("app/main.tsx", &app)]);
    let out = plinth().arg("check").arg(&dir).output().unwrap();
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!out.status.success(), "{text}");
    assert!(
        text.contains("`plinth:test` is for test files only"),
        "{text}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_package_never_contains_test_files() {
    let tests = r#"import { test, render, expect } from "plinth:test";
import { Counter } from "./main";

test("a test name that only the test file has", async () => {
  const s = await render(<Counter />);
  expect(s.byText("idle")).toExist();
});
"#;
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        ("app/counter.test.tsx", tests),
    ]);
    let build = |dir: &Path| -> Vec<u8> {
        let out = plinth().arg("build").arg(dir).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = std::fs::read(dir.join("dist/testkit.plnt")).unwrap();
        plinth_package::Package::read(&bytes).unwrap().component
    };
    let with_tests = build(&dir);
    assert!(
        !with_tests.windows(10).any(|w| w == b"only the t"),
        "the app module has the test name"
    );
    std::fs::remove_file(dir.join("app/counter.test.tsx")).unwrap();
    let without = build(&dir);
    assert_eq!(with_tests, without, "the test file changes the app module");
    // `plinth check` does not read test files either.
    std::fs::write(dir.join("app/counter.test.tsx"), "this is not Plinth TS").unwrap();
    let out = plinth().arg("check").arg(&dir).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_examples_pass_their_tests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for name in ["counter", "todo"] {
        let (out, text) = run_tests(&root.join(name), &[]);
        assert!(out.status.success(), "{name}: {text}");
        assert!(text.contains(" passed, 0 failed"), "{name}: {text}");
    }
}
