//! The desktop `SecretsBackend` for `plinth:secrets` (core 1.13,
//! `docs/SECRETS.md` §4).
//!
//! The backend keeps each secret in the system keychain, through the
//! `keyring` crate:
//!
//! - Windows: the Credential Manager. The target name is
//!   `plinth/<app id>/<instance>/<name>`.
//! - macOS: the Keychain (a generic password item). Service
//!   `plinth.<app id>.<instance>`, account `<name>`.
//! - Linux: the Secret Service. If no service runs, each call answers
//!   `Unsupported`. The backend never falls back to a plain file.
//!
//! A keychain cannot list items by prefix on every platform. So the backend
//! also keeps one index item for each scope (`__index__`, a JSON array of the
//! names). A new name goes in the index first and the item second. If the
//! item write fails, the backend takes the name out of the index again. A
//! crash between the two steps leaves a listed name with no item (`get`
//! answers `null`). It never leaves an item that the index does not list,
//! because the host could not delete such an item when it removes the app
//! (`remove_app_secrets` reads the index). A removal deletes the item first
//! and the index second.
//!
//! The Windows Credential Manager limits a value to 2560 bytes, but a secret
//! can have 16 KiB. So the backend cuts each value (and the index) into chunks
//! of 2048 bytes. Chunk 0 is the item named `<name>`. It starts with a header:
//! one byte (the number of chunks, plus 0x80) and four bytes (the CRC-32 of
//! the whole value). Chunk `i` (`i > 0`) is the item `<name>~<i>`. A `~`
//! cannot be in a secret name, and the backend rejects such a name itself, so
//! the chunk names do not collide with a secret. The backend writes the
//! chunks 1 and up first, then chunk 0. A reader that finds no chunk 0 sees
//! no value. A crash in the middle of a replace leaves the old chunk 0 with
//! new chunks: the CRC-32 does not match, and `get` fails (`failed`) in place
//! of returning a wrong value. The next `set` repairs the secret. (A header
//! byte with no 0x80 flag is the older format with no CRC-32. The backend
//! still reads it.)
//!
//! The Windows Credential Manager compares target names without case. So the
//! backend writes each uppercase letter of a name as `^` and the lowercase
//! letter (`Token` is the item `^token`). `Token` and `token` are then two
//! different secrets on every platform. A name never holds `^`.
//!
//! The backend never puts a value in a log line or in an error text.

use plinth_runner_wasmtime::secrets::{valid_name, SecretsBackend, SecretsError, SecretsScope};
use std::collections::HashMap;

/// The name of the index item of a scope. The backend rejects it as a secret
/// name.
const INDEX: &str = "__index__";
/// The bytes of data in one chunk.
const CHUNK: usize = 2048;
/// The most chunks of one blob (16 KiB).
const MAX_CHUNKS: usize = 8;
/// A flag in the header byte: four bytes of CRC-32 follow.
const HAS_CRC: u8 = 0x80;

/// The secrets backend of this platform: the system keychain.
pub fn system_secrets() -> Box<dyn SecretsBackend> {
    Box::new(KeyringSecrets)
}

/// Deletes all secrets of the app `app_id` (instance `default`). The host
/// calls this when it removes the app and its data (`docs/SECRETS.md` §3,
/// rule 3). A host with no keychain has nothing to delete.
pub fn remove_app_secrets(app_id: &str) -> Result<(), SecretsError> {
    let _guard = lock();
    remove_all(&Chunked(KeyringItems), &SecretsScope::new(app_id))
}

/// One lock for every keychain call of this process. A call reads the index,
/// changes it and writes it again, so two calls at the same time would lose
/// one change. The Windows Credential Manager also loses writes and deletes
/// that threads make at the same time. (Another process can still race with
/// this one. The host runs as one process.)
static KEYCHAIN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    // A panic in another call does not make the keychain unusable.
    KEYCHAIN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Deletes all secrets of `scope` in `store`, then the index.
