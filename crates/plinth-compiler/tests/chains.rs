//! Long left-deep chains (`docs/GAPS.md` R3, `docs/PRODUCTION.md` L6):
//! `a + b + c + ...`, `a && b && ...` and `x.f().f().f()...` with many
//! thousand terms compile without recursion on the chain, and real
//! nesting past the limit is the diagnostic `PL1008`, never a crash.

use plinth_compiler::driver::MemFs;
use plinth_protocol::ControlKind;
use plinth_runner_wasmtime::{Limits, Runner};
use plinth_ui::tree::Tree;
use std::time::Instant;

/// A program that declares `decls` at the top level and shows `"" + x`
/// in a `Text`, with `x` declared in `decls`.
fn program(decls: &str) -> String {
    format!(
        "import {{ app, Screen, Text }} from \"plinth:ui\";\n{decls}\nfunction Home() {{ return <Screen title=\"Home\"><Text>{{\"\" + x}}</Text></Screen>; }}\nexport default app({{ screens: {{ home: {{ title: \"Home\", component: Home }} }} }});\n"
    )
}

fn compile(main: &str) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", main);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().take(5).map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    })
}

/// Compiles and runs `decls`, and returns the text that the app shows.
fn shown(decls: &str) -> String {
    let artifact = compile(&program(decls));
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(ControlKind::Text)
            && let Some(t) = &node.text
        {
            return t.clone();
        }
        stack.extend(node.children.iter());
    }
    panic!("no Text found");
}

fn sum_source(n: usize) -> String {
    let terms: Vec<String> = (0..=n).map(|i| i.to_string()).collect();
    format!("const x = {};", terms.join(" + "))
}

#[test]
fn a_sum_of_5000_terms() {
    assert_eq!(shown(&sum_source(5000)), "12502500");
}

#[test]
fn a_string_concatenation_of_5000_terms() {
    let terms: Vec<&str> = (0..5000).map(|i| if i % 2 == 0 { "\"a\"" } else { "\"b\"" }).collect();
    let decls = format!("const s = {};\nconst x = s.length + \":\" + s.slice(0, 4) + \":\" + s.slice(4996);", terms.join(" + "));
    assert_eq!(shown(&decls), "5000:abab:abab");
}

#[test]
fn mixed_operators_in_a_chain_of_5000_terms() {
    // `1 * 2 - 1 + 1 * 2 - 1 + ...`: each group of three gives 1.
    let groups = vec!["1 * 2 - 1"; 5000].join(" + ");
    assert_eq!(shown(&format!("const x = {groups};")), "5000");
}

#[test]
fn and_and_or_chains_of_5000_terms() {
    let and = vec!["t"; 5000].join(" && ");
    let or = vec!["f"; 5000].join(" || ");
    let decls = format!("const t = 1 > 0;\nconst f = 1 < 0;\nconst a = {and};\nconst b = {or};\nconst x = a + \",\" + b;");
    assert_eq!(shown(&decls), "true,false");
}

/// `??`, `?.` and `||` on strings give a `Block` for each link; the
/// checker merges them into one (`flatten_block`). Each link keeps one
/// local, so these chains stop at the Wasm limit of 50,000 locals in a
/// function.
#[test]
fn coalesce_optional_and_string_or_chains_of_5000_terms() {
    let nulls = vec!["n"; 5000].join(" ?? ");
    let empties = vec!["e"; 5000].join(" || ");
    let nexts = "?.next".repeat(5000);
    let decls = format!(
        "class L {{\n  next: L | null = null;\n  v: number = 1;\n}}\nconst l = new L();\nconst n: number | null = null;\nconst e: string = \"\";\nconst a = {nulls} ?? 7;\nconst b = {empties} || \"z\";\nconst c = l.next{nexts}?.v ?? 3;\nconst x = a + b + c;"
    );
    assert_eq!(shown(&decls), "7z3");
}

/// Each `??`, `?.` or non-boolean `||` link keeps one Wasm local and used
/// to allocate a fresh one for each link, so a long chain of such links
/// ran into the Wasm limit of locals in a function ("the app module is
/// not valid") near 50,000 links. The fix (docs/GAPS.md R3) reuses one
/// local per type across the links of a chain, so 60,000 links of each
/// kind compile and run.
#[test]
fn coalesce_chain_of_60000_terms() {
    let nulls = vec!["n"; 60_000].join(" ?? ");
    let decls = format!("const n: number | null = null;\nconst x = {nulls} ?? 7;");
    assert_eq!(shown(&decls), "7");
}

#[test]
fn optional_member_chain_of_60000_terms() {
    let nexts = "?.next".repeat(60_000);
    let decls = format!(
        "class L {{\n  next: L | null = null;\n  v: number = 1;\n}}\nconst l = new L();\nconst x = l.next{nexts}?.v ?? 3;"
    );
    assert_eq!(shown(&decls), "3");
}

#[test]
fn string_or_chain_of_60000_terms() {
    let empties = vec!["e"; 60_000].join(" || ");
    let decls = format!("const e: string = \"\";\nconst x = {empties} || \"z\";");
    assert_eq!(shown(&decls), "z");
}

