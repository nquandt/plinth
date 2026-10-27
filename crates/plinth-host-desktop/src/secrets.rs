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
//! names). It writes the item first and the index second. It removes the item
//! first and the index second. A crash between the two steps leaves an item
//! that `names` does not list. It never leaves a listed name with no item.
//!
//! The Windows Credential Manager limits a value to 2560 bytes, but a secret
//! can have 16 KiB. So the backend cuts each value (and the index) into chunks
//! of 2048 bytes. Chunk 0 is the item named `<name>`. It starts with one byte,
//! the number of chunks. Chunk `i` (`i > 0`) is the item `<name>~<i>`. A `~`
//! cannot be in a secret name, so the chunk names do not collide with it. The
//! backend writes the chunks 1 and up first, then chunk 0. A reader that finds
//! no chunk 0 sees no value.
//!
//! The backend never puts a value in a log line or in an error text.

use plinth_runner_wasmtime::secrets::{SecretsBackend, SecretsError, SecretsScope};
use std::collections::HashMap;

/// The name of the index item of a scope. The backend rejects it as a secret
/// name.
const INDEX: &str = "__index__";
/// The bytes of data in one chunk.
const CHUNK: usize = 2048;
/// The most chunks of one blob (16 KiB).
const MAX_CHUNKS: usize = 8;

/// The secrets backend of this platform: the system keychain.
pub fn system_secrets() -> Box<dyn SecretsBackend> {
    Box::new(KeyringSecrets)
}

/// Deletes all secrets of the app `app_id` (instance `default`). The host
/// calls this when it removes the app and its data (`docs/SECRETS.md` §3,
/// rule 3). A host with no keychain has nothing to delete.
pub fn remove_app_secrets(app_id: &str) -> Result<(), SecretsError> {
    let scope = SecretsScope::new(app_id);
    let mut backend = KeyringSecrets;
    match backend.names(&scope) {
        Ok(names) => {
            for name in names {
                backend.remove(&scope, &name)?;
            }
            // The index is empty now. Delete the blob chunks too.
            backend.delete_blob(&scope, INDEX)
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

/// The item key of chunk `i` of the blob `key`.
fn chunk_key(key: &str, i: usize) -> String {
    if i == 0 { key.to_owned() } else { format!("{key}~{i}") }
}

impl KeyringSecrets {
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

    /// Reads one item. `None` when it does not exist.
    fn read_item(scope: &SecretsScope, key: &str) -> Result<Option<Vec<u8>>, SecretsError> {
        match Self::entry(scope, key)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("read", &e)),
        }
    }

    fn write_item(scope: &SecretsScope, key: &str, bytes: &[u8]) -> Result<(), SecretsError> {
        match Self::entry(scope, key)?.set_secret(bytes) {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("write", &e)),
        }
    }

    /// Deletes one item. An item that does not exist is not an error.
    fn delete_item(scope: &SecretsScope, key: &str) -> Result<(), SecretsError> {
        match Self::entry(scope, key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(keyring::Error::NoDefaultStore) => Err(SecretsError::Unsupported),
            Err(e) => Err(failed("delete", &e)),
        }
    }

    /// Reads a blob (all its chunks). `None` when chunk 0 does not exist.
    fn read_blob(&self, scope: &SecretsScope, key: &str) -> Result<Option<Vec<u8>>, SecretsError> {
        let Some(first) = Self::read_item(scope, key)? else { return Ok(None) };
        let count = first.first().copied().unwrap_or(0) as usize;
        if count == 0 || count > MAX_CHUNKS {
            return Err(SecretsError::Failed("read: bad header".to_owned()));
        }
        let mut data = first[1..].to_vec();
        for i in 1..count {
            match Self::read_item(scope, &chunk_key(key, i))? {
                Some(part) => data.extend_from_slice(&part),
                None => return Err(SecretsError::Failed("read: missing chunk".to_owned())),
            }
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
            Self::write_item(scope, &chunk_key(key, i), part)?;
        }
        let mut first = Vec::with_capacity(1 + parts[0].len());
        first.push(count as u8);
        first.extend_from_slice(parts[0]);
        Self::write_item(scope, key, &first)?;
        for i in count..MAX_CHUNKS {
            Self::delete_item(scope, &chunk_key(key, i))?;
        }
        Ok(())
    }

    /// Deletes a blob: the chunks 1 and up first, chunk 0 last.
    fn delete_blob(&self, scope: &SecretsScope, key: &str) -> Result<(), SecretsError> {
        for i in (0..MAX_CHUNKS).rev() {
            Self::delete_item(scope, &chunk_key(key, i))?;
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

impl SecretsBackend for KeyringSecrets {
    fn set(&mut self, scope: &SecretsScope, name: &str, value: &str) -> Result<(), SecretsError> {
        if name == INDEX {
            return Err(SecretsError::Failed("reserved name".to_owned()));
        }
        // The item first, the index second.
        self.write_blob(scope, name, value.as_bytes())?;
        let mut names = self.read_index(scope)?;
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
            names.sort();
            self.write_index(scope, &names)?;
        }
        Ok(())
    }

    fn get(&mut self, scope: &SecretsScope, name: &str) -> Result<Option<String>, SecretsError> {
        if name == INDEX {
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
        if name == INDEX {
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
        assert_eq!(KeyringSecrets::read_item(&scope, "big~3"), Ok(None));
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
        assert_eq!(KeyringSecrets::read_item(&scope, "b"), Ok(None));
        assert_eq!(KeyringSecrets::read_item(&scope, INDEX), Ok(None));
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
    fn error_texts_name_no_value() {
        let text = failed("write", &keyring::Error::BadEncoding(b"topsecret".to_vec()));
        assert_eq!(text, SecretsError::Failed("write: bad encoding".to_owned()));
    }
}