fn remove_all<S: ItemStore>(store: &Chunked<S>, scope: &SecretsScope) -> Result<(), SecretsError> {
    match store.read_index(scope) {
        Ok(names) => {
            for name in names {
                store.delete_blob(scope, &name)?;
            }
            // The index is empty now. Delete the blob chunks too.
            store.delete_blob(scope, INDEX)
        }
        Err(SecretsError::Unsupported) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The backend on the system keychain.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringSecrets;

/// A short text for a keychain error. It names the kind only, so it can never
/// hold a value.
fn kind(e: &keyring::Error) -> &'static str {
    use keyring::Error::*;
    match e {
        PlatformFailure(_) => "platform failure",
        NoStorageAccess(_) => "no access to the store",
        NoEntry => "no entry",
        BadEncoding(_) => "bad encoding",
        BadDataFormat(..) => "bad data format",
        BadStoreFormat(_) => "bad store format",
        TooLong(..) => "too long",
        Invalid(..) => "invalid",
        Ambiguous(_) => "ambiguous",
        NoDefaultStore => "no store",
        NotSupportedByStore(_) => "not supported by the store",
        _ => "other",
    }
}

fn failed(what: &str, e: &keyring::Error) -> SecretsError {
    SecretsError::Failed(format!("{what}: {}", kind(e)))
}

/// The CRC-32 (IEEE) of `data`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// The item key of chunk `i` of the blob `key`. The key is written without
/// uppercase letters (see the header of this file).
fn chunk_key(key: &str, i: usize) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('^');
        }
        out.push(c.to_ascii_lowercase());
    }
    if i > 0 {
        out.push('~');
        out.push_str(&i.to_string());
    }
    out
}

/// A store of items: byte strings under a key, in the scope of one app. The
/// keychain is one; tests use a double that can fail.
trait ItemStore: Send {
    /// One item. `None` when it does not exist.
    fn read(&self, scope: &SecretsScope, key: &str) -> Result<Option<Vec<u8>>, SecretsError>;
    fn write(&self, scope: &SecretsScope, key: &str, bytes: &[u8]) -> Result<(), SecretsError>;
    /// An item that does not exist is not an error.
    fn delete(&self, scope: &SecretsScope, key: &str) -> Result<(), SecretsError>;
}

/// The items in the system keychain.
#[derive(Clone, Copy, Debug, Default)]
struct KeyringItems;

impl KeyringItems {
    /// The keychain entry for the item `key` of `scope`.
    fn entry(scope: &SecretsScope, key: &str) -> Result<keyring::Entry, SecretsError> {
        // The first call opens the platform store. If that fails, there is no
        // safe store on this host.
        if keyring::Entry::store_status().is_err() {
            return Err(SecretsError::Unsupported);
        }
        let service = format!("plinth.{}.{}", scope.app, scope.instance);
        let target = scope.target(key);
        // Only the Windows store knows the `target` modifier.
        let mut modifiers: HashMap<&str, &str> = HashMap::new();
        if cfg!(windows) {
            modifiers.insert("target", &target);
        }
        match keyring_core::Entry::new_with_modifiers(&service, key, &modifiers) {
            Ok(inner) => Ok(keyring::Entry { inner }),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("open entry", &e)),
        }
    }
}

impl ItemStore for KeyringItems {
    fn read(&self, scope: &SecretsScope, key: &str) -> Result<Option<Vec<u8>>, SecretsError> {
        match Self::entry(scope, key)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("read", &e)),
        }
    }

    fn write(&self, scope: &SecretsScope, key: &str, bytes: &[u8]) -> Result<(), SecretsError> {
        match Self::entry(scope, key)?.set_secret(bytes) {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("write", &e)),
        }
    }

    fn delete(&self, scope: &SecretsScope, key: &str) -> Result<(), SecretsError> {
        match Self::entry(scope, key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("delete", &e)),
        }
    }
}

