//! These tests run the chunking, the index and the repair rules of
//! `secrets.rs` on an in-memory `ItemStore`. The double can fail after a
//! number of writes (a crash in the middle of a call), it can compare keys
//! without case (as the Windows Credential Manager does), and it refuses a
//! blob over 2560 bytes (the Windows limit). They need no keychain.
//! `PLINTH_FUZZ_ITERS` sets the number of random operations (default 400) and
//! `PLINTH_FUZZ_SEED` the seed.

use super::*;
use plinth_runner_wasmtime::secrets::MAX_VALUE_BYTES;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const WINDOWS_BLOB_LIMIT: usize = 2560;

#[derive(Default)]
struct State {
    items: BTreeMap<String, Vec<u8>>,
    /// The number of writes and deletes that still work. `None`: no limit.
    budget: Option<usize>,
    fail_reads: bool,
    ignore_case: bool,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<State>>);

impl Fake {
    fn windows_like() -> Self {
        let f = Self::default();
        f.0.lock().unwrap().ignore_case = true;
        f
    }
    fn set_budget(&self, budget: Option<usize>) {
        self.0.lock().unwrap().budget = budget;
    }
    fn keys(&self) -> Vec<String> {
        self.0.lock().unwrap().items.keys().cloned().collect()
    }
    fn put_raw(&self, scope: &SecretsScope, key: &str, bytes: &[u8]) {
        let mut state = self.0.lock().unwrap();
        let id = Self::id(&state, scope, key);
        state.items.insert(id, bytes.to_vec());
    }
    fn id(state: &State, scope: &SecretsScope, key: &str) -> String {
        let t = scope.target(key);
        if state.ignore_case { t.to_lowercase() } else { t }
    }
    fn spend(state: &mut State) -> Result<(), SecretsError> {
        match &mut state.budget {
            Some(0) => Err(SecretsError::Failed("injected failure".to_owned())),
            Some(n) => {
                *n -= 1;
                Ok(())
            }
            None => Ok(()),
        }
    }
}

impl ItemStore for Fake {
    fn read(&self, scope: &SecretsScope, key: &str) -> Result<Option<Vec<u8>>, SecretsError> {
        let state = self.0.lock().unwrap();
        if state.fail_reads {
            return Err(SecretsError::Failed("injected failure".to_owned()));
        }
        Ok(state.items.get(&Self::id(&state, scope, key)).cloned())
    }
    fn write(&self, scope: &SecretsScope, key: &str, bytes: &[u8]) -> Result<(), SecretsError> {
        let mut state = self.0.lock().unwrap();
        Self::spend(&mut state)?;
        assert!(bytes.len() <= WINDOWS_BLOB_LIMIT, "an item of {} bytes is over the Windows limit", bytes.len());
        let id = Self::id(&state, scope, key);
        state.items.insert(id, bytes.to_vec());
        Ok(())
    }
    fn delete(&self, scope: &SecretsScope, key: &str) -> Result<(), SecretsError> {
        let mut state = self.0.lock().unwrap();
        Self::spend(&mut state)?;
        let id = Self::id(&state, scope, key);
        state.items.remove(&id);
        Ok(())
    }
}