/// Each `n > 0` needs the narrowing of `n !== null` at the start of the
/// chain, which the loop in the checker carries from link to link.
#[test]
fn a_narrowing_and_chain_of_1000_terms() {
    let tests = vec!["n > 0"; 1000].join(" && ");
    let decls = format!("const n: number | null = 5;\nconst x = n !== null && {tests};");
    assert_eq!(shown(&decls), "true");
}

/// `lin` in the async transform walks a chain whose `await` is at the
/// innermost left side with a loop.
#[test]
fn an_await_at_the_start_of_a_chain_of_5000_terms() {
    let terms = vec!["1"; 5000].join(" + ");
    let decls = format!(
        "let x = 0;\nasync function f(): Promise<number> {{ return 1; }}\nasync function g(): Promise<void> {{ x = await f() + {terms}; }}\ng();"
    );
    let artifact = compile(&program(&decls));
    assert!(!artifact.app.is_empty());
}

#[test]
fn a_method_chain_of_5000_calls() {
    let calls = ".add(1)".repeat(5000);
    let decls = format!(
        "class B {{\n  n: number = 0;\n  add(v: number): B {{ this.n = this.n + v; return this; }}\n}}\nconst x = new B(){calls}.n;"
    );
    assert_eq!(shown(&decls), "5000");
}

#[test]
fn a_sum_of_100000_terms_compiles_in_reasonable_time() {
    let n = 100_000;
    let start = Instant::now();
    compile(&program(&sum_source(n)));
    let sum = start.elapsed();
    let and = vec!["t"; n].join(" && ");
    let start = Instant::now();
    compile(&program(&format!("const t = 1 > 0;\nconst x = {and};")));
    let and = start.elapsed();
    let cat = vec!["\"ab\""; n].join(" + ");
    let start = Instant::now();
    compile(&program(&format!("const x = ({cat}).length;")));
    let cat = start.elapsed();
    let calls = ".add(1)".repeat(n);
    let start = Instant::now();
    compile(&program(&format!(
        "class B {{\n  n: number = 0;\n  add(v: number): B {{ this.n = this.n + v; return this; }}\n}}\nconst x = new B(){calls}.n;"
    )));
    let calls = start.elapsed();
    let terms = vec!["1"; n].join(" + ");
    let start = Instant::now();
    compile(&program(&format!(
        "let x = 0;\nasync function f(): Promise<number> {{ return 1; }}\nasync function g(): Promise<void> {{ x = await f() + {terms}; }}\ng();"
    )));
    let awaits = start.elapsed();
    eprintln!(
        "compile time for {n} terms: sum {sum:?}, && {and:?}, string concatenation {cat:?}, method calls {calls:?}, await + sum {awaits:?}"
    );
}

fn nesting_code(decls: &str) -> Vec<(&'static str, String)> {
    let fs = MemFs::default().with("app/main.tsx", &program(decls));
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    assert!(artifact.is_none(), "too deep nesting must not build");
    front.diags.iter().map(|d| (d.code, d.message.clone())).collect()
}

/// Real nesting (parentheses on the right, `?:`, unary operators, blocks,
/// arrays, calls, JSX, type arguments) cannot be iterative. Past the
/// limit it is `PL1008`, once, and never a stack overflow, also far past
/// the limit: the parser reports it up to `nesting::SCAN_LIMIT`, and the
/// scan before the parser past that.
#[test]
fn real_nesting_past_the_limit_is_pl1008() {
    let max = plinth_compiler::parse::MAX_DEPTH as usize;
    for n in [max + 1, 15_000, 100_000] {
        let cases = [
            format!("const x = {}1{};", "(1 + ".repeat(n), ")".repeat(n)),
            format!("const x = {}0;", "1 > 0 ? 1 : ".repeat(n)),
            format!("const x = {}1;", "-".repeat(n).split("").collect::<Vec<_>>().join(" ")),
            format!("const x = JSON.stringify({}1{});", "[".repeat(n), "]".repeat(n)),
            format!("function g(v: number): number {{ return v; }}\nconst x = {}1{};", "g(".repeat(n), ")".repeat(n)),
            format!("let x = 0;\nfunction f() {{ {}x = 1;{} }}", "{ ".repeat(n), " }".repeat(n)),
            format!("let x = 0;\nfunction f() {{ {}x = 1; }}", "if (x > 0) ".repeat(n)),
            format!("const x = 1;\nconst y = {}<Screen title=\"a\" />{};", "<Screen title=\"a\">".repeat(n), "</Screen>".repeat(n)),
            format!("const x = 1;\nlet y: {}number{} = [];", "Array<".repeat(n), ">".repeat(n)),
        ];
        for (i, decls) in cases.iter().enumerate() {
            let decls = if decls.contains("JSON") { format!("import {{ JSON }} from \"plinth:core\";\n{decls}") } else { decls.clone() };
            let diags = nesting_code(&decls);
            let want = format!("this code nests more than {max} levels deep");
            assert_eq!(diags, vec![("PL1008", want)], "case {i} at depth {n}");
        }
    }
}
