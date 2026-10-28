//! Hostile and random input for `plinth:secrets` (`docs/SECRETS.md` §8), at
//! the level of the runner functions `secrets::{set, get, remove, names}`.
//! This is the style of `plinth-ui/tests/hostile_ops.rs`: a fixed seed, a
//! small number of iterations for the gate, and an environment variable for a
//! long run.
//!
//! - `PLINTH_FUZZ_ITERS` sets the number of random calls (default 3000).
//! - `PLINTH_FUZZ_SEED` sets the seed.
//!
//! The tests give random and hostile names and values to the functions
//! (NUL, the edges of the limits, `/`, `..`, `~`, `__index__`, characters
//! from many scripts), and compare each answer with a model that is written
//! separately from the code under test. They check that:
//! - nothing panics;
//! - the limits hold exactly (64 characters, 16 KiB, 64 secrets);
//! - apps do not see each other (many scopes, same names);
//! - `names()` is always sorted and equal to the items;
//! - a backend that fails never puts a value in a completion;
//! - a call that the policy denies never reaches the backend.

use plinth_protocol::Value;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::secrets::{
    self, MAX_SECRETS, MAX_VALUE_BYTES, MemorySecrets, SECRETS, SecretsBackend, SecretsError, SecretsScope,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

// -- Helpers ---------------------------------------------------------------

/// SplitMix64: a small fixed-seed generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn iterations() -> usize {
    std::env::var("PLINTH_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000)
}

fn seed() -> u64 {
    std::env::var("PLINTH_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x5EC2_E75)
}

const NAME_CHARS: &[char] = &['a', 'z', 'A', 'Z', '0', '9', '.', '_', '-'];
/// Characters that a name must not have, from many classes.
const HOSTILE_CHARS: &[char] = &[
    '/', '\\', '~', '^', ':', '*', '?', '"', '<', '>', '|', ' ', '\t', '\n', '\r', '\0', '\u{7f}', '\u{85}', '\u{a0}', 'é', 'ß', 'İ',
    'ı', '日', '本', 'א', 'ع', '\u{0301}', '\u{200b}', '\u{202e}', '\u{feff}', '\u{fffd}', '😀', '𝔸', '\u{10ffff}',
];
const SPECIAL_NAMES: &[&str] = &[
    "/", "//", "..", ".", "...", "../..", "../x", "a/../b", "~", "~1", "x~1", "x~", "__index__", "__index__~1", "_", "-", "_-.", "CON", "NUL",
    "com1", "aux", "a b", " a", "a ", "\0", "a\0", "\0a", "%00", "%2e%2e", "a\u{0301}", "\u{e9}", "A", "a",
];

/// A name with 1 to 64 characters from the allowed set, or one of the edges.
fn valid_name(rng: &mut Rng) -> String {
    let len = *rng.pick(&[1, 2, 3, 10, 31, 32, 33, 63, 64, 5, 8]);
    (0..len).map(|_| *rng.pick(NAME_CHARS)).collect()
}

fn hostile_name(rng: &mut Rng) -> String {
    match rng.below(7) {
        0 => rng.pick(SPECIAL_NAMES).to_string(),
        // A valid name with one hostile character in it.
        1 => {
            let mut chars: Vec<char> = valid_name(rng).chars().collect();
            let at = rng.below(chars.len() + 1);
            chars.insert(at, *rng.pick(HOSTILE_CHARS));
            chars.into_iter().collect()
        }
        // The edge of the length limit, with allowed characters or not.
        2 => (0..*rng.pick(&[0usize, 63, 64, 65, 66, 128, 1000, 10_000])).map(|_| *rng.pick(NAME_CHARS)).collect(),
        3 => (0..*rng.pick(&[64usize, 65])).map(|_| *rng.pick(HOSTILE_CHARS)).collect(),
        // 64 characters that take more than 64 bytes: the limit counts characters.
        4 => "é".repeat(*rng.pick(&[1usize, 64, 65])),
        _ => valid_name(rng),
    }
}

/// The rule of `docs/SECRETS.md` §2, written again here and not taken from the runner.
fn name_ok(name: &str) -> bool {
    let n = name.chars().count();
    (1..=64).contains(&n) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// A value of random length and content, with a canary at the start so that
/// a leak is easy to see.
fn value(rng: &mut Rng, canary: &str) -> String {
    let target = match rng.below(8) {
        0 => 0,
        1 => 1,
        2 => MAX_VALUE_BYTES - 1,
        3 => MAX_VALUE_BYTES,
        4 => MAX_VALUE_BYTES + 1,
        5 => rng.below(MAX_VALUE_BYTES * 2),
        _ => rng.below(300),
    };
    let mut s = String::from(canary);
    while s.len() < target {
        let c = match rng.below(6) {
            0 => '\0',
            1 => *rng.pick(HOSTILE_CHARS),
            2 => char::from_u32(rng.below(0xD800) as u32).unwrap_or('?'),
            3 => char::from_u32(0xE000 + rng.below(0x1_0000) as u32).unwrap_or('?'),
            _ => (b' ' + rng.below(95) as u8) as char,
        };
        if s.len() + c.len_utf8() > target {
            break;
        }
        s.push(c);
    }
    // The edge of the limit is exact: pad with ASCII.
    if target >= MAX_VALUE_BYTES - 1 && s.len() < target {
        s.extend(std::iter::repeat_n('p', target - s.len()));
    }
    s
}

fn ok(n: i32, text: &str) -> Value {
    Value::List(vec![Value::Bool(true), Value::Int(n), Value::Str(text.to_owned()), Value::Null])
}

fn denied(error: &str) -> Value {
    Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(error.to_owned())])
}

/// A backend that can fail, count its calls and remember nothing it fails to store.
#[derive(Clone)]
struct Flaky {
    inner: MemorySecrets,
    state: Arc<Mutex<FlakyState>>,
}

#[derive(Default)]
struct FlakyState {
    /// 0: never fail. n: a call fails when the counter is a multiple of n.
    fail_every: u64,
    unsupported: bool,
    calls: u64,
    failures: u64,
}

impl Flaky {
    fn new() -> Self {
        Self { inner: MemorySecrets::new(), state: Arc::default() }
    }
    fn configure(&self, fail_every: u64, unsupported: bool) {
        let mut s = self.state.lock().unwrap();
        s.fail_every = fail_every;
        s.unsupported = unsupported;
    }
    fn calls(&self) -> u64 {
        self.state.lock().unwrap().calls
    }
    fn failures(&self) -> u64 {
        self.state.lock().unwrap().failures
    }
    /// `Err` when this call must fail. The error text holds `what`, which the
    /// caller makes from a value: a leak of the error text would show it.
    fn gate(&self, what: &str) -> Result<(), SecretsError> {
        let mut s = self.state.lock().unwrap();
        s.calls += 1;
        if s.unsupported {
            return Err(SecretsError::Unsupported);
        }
        if s.fail_every != 0 && s.calls % s.fail_every == 0 {
            s.failures += 1;
            return Err(SecretsError::Failed(format!("the store failed for {what}")));
        }
        Ok(())
    }
}

impl SecretsBackend for Flaky {
    fn set(&mut self, scope: &SecretsScope, name: &str, v: &str) -> Result<(), SecretsError> {
        self.gate(v)?;
        self.inner.set(scope, name, v)
    }
    fn get(&mut self, scope: &SecretsScope, name: &str) -> Result<Option<String>, SecretsError> {
        self.gate(name)?;
        self.inner.get(scope, name)
    }
    fn remove(&mut self, scope: &SecretsScope, name: &str) -> Result<(), SecretsError> {
        self.gate(name)?;
        self.inner.remove(scope, name)
    }
    fn names(&mut self, scope: &SecretsScope) -> Result<Vec<String>, SecretsError> {
        self.gate("names")?;
        self.inner.names(scope)
    }
}

/// True when `result` has `ok` false (a failure completion).
fn is_failure(result: &Value) -> bool {
    matches!(result, Value::List(items) if items.first() == Some(&Value::Bool(false)))
}

/// Checks the shape of a completion: four elements, `ok`, `n`, `text`, `error`.
fn assert_shape(result: &Value, what: &str) {
    let Value::List(items) = result else { panic!("{what}: the completion is not a list: {result:?}") };
    assert_eq!(items.len(), 4, "{what}: {result:?}");
    let ok_flag = matches!(items[0], Value::Bool(true));
    assert!(matches!(items[0], Value::Bool(_)) && matches!(items[1], Value::Int(_)) && matches!(items[2], Value::Str(_)), "{what}: {result:?}");
    if ok_flag {
        assert_eq!(items[3], Value::Null, "{what}: {result:?}");
    } else {
        assert!(matches!(&items[3], Value::Str(e) if e.starts_with("denied:") || e == "failed"), "{what}: {result:?}");
        assert_eq!(items[2], Value::Str(String::new()), "{what}: a failure has no text: {result:?}");
    }
}

fn names_of(result: &Value) -> (i32, Vec<String>) {
    let Value::List(items) = result else { panic!("{result:?}") };
    let (Value::Int(n), Value::Str(text)) = (&items[1], &items[2]) else { panic!("{result:?}") };
    (*n, serde_json::from_str(text).unwrap_or_else(|e| panic!("names text is not JSON ({e}): {text:?}")))
}

// -- Tests -----------------------------------------------------------------

/// Random calls for several apps with the same names, against a model.
#[test]
fn random_calls_match_the_model_and_keep_every_limit() {
    let mut rng = Rng(seed());
    let policy = Policy::new([SECRETS]);
    let mut backend = Flaky::new();
    let scopes: Vec<SecretsScope> = (0..4).map(|i| SecretsScope::new(format!("app.{i}"))).collect();
    let mut model: Vec<BTreeMap<String, String>> = vec![BTreeMap::new(); scopes.len()];
    // A small pool of names, so that names repeat, and a few hostile names.
    let pool: Vec<String> = (0..70).map(|i| format!("secret-{i}")).chain(["a", "A", "x.y", "_", "-"].map(String::from)).collect();

    for step in 0..iterations() {
        let s = rng.below(scopes.len());
        let scope = &scopes[s];
        let name = if rng.below(4) == 0 { hostile_name(&mut rng) } else { rng.pick(&pool).clone() };
        let what = format!("step {step}, scope {s}, name {name:?}");
        match rng.below(10) {
            0..=3 => {
                let v = value(&mut rng, "CANARY-");
                let result = secrets::set(&policy, &mut backend, scope, &name, &v);
                assert_shape(&result, &what);
                let expected = if !name_ok(&name) || v.len() > 16 * 1024 {
                    denied("denied:limit")
                } else if model[s].len() >= 64 && !model[s].contains_key(&name) {
                    denied("denied:limit")
                } else {
                    model[s].insert(name.clone(), v.clone());
                    ok(0, "")
                };
                assert_eq!(result, expected, "{what}: set, value of {} bytes", v.len());
            }
            4..=6 => {
                let result = secrets::get(&policy, &mut backend, scope, &name);
                assert_shape(&result, &what);
                let expected = if !name_ok(&name) {
                    denied("denied:limit")
                } else {
                    match model[s].get(&name) {
                        Some(v) => ok(1, v),
                        None => ok(0, ""),
                    }
                };
                assert_eq!(result, expected, "{what}: get");
            }
            7..=8 => {
                let result = secrets::remove(&policy, &mut backend, scope, &name);
                assert_shape(&result, &what);
                let expected = if !name_ok(&name) {
                    denied("denied:limit")
                } else {
                    model[s].remove(&name);
                    ok(0, "")
                };
                assert_eq!(result, expected, "{what}: remove");
            }
            _ => {
                let result = secrets::names(&policy, &mut backend, scope);
                assert_shape(&result, &what);
                let (n, names) = names_of(&result);
                assert_eq!(n as usize, names.len(), "{what}: the count");
                assert!(names.windows(2).all(|w| w[0] < w[1]), "{what}: names are not sorted: {names:?}");
                assert_eq!(names, model[s].keys().cloned().collect::<Vec<_>>(), "{what}: names");
            }
        }
        assert!(model[s].len() <= MAX_SECRETS, "{what}: more than 64 secrets");
    }

    // At the end, each scope holds exactly its model, in the backend too.
    let mut direct = backend.inner.clone();
    for (i, scope) in scopes.iter().enumerate() {
        assert_eq!(direct.names(scope).unwrap(), model[i].keys().cloned().collect::<Vec<_>>());
        for (k, v) in &model[i] {
            assert_eq!(direct.get(scope, k).unwrap().as_ref(), Some(v));
        }
    }
}

/// The edges of every limit, one by one.
#[test]
fn the_limits_hold_exactly() {
    let policy = Policy::new([SECRETS]);
    let mut b = MemorySecrets::new();
    let scope = SecretsScope::new("app.one");
    let limit = denied("denied:limit");

    // Names: 1 and 64 characters work; 0 and 65 do not; the count is of characters.
    for n in [1usize, 2, 63, 64] {
        assert_eq!(secrets::set(&policy, &mut b, &scope, &"n".repeat(n), "v"), ok(0, ""), "{n} characters");
    }
    for n in [0usize, 65, 66, 1000] {
        assert_eq!(secrets::set(&policy, &mut b, &scope, &"n".repeat(n), "v"), limit, "{n} characters");
        assert_eq!(secrets::get(&policy, &mut b, &scope, &"n".repeat(n)), limit);
        assert_eq!(secrets::remove(&policy, &mut b, &scope, &"n".repeat(n)), limit);
    }
    for bad in SPECIAL_NAMES.iter().filter(|n| !name_ok(n)) {
        assert_eq!(secrets::set(&policy, &mut b, &scope, bad, "v"), limit, "{bad:?}");
        assert_eq!(secrets::get(&policy, &mut b, &scope, bad), limit, "{bad:?}");
        assert_eq!(secrets::remove(&policy, &mut b, &scope, bad), limit, "{bad:?}");
    }
    // The names that are fine and look odd.
    for fine in ["..", ".", "...", "_", "-", "_-.", "CON", "A"] {
        assert_eq!(secrets::set(&policy, &mut b, &scope, fine, "v"), ok(0, ""), "{fine:?}");
        assert_eq!(secrets::get(&policy, &mut b, &scope, fine), ok(1, "v"), "{fine:?}");
    }
    // `__index__` is a valid name for the runner. Its backend must keep a value apart from its own index.
    assert_eq!(secrets::set(&policy, &mut b, &scope, "__index__", "v"), ok(0, ""));
    assert_eq!(secrets::get(&policy, &mut b, &scope, "__index__"), ok(1, "v"));

    // Values: 16 KiB works, 16 KiB + 1 does not, the size is in bytes.
    let mut b = MemorySecrets::new();
    for n in [0usize, 1, MAX_VALUE_BYTES - 1, MAX_VALUE_BYTES] {
        let v = "x".repeat(n);
        assert_eq!(secrets::set(&policy, &mut b, &scope, "v", &v), ok(0, ""), "{n} bytes");
        assert_eq!(secrets::get(&policy, &mut b, &scope, "v"), ok(1, &v), "{n} bytes");
    }
    assert_eq!(secrets::set(&policy, &mut b, &scope, "v", &"x".repeat(MAX_VALUE_BYTES + 1)), limit);
    // A failed replace keeps the old value.
    assert_eq!(secrets::get(&policy, &mut b, &scope, "v"), ok(1, &"x".repeat(MAX_VALUE_BYTES)));
    let two_bytes = "é".repeat(MAX_VALUE_BYTES / 2);
    assert_eq!(secrets::set(&policy, &mut b, &scope, "w", &two_bytes), ok(0, ""));
    assert_eq!(secrets::set(&policy, &mut b, &scope, "w", &format!("{two_bytes}x")), limit);
    let four_bytes = "😀".repeat(MAX_VALUE_BYTES / 4);
    assert_eq!(secrets::set(&policy, &mut b, &scope, "e", &four_bytes), ok(0, ""));
    assert_eq!(secrets::get(&policy, &mut b, &scope, "e"), ok(1, &four_bytes));
    assert_eq!(secrets::set(&policy, &mut b, &scope, "e", &format!("{four_bytes}\0")), limit);
    // NUL is a character like another one.
    assert_eq!(secrets::set(&policy, &mut b, &scope, "nul", "a\0b\0"), ok(0, ""));
    assert_eq!(secrets::get(&policy, &mut b, &scope, "nul"), ok(1, "a\0b\0"));

    // Count: 64 secrets work, the 65th does not, a replace and a remove still work.
    let mut b = MemorySecrets::new();
    for i in 0..64 {
        assert_eq!(secrets::set(&policy, &mut b, &scope, &format!("s{i}"), "v"), ok(0, ""));
    }
    assert_eq!(secrets::set(&policy, &mut b, &scope, "s64", "v"), limit);
    assert_eq!(secrets::set(&policy, &mut b, &scope, "s63", "new"), ok(0, ""));
    assert_eq!(secrets::set(&policy, &mut b, &scope, "S63", "case"), limit, "a name with another case is a new name");
    assert_eq!(secrets::remove(&policy, &mut b, &scope, "s0"), ok(0, ""));
    assert_eq!(secrets::set(&policy, &mut b, &scope, "s64", "v"), ok(0, ""));
    let (n, names) = names_of(&secrets::names(&policy, &mut b, &scope));
    assert_eq!((n, names.len()), (64, 64));
    // Another app has its own count.
    assert_eq!(secrets::set(&policy, &mut b, &SecretsScope::new("app.two"), "s64", "v"), ok(0, ""));
    // The instance is part of the scope.
    let other_instance = SecretsScope { app: "app.one".to_owned(), instance: "other".to_owned() };
    assert_eq!(secrets::get(&policy, &mut b, &other_instance, "s64"), ok(0, ""));
}

/// Set and remove the same names again and again: no growth, no stale value.
#[test]
fn repeated_set_and_remove_cycles_leave_no_trace() {
    let policy = Policy::new([SECRETS]);
    let mut b = MemorySecrets::new();
    let scope = SecretsScope::new("app.one");
    for cycle in 0..200 {
        assert_eq!(secrets::set(&policy, &mut b, &scope, "t", &format!("value-{cycle}")), ok(0, ""));
        assert_eq!(secrets::get(&policy, &mut b, &scope, "t"), ok(1, &format!("value-{cycle}")));
        assert_eq!(secrets::remove(&policy, &mut b, &scope, "t"), ok(0, ""));
        assert_eq!(secrets::get(&policy, &mut b, &scope, "t"), ok(0, ""));
        assert_eq!(secrets::names(&policy, &mut b, &scope), ok(0, "[]"));
    }
    // Fill to the limit and clear, many times: the limit is the same each time.
    for round in 0..5 {
        for i in 0..64 {
            assert_eq!(secrets::set(&policy, &mut b, &scope, &format!("k{i}"), &format!("{round}")), ok(0, ""), "round {round}");
        }
        assert_eq!(secrets::set(&policy, &mut b, &scope, "extra", "v"), denied("denied:limit"), "round {round}");
        for i in 0..64 {
            assert_eq!(secrets::remove(&policy, &mut b, &scope, &format!("k{i}")), ok(0, ""));
        }
        assert_eq!(secrets::names(&policy, &mut b, &scope), ok(0, "[]"), "round {round}");
    }
}

/// Many apps with the same names: no app sees another one.
#[test]
fn many_apps_with_the_same_names_stay_apart() {
    let policy = Policy::new([SECRETS]);
    let mut b = MemorySecrets::new();
    let apps: Vec<SecretsScope> = (0..50).map(|i| SecretsScope::new(format!("app.{i}"))).collect();
    // App ids that could be mistaken for each other by a careless key.
    let tricky: Vec<SecretsScope> = ["a/b", "a", "a/b/default", "a.b", "A", "", "..", "a\0b"]
        .iter()
        .map(|s| SecretsScope::new(*s))
        .chain([SecretsScope { app: "a".into(), instance: "b/default".into() }])
        .collect();
    let all: Vec<&SecretsScope> = apps.iter().chain(&tricky).collect();
    for (i, scope) in all.iter().enumerate() {
        for name in ["token", "Token", "a", "b.c"] {
            assert_eq!(secrets::set(&policy, &mut b, scope, name, &format!("{i}:{name}")), ok(0, ""));
        }
    }
    for (i, scope) in all.iter().enumerate() {
        for name in ["token", "Token", "a", "b.c"] {
            assert_eq!(secrets::get(&policy, &mut b, scope, name), ok(1, &format!("{i}:{name}")), "{scope:?} {name}");
        }
        assert_eq!(secrets::names(&policy, &mut b, scope), ok(4, r#"["Token","a","b.c","token"]"#));
    }
    for scope in all.iter().step_by(2) {
        for name in ["token", "Token", "a", "b.c"] {
            secrets::remove(&policy, &mut b, scope, name);
        }
    }
    for (i, scope) in all.iter().enumerate() {
        let expected = if i % 2 == 0 { ok(0, "[]") } else { ok(4, r#"["Token","a","b.c","token"]"#) };
        assert_eq!(secrets::names(&policy, &mut b, scope), expected, "{scope:?}");
    }
}

/// A backend that fails: the completion has no value, no error text of the
/// backend, and a failed call does not change the state.
#[test]
fn a_failing_backend_never_leaks_a_value() {
    let mut rng = Rng(seed() ^ 0xFA11);
    let policy = Policy::new([SECRETS]);
    let mut backend = Flaky::new();
    let scope = SecretsScope::new("app.one");
    let canary = "LEAK-CANARY-9c1d";
    let dump = |b: &Flaky| -> BTreeMap<String, String> {
        let mut inner = b.inner.clone();
        inner.names(&scope).unwrap().into_iter().map(|n| (n.clone(), inner.get(&scope, &n).unwrap().unwrap())).collect()
    };
    for step in 0..iterations() {
        // Every 3rd to 5th backend call fails.
        backend.configure(3 + rng.below(3) as u64, false);
        let name = rng.pick(&["a", "b", "c", "d", "e", "f"]).to_string();
        let v = value(&mut rng, canary);
        let before = dump(&backend);
        let what = format!("step {step}, name {name:?}");
        let kind = rng.below(4);
        let result = match kind {
            0 | 1 => secrets::set(&policy, &mut backend, &scope, &name, &v),
            2 => secrets::get(&policy, &mut backend, &scope, &name),
            _ => secrets::remove(&policy, &mut backend, &scope, &name),
        };
        assert_shape(&result, &what);
        let text = format!("{result:?}");
        let after = dump(&backend);
        let mut expected = before.clone();
        if is_failure(&result) {
            // A failure names no value, and not the text of the backend.
            assert!(!text.contains(canary), "{what}: a failed call leaks the value: {text}");
            assert!(!text.contains("the store failed"), "{what}: the backend text is in the completion: {text}");
            if v.len() <= MAX_VALUE_BYTES {
                assert!(text.contains("\"failed\"") || text.contains("denied:limit"), "{what}: {text}");
            }
        } else {
            match kind {
                0 | 1 => {
                    expected.insert(name.clone(), v.clone());
                }
                3 => {
                    expected.remove(&name);
                }
                _ => {}
            }
        }
        if is_failure(&result) && text.contains("denied:limit") {
            // A value over 16 KiB: nothing changes.
            expected = before;
        }
        assert_eq!(after, expected, "{what}: a failed call changed the state, or a worked call did not");
    }
    assert!(backend.failures() > 0, "the test never made the backend fail");
    // A backend that is unsupported answers `denied:unsupported` and nothing else.
    backend.configure(0, true);
    for r in [
        secrets::set(&policy, &mut backend, &scope, "a", canary),
        secrets::get(&policy, &mut backend, &scope, "a"),
        secrets::remove(&policy, &mut backend, &scope, "a"),
        secrets::names(&policy, &mut backend, &scope),
    ] {
        assert_eq!(r, denied("denied:unsupported"));
    }
}

/// A call that the policy denies never reaches the backend, for any input.
#[test]
fn denied_calls_never_reach_the_backend() {
    let mut rng = Rng(seed() ^ 0xDE41);
    let mut undeclared = Policy::new(["store.kv"]);
    let mut refused = Policy::new([SECRETS]);
    refused.refuse(SECRETS);
    let mut unsupported = Policy::new([SECRETS]);
    unsupported.mark_unsupported(SECRETS);
    let _ = &mut undeclared;
    let backend = Flaky::new();
    let scope = SecretsScope::new("app.one");
    for (policy, error) in [(&undeclared, "denied:undeclared"), (&refused, "denied:refused"), (&unsupported, "denied:unsupported")] {
        let mut b = backend.clone();
        for _ in 0..(iterations() / 4).max(50) {
            // The policy answers first, also for a name or a value that is bad.
            let name = hostile_name(&mut rng);
            let v = value(&mut rng, "C");
            for r in [
                secrets::set(policy, &mut b, &scope, &name, &v),
                secrets::get(policy, &mut b, &scope, &name),
                secrets::remove(policy, &mut b, &scope, &name),
                secrets::names(policy, &mut b, &scope),
            ] {
                assert_eq!(r, denied(error));
            }
        }
    }
    assert_eq!(backend.calls(), 0, "a denied call reached the backend");
}

/// Strings that no app can make but a broken core could: the functions take
/// `&str`, so the host has checked the UTF-8 already. This test only runs the
/// strings that Rust accepts and that look like the hard cases of Unicode.
#[test]
fn hard_unicode_never_panics() {
    let policy = Policy::new([SECRETS]);
    let mut b = MemorySecrets::new();
    let scope = SecretsScope::new("app.one");
    let hard = [
        "\0",
        "\u{fffd}",
        "\u{feff}",
        "\u{202e}abc\u{202c}",
        "e\u{301}",
        "\u{10ffff}",
        "\u{d7ff}",
        "\u{e000}",
        "𝔸𝔹",
        "👨\u{200d}👩\u{200d}👧",
        "\r\n",
        "\u{85}",
    ];
    for h in hard {
        // As a name: always a limit error. As a value: stored and returned unchanged.
        assert_eq!(secrets::set(&policy, &mut b, &scope, h, "v"), denied("denied:limit"), "{h:?}");
        assert_eq!(secrets::get(&policy, &mut b, &scope, h), denied("denied:limit"));
        assert_eq!(secrets::remove(&policy, &mut b, &scope, h), denied("denied:limit"));
        assert_eq!(secrets::set(&policy, &mut b, &scope, "v", h), ok(0, ""), "{h:?}");
        assert_eq!(secrets::get(&policy, &mut b, &scope, "v"), ok(1, h), "{h:?}");
    }
    // A names text is JSON: every name is plain ASCII, so it needs no escape.
    let (_, names) = names_of(&secrets::names(&policy, &mut b, &scope));
    assert_eq!(names, ["v"]);
}
