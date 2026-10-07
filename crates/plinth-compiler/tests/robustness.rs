//! Robustness: the compiler must never panic, whatever the source text.
//!
//! The test takes the sources of the example apps, changes them at random
//! (deletes, duplicates and swaps lines, removes spans, inserts tokens,
//! swaps names), and compiles each result. A diagnostic is a good result;
//! a panic is a bug. The random source is fixed, so a failure repeats.
//!
//! `PLINTH_FUZZ_ITERS=<n>` sets the number of cases (default 300) and
//! `PLINTH_FUZZ_SEED=<n>` the seed. A failure prints the case seed and
//! writes the bad source to `target/fuzz-failures/`.

use plinth_compiler::driver::MemFs;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

/// One example project: its files under `app/` (and the names of its
/// assets), keyed by the path relative to the project root.
struct Project {
    name: String,
    files: HashMap<String, String>,
    capabilities: Vec<String>,
}

fn examples_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn collect(dir: &Path, root: &Path, out: &mut HashMap<String, String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, root, out);
        } else if p.extension().is_some_and(|x| x == "ts" || x == "tsx") {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if let Ok(text) = std::fs::read_to_string(&p) {
                out.insert(rel, text);
            }
        }
    }
}

fn capabilities(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("plinth.toml")) else { return Vec::new() };
    let Ok(value) = text.parse::<toml::Value>() else { return Vec::new() };
    value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn projects() -> Vec<Project> {
    let mut roots = Vec::new();
    let base = examples_root();
    for e in std::fs::read_dir(&base).unwrap().flatten() {
        let p = e.path();
        if p.join("app/main.tsx").is_file() {
            roots.push(p);
        } else if p.is_dir() {
            for e in std::fs::read_dir(&p).unwrap().flatten() {
                if e.path().join("app/main.tsx").is_file() {
                    roots.push(e.path());
                }
            }
        }
    }
    roots.sort();
    roots
        .into_iter()
        .map(|root| {
            let mut files = HashMap::new();
            collect(&root.join("app"), &root, &mut files);
            if let Ok(rd) = std::fs::read_dir(root.join("assets")) {
                for e in rd.flatten() {
                    files.insert(format!("assets/{}", e.file_name().to_string_lossy()), String::new());
                }
            }
            let name = root.strip_prefix(&base).unwrap().to_string_lossy().replace('\\', "/");
            Project { name, capabilities: capabilities(&root), files }
        })
        .collect()
}

const TOKENS: &[&str] = &[
    "(", ")", "{", "}", "[", "]", "<", ">", "</", "/>", "<>", "</>", "=>", ";", ",", ".", "?", ":", "...", "=", "==",
    "!", "?.", "??", "&&", "||", "+", "-", "*", "/", "%", "`", "${", "\"", "'", "null", "undefined", "as", "as any",
    "await", "async", "return", "if", "else", "for", "while", "const", "let", "function", "class", "new", "this",
    "typeof", "instanceof", "in", "of", "0", "1.5", "-1", "1e309", "\"\"", "[]", "{}", "() => {}", "x", "int",
    "number", "string", "boolean", "void", "never", "unknown", "any", "keyof", "readonly", "enum", "interface",
    "type", "extends", "implements", "import", "export", "default", "from", "yield", "break", "continue", "switch",
    "case", "try", "catch", "finally", "throw", "delete", "<Text>", "<Box>", "</Box>", "{...p}", "key={1}", "@",
    "#", "\\", "\u{1F600}", "\n", "  ",
];

/// Byte offsets of char boundaries are needed for slicing.
fn boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn identifiers(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(&s[start..i]);
        } else {
            i += 1;
        }
    }
    out
}