/// Secrets in chunks, with an index, on an `ItemStore`.
struct Chunked<S>(S);

impl<S: ItemStore> Chunked<S> {
    /// Reads a blob (all its chunks). `None` when chunk 0 does not exist.
    fn read_blob(&self, scope: &SecretsScope, key: &str) -> Result<Option<Vec<u8>>, SecretsError> {
        let Some(first) = self.0.read(scope, &chunk_key(key, 0))? else { return Ok(None) };
        let head = first.first().copied().unwrap_or(0);
        let count = (head & !HAS_CRC) as usize;
        let header = if head & HAS_CRC != 0 { 5 } else { 1 };
        if count == 0 || count > MAX_CHUNKS || first.len() < header {
            return Err(SecretsError::Failed("read: bad header".to_owned()));
        }
        let mut data = first[header..].to_vec();
        for i in 1..count {
            match self.0.read(scope, &chunk_key(key, i))? {
                Some(part) => data.extend_from_slice(&part),
                None => return Err(SecretsError::Failed("read: missing chunk".to_owned())),
            }
        }
        if head & HAS_CRC != 0 && first[1..5] != crc32(&data).to_le_bytes() {
            return Err(SecretsError::Failed("read: chunks do not match".to_owned()));
        }
        Ok(Some(data))
    }

    /// Writes a blob. It writes chunk 0 last, then removes the chunks that an
    /// older, longer value used.
    fn write_blob(&self, scope: &SecretsScope, key: &str, data: &[u8]) -> Result<(), SecretsError> {
        let count = data.len().div_ceil(CHUNK).max(1);
        if count > MAX_CHUNKS {
            return Err(SecretsError::Failed("write: value too long".to_owned()));
        }
        let parts: Vec<&[u8]> = if data.is_empty() { vec![&[][..]] } else { data.chunks(CHUNK).collect() };
        for (i, part) in parts.iter().enumerate().skip(1) {
            self.0.write(scope, &chunk_key(key, i), part)?;
        }
        let mut first = Vec::with_capacity(5 + parts[0].len());
        first.push(count as u8 | HAS_CRC);
        first.extend_from_slice(&crc32(data).to_le_bytes());
        first.extend_from_slice(parts[0]);
        self.0.write(scope, &chunk_key(key, 0), &first)?;
        for i in count..MAX_CHUNKS {
            self.0.delete(scope, &chunk_key(key, i))?;
        }
        Ok(())
    }

    /// Deletes a blob: the chunks 1 and up first, chunk 0 last.
    fn delete_blob(&self, scope: &SecretsScope, key: &str) -> Result<(), SecretsError> {
        for i in (0..MAX_CHUNKS).rev() {
            self.0.delete(scope, &chunk_key(key, i))?;
        }
        Ok(())
    }

    fn read_index(&self, scope: &SecretsScope) -> Result<Vec<String>, SecretsError> {
        match self.read_blob(scope, INDEX)? {
            None => Ok(Vec::new()),
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| SecretsError::Failed("read: bad index".to_owned())),
        }
    }

    fn write_index(&self, scope: &SecretsScope, names: &[String]) -> Result<(), SecretsError> {
        if names.is_empty() {
            return self.delete_blob(scope, INDEX);
        }
        let json = serde_json::to_vec(names).map_err(|_| SecretsError::Failed("write: bad index".to_owned()))?;
        self.write_blob(scope, INDEX, &json)
    }
}

