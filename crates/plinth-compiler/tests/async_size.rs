//! The code size of `async`/`await` against the same logic with `done`
//! callbacks. It prints the app module sizes and checks nothing; it is a
//! measure for "smaller async code" (HANDOFF §10 item 7).

use plinth_compiler::driver::MemFs;

fn app_size(body: &str) -> usize {
    let main = format!(
        "import {{ app, signal, Screen, Text, Button }} from \"plinth:ui\";\nimport {{ confirm, alert }} from \"plinth:dialog\";\nconst out = signal(\"\");\n{body}\nfunction Home() {{ return <Screen title=\"H\"><Text>{{out()}}</Text><Button label=\"go\" onPress={{() => run()}} /></Screen>; }}\nexport default app({{ screens: {{ home: {{ title: \"H\", component: Home }} }} }});\n"
    );
    let fs = MemFs::default().with("app/main.tsx", &main).with("plinth.toml", "");
    let (front, artifact) = plinth_compiler::compile(&fs).unwrap();
    let artifact = artifact.unwrap_or_else(|| panic!("{:?}", front.diags.iter().map(|d| front.sources.render(d)).collect::<Vec<_>>()));
    artifact.app.len()
}

#[test]
#[ignore]
fn async_code_size() {
    let base = app_size("function run(): void { out.set(\"x\"); }");
    for n in [1usize, 2, 4, 8] {
        let awaits: String = (0..n).map(|i| format!("  const a{i} = await confirm(\"q{i}\");\n  out.set(out() + (a{i} ? \"y\" : \"n\"));\n")).collect();
        let asynced = app_size(&format!("async function run(): Promise<void> {{\n{awaits}}}"));
        let mut cb = String::from("out.set(\"done\");");
        for i in (0..n).rev() {
            cb = format!("confirm(\"q{i}\", (a{i}) => {{ out.set(out() + (a{i} ? \"y\" : \"n\")); {cb} }});");
        }
        let callbacks = app_size(&format!("function run(): void {{ {cb} }}"));
        eprintln!("{n} await(s): async {} B, callbacks {} B (base {base} B): async adds {} B, callbacks add {} B", asynced, callbacks, asynced - base, callbacks - base);
    }
}