/// Changes `text` in one to three random ways.
fn mutate(text: &str, other: &str, rng: &mut Rng) -> String {
    let mut s = text.to_owned();
    for _ in 0..1 + rng.below(3) {
        let lines: Vec<&str> = s.split_inclusive('\n').collect();
        s = match rng.below(9) {
            0 if !lines.is_empty() => {
                let i = rng.below(lines.len());
                lines.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, l)| *l).collect()
            }
            1 if !lines.is_empty() => {
                let i = rng.below(lines.len());
                let mut v = lines.clone();
                v.insert(i, lines[i]);
                v.concat()
            }
            2 if lines.len() > 1 => {
                let (i, j) = (rng.below(lines.len()), rng.below(lines.len()));
                let mut v = lines.clone();
                v.swap(i, j);
                v.concat()
            }
            3 => {
                let a = boundary(&s, rng.below(s.len() + 1));
                let b = boundary(&s, a + 1 + rng.below(40));
                format!("{}{}", &s[..a], &s[b..])
            }
            4 | 5 => {
                let a = boundary(&s, rng.below(s.len() + 1));
                let t = TOKENS[rng.below(TOKENS.len())];
                format!("{}{}{}", &s[..a], t, &s[a..])
            }
            6 => {
                let ids = identifiers(&s);
                if ids.len() < 2 {
                    s
                } else {
                    let from = ids[rng.below(ids.len())].to_owned();
                    let to = ids[rng.below(ids.len())].to_owned();
                    // Replace one random use.
                    let uses: Vec<usize> = s.match_indices(from.as_str()).map(|(i, _)| i).collect();
                    let at = uses[rng.below(uses.len())];
                    format!("{}{}{}", &s[..at], to, &s[at + from.len()..])
                }
            }
            7 => {
                // A line from another file of the project.
                let other: Vec<&str> = other.split_inclusive('\n').collect();
                if other.is_empty() || lines.is_empty() {
                    s
                } else {
                    let mut v = lines.clone();
                    v.insert(rng.below(lines.len() + 1), other[rng.below(other.len())]);
                    v.concat()
                }
            }
            _ => {
                let a = boundary(&s, rng.below(s.len() + 1));
                s[..a].to_owned()
            }
        };
    }
    s
}

fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic>".into()
    }
}

thread_local! {
    static PANIC_AT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

#[test]
fn mutated_examples_never_panic_the_compiler() {
    let iters: usize = std::env::var("PLINTH_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
    let seed: u64 = std::env::var("PLINTH_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x5EED);
    let projects = projects();
    assert!(projects.len() > 10, "found only {} example projects", projects.len());

    // Silence the default hook: the test reports each panic itself, with
    // the place that the hook records.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        PANIC_AT.with(|p| *p.borrow_mut() = at);
    }));

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fuzz-failures");
    let mut failures: Vec<String> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let (mut ok, mut rejected) = (0usize, 0usize);
    for case in 0..iters {
        let case_seed = seed.wrapping_add(case as u64).wrapping_mul(0x2545_F491_4F6C_DD1D);
        let mut rng = Rng(case_seed);
        let p = &projects[rng.below(projects.len())];
        let mut sources: Vec<&String> = p.files.keys().filter(|k| !k.starts_with("assets/")).collect();
        sources.sort();
        let target = sources[rng.below(sources.len())].clone();
        let other = sources[rng.below(sources.len())];
        let text = mutate(&p.files[&target], &p.files[other], &mut rng);
        let mut files = p.files.clone();
        files.insert(target.clone(), text.clone());
        // A stack overflow ends the process with no report, so the case
        // that runs is on disk.
        let _ = std::fs::create_dir_all(&out_dir);
        let _ = std::fs::write(out_dir.join("current.txt"), format!("case seed {case_seed} ({}/{target})\n{text}", p.name));
        let fs = MemFs(files);
        let caps = p.capabilities.clone();
        let result = catch_unwind(AssertUnwindSafe(|| plinth_compiler::compile_with_capabilities(&fs, &caps)));
        match result {
            Ok(Ok((_, Some(_)))) => ok += 1,
            Ok(Ok((front, None))) => {
                assert!(front.has_errors(), "case {case_seed}: no artifact and no error");
                rejected += 1;
            }
            Ok(Err(e)) => {
                // An internal error is not a panic, but it is still a bug:
                // the front end accepted the program.
                failures.push(format!("case seed {case_seed} ({}/{target}): internal error: {e:#}", p.name));
            }
            Err(e) => {
                let at = PANIC_AT.with(|p| p.borrow().clone());
                let msg = format!("{} at {at}", panic_text(&*e));
                let first = msg.lines().next().unwrap_or("").to_owned();
                let n = seen.entry(first.clone()).or_default();
                *n += 1;
                if *n == 1 {
                    let _ = std::fs::create_dir_all(&out_dir);
                    let file = out_dir.join(format!("{case_seed}.tsx"));
                    let _ = std::fs::write(&file, &text);
                    failures.push(format!("case seed {case_seed} ({}/{target}): panic: {msg}\n  source: {}", p.name, file.display()));
                }
            }
        }
    }
    std::panic::set_hook(hook);
    eprintln!("{iters} cases: {ok} built, {rejected} rejected with diagnostics, {} distinct failures", failures.len());
    assert!(failures.is_empty(), "the compiler failed on mutated sources:\n{}", failures.join("\n"));
}