/// Threads that use the real keychain at the same time lose no secret. The
/// Credential Manager loses writes when threads call it at the same time, and
/// two `set` calls of one app race on the index. The lock `KEYCHAIN` prevents
/// both. The test skips when the host has no usable keychain.
#[test]
fn threads_at_the_same_time_lose_nothing() {
    let probe = format!("plinth.test.threads.{}.probe", std::process::id());
    let works = KeyringSecrets.set(&SecretsScope::new(&probe), "probe", "x").is_ok();
    let _ = remove_app_secrets(&probe);
    if !works {
        eprintln!("skip: no usable system keychain on this host");
        return;
    }
    // Two threads for each app (the same index), four apps.
    let apps: Vec<String> = (0..4).map(|n| format!("plinth.test.threads.{}.{n}", std::process::id())).collect();
    let handles: Vec<_> = apps
        .iter()
        .flat_map(|app| (0..2).map(move |t| (app.clone(), t)))
        .map(|(app, t)| {
            std::thread::spawn(move || {
                let scope = SecretsScope::new(&app);
                for i in 0..6 {
                    KeyringSecrets.set(&scope, &format!("t{t}-n{i}"), &format!("{app}:{t}:{i}")).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    for app in &apps {
        let scope = SecretsScope::new(app);
        assert_eq!(KeyringSecrets.names(&scope).unwrap().len(), 12, "{app}: the index lost a name");
        for t in 0..2 {
            for i in 0..6 {
                assert_eq!(KeyringSecrets.get(&scope, &format!("t{t}-n{i}")), Ok(Some(format!("{app}:{t}:{i}"))));
            }
        }
    }
    for app in &apps {
        remove_app_secrets(app).unwrap();
        assert_eq!(KeyringSecrets.names(&SecretsScope::new(app)), Ok(vec![]));
    }
}

fn scope() -> SecretsScope {
    SecretsScope::new("test.app")
}

fn backend() -> (Chunked<Fake>, Fake) {
    let fake = Fake::windows_like();
    (Chunked(fake.clone()), fake)
}

/// A string of exactly `bytes` bytes of UTF-8, that starts with `lead` ASCII
/// bytes and then has `ch` as often as it fits.
fn text_of(bytes: usize, lead: usize, ch: char) -> String {
    let mut s = "a".repeat(lead.min(bytes));
    while s.len() + ch.len_utf8() <= bytes {
        s.push(ch);
    }
    while s.len() < bytes {
        s.push('z');
    }
    assert_eq!(s.len(), bytes);
    s
}

/// A fixed-seed random number generator (SplitMix64).
fn rng(mut seed: u64) -> impl FnMut() -> u64 {
    move || {
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[test]
fn values_round_trip_at_every_chunk_boundary() {
    let (mut b, _) = backend();
    let sizes = [0, 1, 2047, 2048, 2049, 2050, 4095, 4096, 4097, 8191, 8192, 8193, 16383, MAX_VALUE_BYTES];
    for &size in &sizes {
        for ch in ['a', 'é', '日', '😀'] {
            // `lead` moves the multi-byte characters over the boundary.
            for lead in 0..4 {
                let v = text_of(size, lead, ch);
                b.set(&scope(), "v", &v).unwrap();
                assert_eq!(b.get(&scope(), "v").unwrap().as_deref(), Some(v.as_str()), "size {size} lead {lead} {ch:?}");
            }
        }
    }
}

#[test]
fn a_value_over_the_limit_fails_and_leaves_no_trace() {
    let (mut b, fake) = backend();
    b.set(&scope(), "keep", "old").unwrap();
    let before = fake.keys();
    let too_long = "x".repeat(MAX_VALUE_BYTES + 1);
    assert!(matches!(b.set(&scope(), "new", &too_long), Err(SecretsError::Failed(_))));
    assert_eq!(fake.keys(), before, "a failed set left an item or an index entry");
    assert_eq!(b.names(&scope()).unwrap(), vec!["keep".to_owned()]);
    // A replace that fails keeps the old value.
    assert!(b.set(&scope(), "keep", &too_long).is_err());
    assert_eq!(b.get(&scope(), "keep").unwrap().as_deref(), Some("old"));
}

#[test]
fn a_shorter_value_leaves_no_old_chunk() {
    let (mut b, fake) = backend();
    let item_count = |f: &Fake| f.keys().iter().filter(|k| k.ends_with("/v") || k.contains("/v~")).count();
    b.set(&scope(), "v", &"x".repeat(MAX_VALUE_BYTES)).unwrap();
    assert_eq!(item_count(&fake), 8);
    b.set(&scope(), "v", &"y".repeat(5000)).unwrap();
    assert_eq!(item_count(&fake), 3);
    b.set(&scope(), "v", "z").unwrap();
    assert_eq!(item_count(&fake), 1);
    assert_eq!(b.get(&scope(), "v").unwrap().as_deref(), Some("z"));
    b.remove(&scope(), "v").unwrap();
    assert_eq!(item_count(&fake), 0);
}

#[test]
fn a_chunk_name_is_not_a_secret() {
    let (mut b, fake) = backend();
    b.set(&scope(), "x", &"c".repeat(5000)).unwrap();
    // A caller that does not use the runner cannot reach the chunks of `x`.
    for bad in ["x~1", "x~2", "x~", "~1", "^x", "X~1", "__index__", "__index__~1", "", "a/b", "a b", "../x", "a\0b"] {
        assert!(b.set(&scope(), bad, "evil").is_err(), "set {bad:?}");
        assert_eq!(b.get(&scope(), bad), Ok(None), "get {bad:?}");
        assert_eq!(b.remove(&scope(), bad), Ok(()), "remove {bad:?}");
    }
    assert_eq!(b.get(&scope(), "x").unwrap().unwrap(), "c".repeat(5000));
    assert_eq!(b.names(&scope()).unwrap(), vec!["x".to_owned()]);
    assert_eq!(fake.keys().len(), 3 + 1, "3 chunks of x and the index");
}

#[test]
fn names_that_differ_only_in_case_stay_apart() {
    // The double compares keys without case, like the Credential Manager.
    let (mut b, fake) = backend();
    b.set(&scope(), "Token", "upper").unwrap();
    b.set(&scope(), "token", "lower").unwrap();
    b.set(&scope(), "TOKEN", "caps").unwrap();
    assert_eq!(b.get(&scope(), "Token").unwrap().as_deref(), Some("upper"));
    assert_eq!(b.get(&scope(), "token").unwrap().as_deref(), Some("lower"));
    assert_eq!(b.get(&scope(), "TOKEN").unwrap().as_deref(), Some("caps"));
    assert_eq!(b.names(&scope()).unwrap(), vec!["TOKEN".to_owned(), "Token".to_owned(), "token".to_owned()]);
    b.remove(&scope(), "Token").unwrap();
    assert_eq!(b.get(&scope(), "token").unwrap().as_deref(), Some("lower"));
    assert_eq!(b.get(&scope(), "TOKEN").unwrap().as_deref(), Some("caps"));
    // A double that keeps the case shows the real keys: none has an uppercase letter.
    let strict = Fake::default();
    let mut s = Chunked(strict.clone());
    for name in ["Token", "token", "TOKEN"] {
        s.set(&scope(), name, "v").unwrap();
    }
    assert!(strict.keys().iter().all(|k| *k == k.to_lowercase()), "{:?}", strict.keys());
    assert_eq!(strict.keys().len(), 4, "3 secrets and the index");
    drop(fake);
}

#[test]
fn the_item_key_has_no_uppercase_and_cannot_collide() {
    let names = ["a", "A", "aa", "aA", "Aa", "AA", "a.B", "a.b", "a-B", "x1", "X1"];
    let keys: Vec<String> = names.iter().map(|n| chunk_key(n, 0)).collect();
    for (i, a) in keys.iter().enumerate() {
        assert_eq!(*a, a.to_lowercase(), "the key of {} has an uppercase letter", names[i]);
        for (j, c) in keys.iter().enumerate() {
            assert!(i == j || a != c, "{} and {} collide", names[i], names[j]);
        }
    }
    // No chunk key of one name equals the key of another valid name.
    for n in names {
        for i in 1..MAX_CHUNKS {
            assert!(!names.iter().any(|m| chunk_key(m, 0) == chunk_key(n, i)));
        }
    }
}

#[test]
fn a_crash_in_the_middle_of_a_replace_never_gives_a_wrong_value() {
    let old = "o".repeat(5000);
    let new = "n".repeat(5000);
    for budget in 0..8 {
        let (mut b, fake) = backend();
        b.set(&scope(), "v", &old).unwrap();
        fake.set_budget(Some(budget));
        let result = b.set(&scope(), "v", &new);
        fake.set_budget(None);
        match b.get(&scope(), "v") {
            Ok(Some(v)) => assert!(v == old || v == new, "budget {budget}: a mix of old and new ({} bytes)", v.len()),
            Ok(None) => panic!("budget {budget}: the secret is gone"),
            Err(SecretsError::Failed(_)) => assert!(result.is_err(), "budget {budget}: a set that worked left a bad value"),
            Err(e) => panic!("budget {budget}: {e:?}"),
        }
        // The next set repairs it.
        b.set(&scope(), "v", &new).unwrap();
        assert_eq!(b.get(&scope(), "v").unwrap().as_deref(), Some(new.as_str()));
        assert_eq!(b.names(&scope()).unwrap(), vec!["v".to_owned()]);
    }
}

#[test]
fn a_crash_in_a_new_secret_never_leaves_an_item_the_index_does_not_list() {
    let value = "n".repeat(5000);
    for budget in 0..8 {
        let (b, fake) = backend();
        let mut b = b;
        b.set(&scope(), "other", "1").unwrap();
        fake.set_budget(Some(budget));
        let _ = b.set(&scope(), "Fresh", &value);
        fake.set_budget(None);
        // The uninstall removes everything that the index lists. Nothing may remain.
        remove_all(&b, &scope()).unwrap();
        assert_eq!(fake.keys(), Vec::<String>::new(), "budget {budget}: an item remained after the uninstall");
    }
}

#[test]
fn a_crash_in_a_remove_never_gives_a_wrong_value() {
    let value = "r".repeat(5000);
    for budget in 0..10 {
        let (mut b, fake) = backend();
        b.set(&scope(), "a", "keep").unwrap();
        b.set(&scope(), "v", &value).unwrap();
        fake.set_budget(Some(budget));
        let _ = b.remove(&scope(), "v");
        fake.set_budget(None);
        match b.get(&scope(), "v") {
            Ok(Some(v)) => assert_eq!(v, value),
            Ok(None) | Err(SecretsError::Failed(_)) => {}
            Err(e) => panic!("{e:?}"),
        }
        assert_eq!(b.get(&scope(), "a").unwrap().as_deref(), Some("keep"));
        // A new try finishes the remove.
        b.remove(&scope(), "v").unwrap();
        assert_eq!(b.get(&scope(), "v"), Ok(None));
        assert_eq!(b.names(&scope()).unwrap(), vec!["a".to_owned()]);
    }
}

#[test]
fn a_failing_store_never_puts_a_value_in_an_error() {
    let canary = "TOPSECRET-canary-7f3a";
    let (mut b, fake) = backend();
    b.set(&scope(), "t", canary).unwrap();
    for budget in 0..6 {
        fake.set_budget(Some(budget));
        let r = b.set(&scope(), "t2", canary);
        assert!(!format!("{r:?}").contains(canary));
    }
    fake.set_budget(None);
    fake.0.lock().unwrap().fail_reads = true;
    for r in [format!("{:?}", b.get(&scope(), "t")), format!("{:?}", b.names(&scope())), format!("{:?}", b.set(&scope(), "t", canary))] {
        assert!(r.contains("Err"), "{r}");
        assert!(!r.contains(canary), "{r}");
    }
}

#[test]
fn the_older_header_with_no_crc_is_still_read() {
    let (mut b, fake) = backend();
    // Format 1: one byte (the chunk count) and the data.
    let mut first = vec![2u8];
    first.extend(vec![b'a'; 2048]);
    fake.put_raw(&scope(), &chunk_key("old", 0), &first);
    fake.put_raw(&scope(), &chunk_key("old", 1), b"bcd");
    fake.put_raw(&scope(), &chunk_key(INDEX, 0), &[&[1u8][..], br#"["old"]"#].concat());
    assert_eq!(b.get(&scope(), "old").unwrap().unwrap(), format!("{}bcd", "a".repeat(2048)));
    assert_eq!(b.names(&scope()).unwrap(), vec!["old".to_owned()]);
}

#[test]
fn hostile_item_contents_give_an_error_and_no_panic() {
    let mut next = rng(0x5eed);
    for _ in 0..300 {
        let (mut b, fake) = backend();
        let len = (next() % 40) as usize;
        let junk: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        fake.put_raw(&scope(), &chunk_key("v", 0), &junk);
        let more: Vec<u8> = (0..(next() % 30) as usize).map(|_| next() as u8).collect();
        fake.put_raw(&scope(), &chunk_key("v", 1), &more);
        fake.put_raw(&scope(), &chunk_key(INDEX, 0), &junk);
        let _ = b.get(&scope(), "v");
        let _ = b.names(&scope());
        let _ = b.set(&scope(), "w", "x");
        let _ = b.remove(&scope(), "v");
        let _ = remove_all(&b, &scope());
    }
}

/// Random operations against the backend and against a model.
#[test]
fn random_operations_match_a_model() {
    let iters: usize = std::env::var("PLINTH_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let seed = std::env::var("PLINTH_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x5eed_5ec7_u64);
    let mut next = rng(seed);
    let pool = ["a", "A", "token", "Token", "TOKEN", "api.key", "api.KEY", "x_1", "x-1", "X-1", "k", "K", "long-name", "Long-Name"];
    let bad = ["", "x~1", "a~", "__index__", "a/b", "..", "ünï", "a b", "a\0", "^a"];
    let sizes = [0usize, 1, 2047, 2048, 2049, 4095, 4096, 4097, 6144, 16383, MAX_VALUE_BYTES];
    let chars = ['a', 'Z', '\0', 'é', '日', '😀', '"', '\\', '\n'];
    let (mut b, fake) = backend();
    let mut model: BTreeMap<String, String> = BTreeMap::new();
    for step in 0..iters {
        let name = if next() % 8 == 0 { bad[(next() % bad.len() as u64) as usize] } else { pool[(next() % pool.len() as u64) as usize] };
        let valid = valid_name(name) && name != INDEX;
        match next() % 4 {
            0 => {
                let size = if next() % 2 == 0 { sizes[(next() % sizes.len() as u64) as usize] } else { (next() % 5000) as usize };
                let value = text_of(size, (next() % 4) as usize, chars[(next() % chars.len() as u64) as usize]);
                let r = b.set(&scope(), name, &value);
                assert_eq!(r.is_ok(), valid, "step {step}: set {name:?}");
                if valid {
                    model.insert(name.to_owned(), value);
                }
            }
            1 => {
                let got = b.get(&scope(), name).unwrap_or_else(|e| panic!("step {step}: get {name:?}: {e:?}"));
                assert_eq!(got.as_ref(), if valid { model.get(name) } else { None }, "step {step}: get {name:?}");
            }
            2 => {
                b.remove(&scope(), name).unwrap();
                if valid {
                    model.remove(name);
                }
            }
            _ => {}
        }
        // `names` is sorted and matches the model after every step.
        let names = b.names(&scope()).unwrap();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "step {step}: names are not sorted: {names:?}");
        assert_eq!(names, model.keys().cloned().collect::<Vec<_>>(), "step {step}");
    }
    for (k, v) in &model {
        assert_eq!(b.get(&scope(), k).unwrap().as_ref(), Some(v));
    }
    // Another scope does not see the secrets, and the uninstall leaves nothing.
    let other = SecretsScope::new("test.other");
    assert_eq!(b.names(&other).unwrap(), Vec::<String>::new());
    assert_eq!(b.get(&other, "token"), Ok(None));
    remove_all(&b, &scope()).unwrap();
    assert_eq!(fake.keys(), Vec::<String>::new());
}

#[test]
fn many_apps_with_the_same_names_stay_apart() {
    let (mut b, fake) = backend();
    for app in 0..20 {
        let s = SecretsScope::new(format!("app.{app}"));
        for name in ["token", "Token", "k"] {
            b.set(&s, name, &format!("{app}:{name}:{}", "v".repeat(app * 300))).unwrap();
        }
    }
    for app in 0..20 {
        let s = SecretsScope::new(format!("app.{app}"));
        assert_eq!(b.get(&s, "Token").unwrap().unwrap(), format!("{app}:Token:{}", "v".repeat(app * 300)));
        assert_eq!(b.names(&s).unwrap(), vec!["Token".to_owned(), "k".to_owned(), "token".to_owned()]);
    }
    remove_all(&b, &SecretsScope::new("app.7")).unwrap();
    assert_eq!(b.names(&SecretsScope::new("app.7")).unwrap(), Vec::<String>::new());
    assert_eq!(b.get(&SecretsScope::new("app.8"), "k").unwrap().unwrap(), format!("8:k:{}", "v".repeat(2400)));
    assert!(fake.keys().iter().all(|k| !k.contains("/app.7/")));
}