impl<S: ItemStore> SecretsBackend for Chunked<S> {
    fn set(&mut self, scope: &SecretsScope, name: &str, value: &str) -> Result<(), SecretsError> {
        // The runner checks the name before it calls a backend. This check is
        // for any other caller: a name with `~` or `^` could reach the chunks
        // or the folded keys of another secret.
        if name == INDEX || !valid_name(name) {
            return Err(SecretsError::Failed("invalid name".to_owned()));
        }
        let mut names = self.read_index(scope)?;
        // A new name goes in the index first, the item second.
        let added = !names.iter().any(|n| n == name);
        if added {
            names.push(name.to_owned());
            names.sort();
            self.write_index(scope, &names)?;
        }
        if let Err(e) = self.write_blob(scope, name, value.as_bytes()) {
            if added {
                // Best effort: take the name out of the index again.
                names.retain(|n| n != name);
                let _ = self.write_index(scope, &names);
            }
            return Err(e);
        }
        Ok(())
    }

    fn get(&mut self, scope: &SecretsScope, name: &str) -> Result<Option<String>, SecretsError> {
        if name == INDEX || !valid_name(name) {
            return Ok(None);
        }
        match self.read_blob(scope, name)? {
            None => Ok(None),
            Some(bytes) => {
                String::from_utf8(bytes).map(Some).map_err(|_| SecretsError::Failed("read: bad encoding".to_owned()))
            }
        }
    }

    fn remove(&mut self, scope: &SecretsScope, name: &str) -> Result<(), SecretsError> {
        if name == INDEX || !valid_name(name) {
            return Ok(());
        }
        // The item first, the index last.
        self.delete_blob(scope, name)?;
        let mut names = self.read_index(scope)?;
        if names.iter().any(|n| n == name) {
            names.retain(|n| n != name);
            self.write_index(scope, &names)?;
        }
        Ok(())
    }

    fn names(&mut self, scope: &SecretsScope) -> Result<Vec<String>, SecretsError> {
        let mut names = self.read_index(scope)?;
        names.sort();
        Ok(names)
    }
}

impl SecretsBackend for KeyringSecrets {
    fn set(&mut self, scope: &SecretsScope, name: &str, value: &str) -> Result<(), SecretsError> {
        let _guard = lock();
        Chunked(KeyringItems).set(scope, name, value)
    }
    fn get(&mut self, scope: &SecretsScope, name: &str) -> Result<Option<String>, SecretsError> {
        let _guard = lock();
        Chunked(KeyringItems).get(scope, name)
    }
    fn remove(&mut self, scope: &SecretsScope, name: &str) -> Result<(), SecretsError> {
        let _guard = lock();
        Chunked(KeyringItems).remove(scope, name)
    }
    fn names(&mut self, scope: &SecretsScope) -> Result<Vec<String>, SecretsError> {
        let _guard = lock();
        Chunked(KeyringItems).names(scope)
    }
}

#[cfg(test)]
#[path = "secrets_double_tests.rs"]
mod double_tests;

#[cfg(test)]
mod tests {
    //! These tests use the real keychain of the OS, with unique test app ids,
    //! and they remove their items. A test returns early (and says so) when
    //! the host has no usable store, for example Linux CI with no Secret
    //! Service.

    use super::*;
    use plinth_runner_wasmtime::secrets::{MAX_SECRETS, MAX_VALUE_BYTES};
    use std::path::Path;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A test app id. It removes the app's secrets when it drops.
    struct TestApp(String);

