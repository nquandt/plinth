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
fn describe_groups_tests_and_can_nest() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, describe, expect } from "plinth:test";

describe("math", () => {
  test("adds", async () => {
    expect(1 + 1).toBe(2);
  });
  describe("odd numbers", () => {
    test("three is odd", async () => {
      expect(3 % 2).toBe(1);
    });
  });
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("  ok  math > adds ("), "{text}");
    assert!(
        text.contains("  ok  math > odd numbers > three is odd ("),
        "{text}"
    );
    assert!(text.contains("2 passed, 0 failed"), "{text}");
    let (out, text) = run_tests(&dir, &["--filter", "odd numbers"]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("three is odd"), "{text}");
    assert!(!text.contains("  ok  math > adds"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn before_each_and_after_each_run_outer_first_and_inner_first() {
    let app = COUNTER_APP.replace(
        "import { setTimeout } from \"plinth:time\";",
        "import { setTimeout } from \"plinth:time\";\nimport { kv } from \"plinth:store\";",
    );
    let dir = project(&[
        ("app/main.tsx", &app),
        (
            "app/counter.test.tsx",
            r#"import { test, describe, beforeEach, afterEach, expect } from "plinth:test";
import { kv } from "plinth:store";

beforeEach(async () => {
  kv.set("log", `${kv.get("log") ?? ""}outer-before,`);
});
afterEach(async () => {
  kv.set("log", `${kv.get("log") ?? ""}outer-after,`);
});

describe("group", () => {
  beforeEach(async () => {
    kv.set("log", `${kv.get("log") ?? ""}inner-before,`);
  });
  afterEach(async () => {
    kv.set("log", `${kv.get("log") ?? ""}inner-after,`);
  });
  test("order", async () => {
    expect(kv.get("log")).toBe("outer-before,inner-before,");
  });
});
"#,
        ),
        (
            "caps",
            "\n[[capabilities]]\nname = \"store.kv\"\nrationale = \"Log the order.\"\n",
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("1 passed, 0 failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_throw_in_before_each_fails_the_test() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, beforeEach, expect } from "plinth:test";

beforeEach(async () => {
  throw new Error("setup failed");
});

test("never runs cleanly", async () => {
  expect(1).toBe(1);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FAIL  never runs cleanly"), "{text}");
    assert!(text.contains("setup failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn test_only_runs_only_the_only_tests_and_test_skip_does_not_run() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, testOnly, testSkip, expect } from "plinth:test";

test("not selected", async () => {
  expect(1).toBe(2);
});

testOnly("selected", async () => {
  expect(1).toBe(1);
});

testSkip("skipped one", async () => {
  expect(1).toBe(2);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("  ok  selected ("), "{text}");
    assert!(!text.contains("not selected"), "{text}");
    assert!(text.contains("skip  skipped one"), "{text}");
    assert!(text.contains("1 passed, 0 failed, 2 skipped"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn describe_only_and_describe_skip() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, describeOnly, describeSkip, expect } from "plinth:test";

describeSkip("off", () => {
  test("never runs", async () => {
    expect(1).toBe(2);
  });
});

describeOnly("on", () => {
  test("runs", async () => {
    expect(1).toBe(1);
  });
});

test("not in any group", async () => {
  expect(1).toBe(2);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("  ok  on > runs ("), "{text}");
    assert!(!text.contains("not in any group"), "{text}");
    assert!(text.contains("skip  off > never runs"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn number_comparison_matchers() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, expect } from "plinth:test";

test("greater and less", async () => {
  expect(5).toBeGreaterThan(3);
  expect(5).toBeGreaterThanOrEqual(5);
  expect(3).toBeLessThan(5);
  expect(3).toBeLessThanOrEqual(3);
  expect(3).not.toBeGreaterThan(5);
});

test("fails with a clear message", async () => {
  expect(3).toBeGreaterThan(5);
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("  ok  greater and less ("), "{text}");
    assert!(text.contains("FAIL  fails with a clear message"), "{text}");
    assert!(text.contains("expected 3 to be greater than 5"), "{text}");
    assert!(text.contains("1 passed, 1 failed"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn to_throw_on_a_function_value() {
    let dir = project(&[
        ("app/main.tsx", COUNTER_APP),
        (
            "app/counter.test.tsx",
            r#"import { test, expect } from "plinth:test";

function boom(): void {
  throw new Error("bad thing");
}

test("throws, and the message contains text", async () => {
  expect(() => boom()).toThrow();
  expect(() => boom()).toThrow("bad");
  expect(() => {}).not.toThrow();
});

test("the message does not match", async () => {
  expect(() => boom()).toThrow("nope");
});

test("does not throw", async () => {
  expect(() => {}).toThrow();
});
"#,
        ),
    ]);
    let (out, text) = run_tests(&dir, &[]);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("  ok  throws, and the message contains text ("),
        "{text}"
    );
    assert!(text.contains("FAIL  the message does not match"), "{text}");
    assert!(text.contains("it threw \"bad thing\""), "{text}");
    assert!(text.contains("FAIL  does not throw"), "{text}");
    assert!(text.contains("1 passed, 2 failed"), "{text}");
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