/// Each source here made the compiler panic once. They must give a
/// diagnostic now.
const REGRESSIONS: &[&str] = &[
    // `break` at the top level of a module: the checker accepted it, and
    // codegen panicked (a subtract with overflow).
    r#"import { app, Screen } from "plinth:ui";break
function M() { return <Screen title="x"></Screen>; }
export default app({ screens: { m: { title: "M", component: M } } });
"#,
    // `break` in a function, outside a loop.
    r#"import { app, Screen } from "plinth:ui";
function M() { if (1 > 0) { break; } return <Screen title="x"></Screen>; }
export default app({ screens: { m: { title: "M", component: M } } });
"#,
];

#[test]
fn sources_that_once_panicked_give_diagnostics() {
    for (i, src) in REGRESSIONS.iter().enumerate() {
        let fs = MemFs::default().with("app/main.tsx", src);
        let (front, artifact) = plinth_compiler::compile(&fs).unwrap_or_else(|e| panic!("case {i}: internal error {e:#}"));
        assert!(artifact.is_none() && front.has_errors(), "case {i}: expected a diagnostic");
    }
}

fn app_with(expr: &str) -> MemFs {
    let main = format!(
        "import {{ app, Screen, Text }} from \"plinth:ui\";\nimport {{ JSON }} from \"plinth:core\";\nfunction Home() {{ const x = {expr}; return <Screen title=\"H\"><Text>{{\"\" + x}}</Text></Screen>; }}\nexport default app({{ screens: {{ home: {{ title: \"H\", component: Home }} }} }});\n"
    );
    MemFs::default().with("app/main.tsx", &main)
}

/// Deep nesting overflowed the stack (a debug build stopped at a sum of 60
/// terms on the 1 MiB main thread of Windows). Now each compile runs on a
/// large stack and the parser limits the depth. The test threads have a
/// small stack too, so these also check `with_stack`.
#[test]
fn deep_nesting_builds_up_to_the_limit_and_is_a_diagnostic_past_it() {
    let depth = plinth_compiler::parse::MAX_DEPTH as usize;
    let ok = [
        vec!["1"; depth - 10].join("+"),
        format!("{}1{}", "(".repeat(depth - 10), ")".repeat(depth - 10)),
        format!("JSON.stringify({}1{})", "[".repeat(300), "]".repeat(300)),
        format!("{}0", "1 > 0 ? 1 : ".repeat(depth - 10)),
    ];
    for (i, e) in ok.iter().enumerate() {
        let (front, artifact) = plinth_compiler::compile(&app_with(e)).unwrap();
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        assert!(artifact.is_some(), "case {i} must build: {diags:?}");
    }
    let too_deep = [vec!["1"; depth * 5].join("+"), format!("{}1{}", "(".repeat(depth * 5), ")".repeat(depth * 5))];
    for (i, e) in too_deep.iter().enumerate() {
        let (front, artifact) = plinth_compiler::compile(&app_with(e)).unwrap();
        assert!(artifact.is_none(), "case {i} must not build");
        let msgs: Vec<&str> = front.diags.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(msgs, vec![format!("this code nests more than {depth} levels deep").as_str()], "case {i}");
    }
}