    impl TestApp {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            Self(format!("plinth.test.secrets.{}.{n}", std::process::id()))
        }
        fn scope(&self) -> SecretsScope {
            SecretsScope::new(&self.0)
        }
    }

    impl Drop for TestApp {
        fn drop(&mut self) {
            let _ = remove_app_secrets(&self.0);
        }
    }

    /// True when the keychain works. A test that gets false skips.
    fn store_works() -> bool {
        let app = TestApp::new();
        let ok = KeyringSecrets.set(&app.scope(), "probe", "x").is_ok();
        if !ok {
            eprintln!("skip: no usable system keychain on this host");
        }
        ok
    }

    #[test]
    fn round_trip_names_sorted_and_remove() {
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        assert_eq!(b.get(&scope, "token"), Ok(None));
        b.set(&scope, "token", "abc123").unwrap();
        b.set(&scope, "b.name", "two").unwrap();
        b.set(&scope, "a-name", "one").unwrap();
        assert_eq!(b.get(&scope, "token"), Ok(Some("abc123".to_owned())));
        assert_eq!(b.names(&scope), Ok(vec!["a-name".to_owned(), "b.name".to_owned(), "token".to_owned()]));
        // A replace keeps one name.
        b.set(&scope, "token", "new").unwrap();
        assert_eq!(b.get(&scope, "token"), Ok(Some("new".to_owned())));
        assert_eq!(b.names(&scope).unwrap().len(), 3);
        b.remove(&scope, "token").unwrap();
        assert_eq!(b.get(&scope, "token"), Ok(None));
        assert_eq!(b.names(&scope), Ok(vec!["a-name".to_owned(), "b.name".to_owned()]));
        // A name that does not exist is not an error.
        b.remove(&scope, "token").unwrap();
        // An empty value and a non-ASCII value are values.
        b.set(&scope, "empty", "").unwrap();
        assert_eq!(b.get(&scope, "empty"), Ok(Some(String::new())));
        b.set(&scope, "wide", "pässwörd-日本").unwrap();
        assert_eq!(b.get(&scope, "wide"), Ok(Some("pässwörd-日本".to_owned())));
    }

    #[test]
    fn two_apps_do_not_see_each_other() {
        if !store_works() {
            return;
        }
        let (one, two) = (TestApp::new(), TestApp::new());
        let mut b = KeyringSecrets;
        b.set(&one.scope(), "token", "one").unwrap();
        b.set(&two.scope(), "token", "two").unwrap();
        assert_eq!(b.get(&one.scope(), "token"), Ok(Some("one".to_owned())));
        assert_eq!(b.get(&two.scope(), "token"), Ok(Some("two".to_owned())));
        b.remove(&one.scope(), "token").unwrap();
        assert_eq!(b.get(&one.scope(), "token"), Ok(None));
        assert_eq!(b.get(&two.scope(), "token"), Ok(Some("two".to_owned())));
        assert_eq!(b.names(&one.scope()), Ok(vec![]));
        assert_eq!(b.names(&two.scope()), Ok(vec!["token".to_owned()]));
        // Another instance of the same app has its own namespace.
        let other = SecretsScope { app: two.0.clone(), instance: "other".to_owned() };
        assert_eq!(b.get(&other, "token"), Ok(None));
    }

    #[test]
    fn the_limits_of_the_runner_fit_the_store() {
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        // The biggest value is 16 KiB. It needs 8 chunks.
        let big = "é".repeat(MAX_VALUE_BYTES / 2);
        assert_eq!(big.len(), MAX_VALUE_BYTES);
        b.set(&scope, "big", &big).unwrap();
        assert_eq!(b.get(&scope, "big"), Ok(Some(big)));
        // A shorter value in its place leaves no old chunk.
        b.set(&scope, "big", "short").unwrap();
        assert_eq!(b.get(&scope, "big"), Ok(Some("short".to_owned())));
        assert_eq!(KeyringItems.read(&scope, "big~3"), Ok(None));
        // The most secrets, with the longest names: the index needs chunks.
        for i in 0..MAX_SECRETS {
            let name = format!("{i:02}{}", "n".repeat(62));
            b.set(&scope, &name, "v").unwrap();
        }
        assert_eq!(b.names(&scope).unwrap().len(), MAX_SECRETS + 1);
    }

    #[test]
    fn the_index_name_is_reserved() {
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        assert!(matches!(b.set(&scope, INDEX, "x"), Err(SecretsError::Failed(_))));
        assert_eq!(b.get(&scope, INDEX), Ok(None));
        assert_eq!(b.names(&scope), Ok(vec![]));
    }

    #[test]
    fn remove_app_secrets_deletes_all() {
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        b.set(&scope, "a", "1").unwrap();
        b.set(&scope, "b", "x".repeat(5000).as_str()).unwrap();
        remove_app_secrets(&app.0).unwrap();
        assert_eq!(b.names(&scope), Ok(vec![]));
        assert_eq!(b.get(&scope, "a"), Ok(None));
        assert_eq!(KeyringItems.read(&scope, "b"), Ok(None));
        assert_eq!(KeyringItems.read(&scope, INDEX), Ok(None));
    }

    /// Reads every file under `dir` and checks that none holds `needle`.
    fn assert_no_file_holds(dir: &Path, needle: &str) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                assert_no_file_holds(&path, needle);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let utf16: Vec<u8> = needle.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
                let found = bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
                    || bytes.windows(utf16.len()).any(|w| w == utf16.as_slice());
                assert!(!found, "a secret value is in the file {}", path.display());
            }
        }
    }

    #[test]
    fn no_plain_file_holds_the_value() {
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        let value = format!("plain-file-canary-{}", app.0);
        b.set(&scope, "token", &value).unwrap();
        assert_eq!(b.get(&scope, "token"), Ok(Some(value.clone())));
        // The app data folder, where `kv.json` would be.
        let data = plinth_runner_wasmtime::kv::data_dir().join("apps").join(&app.0);
        assert_no_file_holds(&data, &value);
        // The temp dir of this test, with a sample file next to the data, so
        // the scan reads at least one file.
        let tmp = std::env::temp_dir().join(format!("plinth-secrets-{}", app.0));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("kv.json"), r#"{"other":"value"}"#).unwrap();
        assert_no_file_holds(&tmp, &value);
        // The scan finds a value that is in a file (the check can fail).
        std::fs::write(tmp.join("leak.txt"), &value).unwrap();
        let caught = std::panic::catch_unwind(|| assert_no_file_holds(&tmp, &value));
        assert!(caught.is_err(), "the scan did not find a planted value");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn names_that_differ_only_in_case_are_two_secrets() {
        // The Credential Manager compares target names without case.
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        b.set(&scope, "Token", "upper").unwrap();
        b.set(&scope, "token", "lower").unwrap();
        assert_eq!(b.get(&scope, "Token"), Ok(Some("upper".to_owned())));
        assert_eq!(b.get(&scope, "token"), Ok(Some("lower".to_owned())));
        b.remove(&scope, "Token").unwrap();
        assert_eq!(b.get(&scope, "Token"), Ok(None));
        assert_eq!(b.get(&scope, "token"), Ok(Some("lower".to_owned())));
        // A name with `~` cannot reach the chunk of another secret.
        b.set(&scope, "long", &"c".repeat(5000)).unwrap();
        assert!(b.set(&scope, "long~1", "evil").is_err());
        b.remove(&scope, "long~1").unwrap();
        assert_eq!(b.get(&scope, "long").unwrap().map(|v| v.len()), Some(5000));
    }

    #[test]
    fn values_at_the_chunk_boundaries_round_trip_in_the_keychain() {
        if !store_works() {
            return;
        }
        let app = TestApp::new();
        let (scope, mut b) = (app.scope(), KeyringSecrets);
        for (i, size) in [2047usize, 2048, 2049, 4096, 4097].into_iter().enumerate() {
            // A 3-byte character that crosses the boundary of a chunk.
            let mut v = "a".repeat(i % 3);
            while v.len() + 3 <= size {
                v.push('日');
            }
            while v.len() < size {
                v.push('z');
            }
            b.set(&scope, "v", &v).unwrap();
            assert_eq!(b.get(&scope, "v"), Ok(Some(v)), "size {size}");
        }
    }

    #[test]
    fn error_texts_name_no_value() {
        let text = failed("write", &keyring::Error::BadEncoding(b"topsecret".to_vec()));
        assert_eq!(text, SecretsError::Failed("write: bad encoding".to_owned()));
    }
}
