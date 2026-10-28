//! Drift check for `src/host_traits.rs` (PRODUCTION R5).
//!
//! With the `wasmtime` feature, `bindgen!` makes the host traits from
//! `wit/plinth/app.wit`. Without it, `src/host_traits.rs` holds a hand-written
//! copy. The build without wasmtime fails to compile when the copy is wrong,
//! but the default build never reads the copy. This test reads both files as
//! text and compares them, so the default build fails too.
//!
//! The test compares, for each interface: the set of interfaces, the
//! functions (name, order, parameter names, parameter types, return type), the
//! cases of each `enum` and each `variant`. A WIT type that the test does not
//! know makes the test fail with a clear message: teach `rust_type` the new
//! type. The test reads text only. It does not need wasmtime or wasmi.

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn snake(kebab: &str) -> String {
    kebab.replace('-', "_")
}

fn pascal(kebab: &str) -> String {
    kebab
        .split('-')
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

/// Splits `text` at the commas that are not inside `<>`, `()` or `{}`.
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut cur) = (0i32, String::new());
    for c in text.chars() {
        match c {
            '<' | '(' | '{' => depth += 1,
            '>' | ')' | '}' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            out.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

/// The Rust type for a WIT type.
fn rust_type(wit: &str) -> String {
    let wit = wit.trim();
    if let Some(open) = wit.find('<') {
        assert!(wit.ends_with('>'), "bad WIT type {wit:?}");
        let head = &wit[..open];
        let args: Vec<String> = split_top(&wit[open + 1..wit.len() - 1])
            .iter()
            .map(|a| if a == "_" { "()".to_owned() } else { rust_type(a) })
            .collect();
        return match head {
            "option" => format!("Option<{}>", args[0]),
            "list" => format!("Vec<{}>", args[0]),
            "tuple" => format!("({})", args.join(",")),
            "result" => format!("Result<{}>", args.join(",")),
            _ => panic!("the drift test does not know the WIT type {wit:?}: teach `rust_type`"),
        };
    }
    match wit {
        "string" => "String".to_owned(),
        "bool" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" | "char" => wit.to_owned(),
        "s8" | "s16" | "s32" | "s64" => format!("i{}", &wit[1..]),
        w if w.chars().all(|c| c.is_ascii_lowercase() || c == '-') && !w.is_empty() => pascal(w),
        _ => panic!("the drift test does not know the WIT type {wit:?}: teach `rust_type`"),
    }
}

/// One function: `name(param: type, ...) -> ret`, all normalised.
#[derive(Debug, PartialEq, Eq)]
struct Func {
    name: String,
    params: Vec<(String, String)>,
    ret: String,
}

fn strip_spaces(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// What one interface holds.
#[derive(Debug, Default, PartialEq, Eq)]
struct Interface {
    funcs: Vec<Func>,
    /// `enum` and `variant` types: `Name` -> cases, each `Case` or `Case(Type)`.
    types: Vec<(String, Vec<String>)>,
}

fn parse_wit(wit: &str) -> Vec<(String, Interface)> {
    // Remove comments (every `//` in this file starts a comment).
    let code: String = wit
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find("interface ") {
        rest = &rest[at + "interface ".len()..];
        let open = rest.find('{').expect("interface without body");
        let name = rest[..open].trim().to_owned();
        let mut depth = 0;
        let mut end = open;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push((name, parse_wit_body(&rest[open + 1..end])));
        rest = &rest[end..];
    }
    out
}

fn parse_wit_body(body: &str) -> Interface {
    let mut iface = Interface::default();
    // Items end at `;` or at the `}` of an `enum` or a `variant`.
    let mut s = body.trim();
    while !s.is_empty() {
        if s.starts_with("use ") {
            s = s[s.find(';').expect("use without ;") + 1..].trim();
        } else if s.starts_with("enum ") || s.starts_with("variant ") {
            let open = s.find('{').expect("type without body");
            let close = s.find('}').expect("type without end");
            let head: Vec<&str> = s[..open].split_whitespace().collect();
            let cases = split_top(&s[open + 1..close])
                .into_iter()
                .map(|c| match c.find('(') {
                    Some(p) => format!("{}({})", pascal(c[..p].trim()), pascal(c[p + 1..].trim_end_matches(')').trim())),
                    None => pascal(c.trim()),
                })
                .collect();
            iface.types.push((pascal(head[1]), cases));
            s = s[close + 1..].trim();
        } else {
            let end = s.find(';').expect("item without ;");
            iface.funcs.push(parse_wit_func(&s[..end]));
            s = s[end + 1..].trim();
        }
    }
    iface
}

fn parse_wit_func(item: &str) -> Func {
    let (name, rest) = item.split_once(':').expect("function without :");
    let rest = rest.trim().strip_prefix("func").expect("not a func").trim();
    let close = rest.rfind(')').expect("func without )");
    let params = split_top(&rest[1..close])
        .into_iter()
        .map(|p| {
            let (n, t) = p.split_once(':').expect("param without :");
            (snake(n.trim()), strip_spaces(&rust_type(t)))
        })
        .collect();
    let ret = match rest[close + 1..].trim().strip_prefix("->") {
        Some(t) => strip_spaces(&rust_type(t)),
        None => "()".to_owned(),
    };
    Func { name: snake(name.trim()), params, ret }
}

/// The interfaces of the hand-written file.
fn parse_rust(src: &str) -> Vec<(String, Interface)> {
    let mut out: Vec<(String, Interface)> = Vec::new();
    let mut lines = src.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        if let Some(name) = line.strip_prefix("pub mod ").and_then(|l| l.strip_suffix(" {")) {
            if name == "plinth" || name == "app" {
                continue;
            }
            out.push((name.replace('_', "-"), Interface::default()));
        } else if let Some(name) = line.strip_prefix("pub enum ").and_then(|l| l.strip_suffix(" {")) {
            let mut cases = Vec::new();
            for l in lines.by_ref() {
                if l == "}" {
                    break;
                }
                cases.push(strip_spaces(l.trim_end_matches(',')));
            }
            out.last_mut().expect("enum outside a module").1.types.push((name.to_owned(), cases));
        } else if let Some(sig) = line.strip_prefix("fn ") {
            let sig = sig.strip_suffix(';').expect("fn without ;");
            let open = sig.find('(').unwrap();
            // The `)` that closes the parameter list (the return type may hold `()`).
            let close = {
                let mut depth = 0;
                sig.char_indices()
                    .find(|&(_, c)| {
                        depth += (c == '(') as i32 - (c == ')') as i32;
                        depth == 0 && c == ')'
                    })
                    .map(|(i, _)| i)
                    .unwrap()
            };
            let mut parts = split_top(&sig[open + 1..close]);
            assert_eq!(parts.remove(0), "&mut self", "fn {sig} must take &mut self");
            let params = parts
                .into_iter()
                .map(|p| {
                    let (n, t) = p.split_once(':').expect("param without :");
                    (n.trim().to_owned(), strip_spaces(t))
                })
                .collect();
            let ret = match sig[close + 1..].trim().strip_prefix("->") {
                Some(t) => strip_spaces(t),
                None => "()".to_owned(),
            };
            out.last_mut().expect("fn outside a module").1.funcs.push(Func { name: sig[..open].to_owned(), params, ret });
        }
    }
    out
}

/// Sorts interfaces, functions and types by name: the order of the items is
/// not part of the contract.
fn sorted(mut v: Vec<(String, Interface)>) -> Vec<(String, Interface)> {
    v.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, i) in &mut v {
        i.funcs.sort_by(|a, b| a.name.cmp(&b.name));
        i.types.sort();
    }
    v
}

#[test]
fn host_traits_match_the_wit() {
    let wit = sorted(parse_wit(&read("../../wit/plinth/app.wit")));
    let rust = sorted(parse_rust(&read("src/host_traits.rs")));

    let wit_names: Vec<&String> = wit.iter().map(|(n, _)| n).collect();
    let rust_names: Vec<&String> = rust.iter().map(|(n, _)| n).collect();
    assert_eq!(
        wit_names, rust_names,
        "the interfaces of app.wit and of host_traits.rs differ: change src/host_traits.rs"
    );
    for ((name, w), (_, r)) in wit.iter().zip(&rust) {
        assert_eq!(
            w.types, r.types,
            "interface `{name}`: the enum or variant types differ between app.wit and host_traits.rs"
        );
        assert_eq!(
            w.funcs, r.funcs,
            "interface `{name}`: the functions differ between app.wit (left) and host_traits.rs (right)"
        );
    }
    // The tables are not empty: the parser really read the files.
    assert!(wit.len() >= 14, "the parser read too few interfaces: {wit_names:?}");
    let count: usize = wit.iter().map(|(_, i)| i.funcs.len()).sum();
    assert!(count >= 50, "the parser read too few functions: {count}");
}
