//! `plinth:secrets` (core 1.13, `docs/SECRETS.md`): secure storage for
//! passwords and tokens.
//!
//! The runner checks the capability (`secrets`, Low risk) with the `Policy`,
//! checks the limits, then gives the call to a `SecretsBackend`. A host with
//! a safe store (for example the Windows Credential Manager) supplies its own
//! backend with `Guest::set_secrets`. Until then, the guest has an
//! `UnsupportedSecrets`, which answers `denied:unsupported`. The runner never
//! writes a secret to a plain file, and it never logs a value.
//!
//! Every call completes through the same queue as `net.fetch` and
//! `plinth:files`, with a 4-element list: `[ok, n, text, error]`. A denied
//! call has `ok` false and `error` = `"denied:<reason>"`. A denied call never
//! traps.

use crate::policy::{DeniedReason, Policy};
use plinth_protocol::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

pub use plinth_link::capabilities::SECRETS;

/// The longest name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// The longest value, in bytes of UTF-8 (16 KiB).
pub const MAX_VALUE_BYTES: usize = 16 * 1024;
/// The most secrets for one app.
pub const MAX_SECRETS: usize = 64;

/// Whose secrets a call reads: the app id and the instance
/// (`docs/STORAGE.md`, "Instances"). The host builds the key of a secret
/// from the scope and the name, so an app cannot reach the secrets of
/// another app.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SecretsScope {
    pub app: String,
    pub instance: String,
}

impl SecretsScope {
    /// The scope of the app `app` in its `default` instance.
    pub fn new(app: impl Into<String>) -> Self {
        Self { app: app.into(), instance: "default".to_owned() }
    }

    /// The name of one secret in a system store:
    /// `plinth/<app id>/<instance>/<name>`.
    pub fn target(&self, name: &str) -> String {
        format!("plinth/{}/{}/{}", self.app, self.instance, name)
    }
}

impl Default for SecretsScope {
    fn default() -> Self {
        Self::new("")
    }
}

/// Why a backend call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretsError {
    /// This host has no safe store. The call answers `denied:unsupported`.
    Unsupported,
    /// The store failed. The call answers `failed`. The text must not
    /// contain a secret value.
    Failed(String),
}

/// A host's safe store. A desktop or mobile host implements this on the
/// system keychain; tests use `MemorySecrets`. The runner has checked the
/// name and the limits before it calls a backend.
pub trait SecretsBackend: Send {
    /// Stores `value` under `name`, in place of an earlier value.
    fn set(&mut self, scope: &SecretsScope, name: &str, value: &str) -> Result<(), SecretsError>;
    /// The value of `name`, or `None` if the name does not exist.
    fn get(&mut self, scope: &SecretsScope, name: &str) -> Result<Option<String>, SecretsError>;
    /// Removes `name`. A name that does not exist is not an error.
    fn remove(&mut self, scope: &SecretsScope, name: &str) -> Result<(), SecretsError>;
    /// The names of the scope, sorted.
    fn names(&mut self, scope: &SecretsScope) -> Result<Vec<String>, SecretsError>;
}

/// The backend of a host with no safe store. Every call is unsupported. It
/// does not fall back to a plain file.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedSecrets;

impl SecretsBackend for UnsupportedSecrets {
    fn set(&mut self, _: &SecretsScope, _: &str, _: &str) -> Result<(), SecretsError> {
        Err(SecretsError::Unsupported)
    }
    fn get(&mut self, _: &SecretsScope, _: &str) -> Result<Option<String>, SecretsError> {
        Err(SecretsError::Unsupported)
    }
    fn remove(&mut self, _: &SecretsScope, _: &str) -> Result<(), SecretsError> {
        Err(SecretsError::Unsupported)
    }
    fn names(&mut self, _: &SecretsScope) -> Result<Vec<String>, SecretsError> {
        Err(SecretsError::Unsupported)
    }
}

/// A backend that keeps the secrets in memory only. A clone shares the same
/// store, so a test can give clones to two guests and read the store after.
#[derive(Clone, Default)]
pub struct MemorySecrets {
    store: Arc<Mutex<HashMap<SecretsScope, BTreeMap<String, String>>>>,
}

impl MemorySecrets {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretsBackend for MemorySecrets {
    fn set(&mut self, scope: &SecretsScope, name: &str, value: &str) -> Result<(), SecretsError> {
        self.store.lock().unwrap().entry(scope.clone()).or_default().insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn get(&mut self, scope: &SecretsScope, name: &str) -> Result<Option<String>, SecretsError> {
        Ok(self.store.lock().unwrap().get(scope).and_then(|m| m.get(name)).cloned())
    }

    fn remove(&mut self, scope: &SecretsScope, name: &str) -> Result<(), SecretsError> {
        if let Some(m) = self.store.lock().unwrap().get_mut(scope) {
            m.remove(name);
        }
        Ok(())
    }

    fn names(&mut self, scope: &SecretsScope) -> Result<Vec<String>, SecretsError> {
        Ok(self.store.lock().unwrap().get(scope).map(|m| m.keys().cloned().collect()).unwrap_or_default())
    }
}

/// True when `name` is a valid secret name: 1 to 64 characters from
/// `a-z A-Z 0-9 . _ -`.
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=MAX_NAME_CHARS).contains(&n) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The completion of a failed call: `[false, 0, "", error]`.
fn failure(error: &str) -> Value {
    Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(error.to_owned())])
}

/// The completion of a successful call: `[true, n, text, null]`.
fn success(n: i32, text: String) -> Value {
    Value::List(vec![Value::Bool(true), Value::Int(n), Value::Str(text), Value::Null])
}

/// The `"denied:<reason>"` text of a call that the policy denied.
fn denied_text(reason: DeniedReason) -> &'static str {
    match reason {
        DeniedReason::Undeclared => "denied:undeclared",
        DeniedReason::Refused => "denied:refused",
        DeniedReason::Unsupported => "denied:unsupported",
    }
}

/// The failure completion for a backend error.
fn backend_failure(error: SecretsError) -> Value {
    match error {
        SecretsError::Unsupported => failure("denied:unsupported"),
        SecretsError::Failed(_) => failure("failed"),
    }
}

/// Runs a `set` call. Returns the completion result.
pub fn set(policy: &Policy, backend: &mut dyn SecretsBackend, scope: &SecretsScope, name: &str, value: &str) -> Value {
    if let Err(reason) = policy.check(SECRETS) {
        return failure(denied_text(reason));
    }
    if !valid_name(name) || value.len() > MAX_VALUE_BYTES {
        return failure("denied:limit");
    }
    // A new name may not be the 65th. A name that exists is replaced.
    match backend.names(scope) {
        Err(e) => return backend_failure(e),
        Ok(names) => {
            if names.len() >= MAX_SECRETS && !names.iter().any(|n| n == name) {
                return failure("denied:limit");
            }
        }
    }
    match backend.set(scope, name, value) {
        Ok(()) => success(0, String::new()),
        Err(e) => backend_failure(e),
    }
}

/// Runs a `get` call. Returns the completion result.
pub fn get(policy: &Policy, backend: &mut dyn SecretsBackend, scope: &SecretsScope, name: &str) -> Value {
    if let Err(reason) = policy.check(SECRETS) {
        return failure(denied_text(reason));
    }
    if !valid_name(name) {
        return failure("denied:limit");
    }
    match backend.get(scope, name) {
        Ok(Some(value)) => success(1, value),
        Ok(None) => success(0, String::new()),
        Err(e) => backend_failure(e),
    }
}

/// Runs a `remove` call. Returns the completion result.
pub fn remove(policy: &Policy, backend: &mut dyn SecretsBackend, scope: &SecretsScope, name: &str) -> Value {
    if let Err(reason) = policy.check(SECRETS) {
        return failure(denied_text(reason));
    }
    if !valid_name(name) {
        return failure("denied:limit");
    }
    match backend.remove(scope, name) {
        Ok(()) => success(0, String::new()),
        Err(e) => backend_failure(e),
    }
}

/// Runs a `names` call. Returns the completion result.
pub fn names(policy: &Policy, backend: &mut dyn SecretsBackend, scope: &SecretsScope) -> Value {
    if let Err(reason) = policy.check(SECRETS) {
        return failure(denied_text(reason));
    }
    match backend.names(scope) {
        Ok(mut names) => {
            names.sort();
            let json = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_owned());
            success(names.len() as i32, json)
        }
        Err(e) => backend_failure(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(n: i32, text: &str) -> Value {
        success(n, text.to_owned())
    }

    fn setup() -> (Policy, MemorySecrets, SecretsScope) {
        (Policy::new([SECRETS]), MemorySecrets::new(), SecretsScope::new("app.one"))
    }

    #[test]
    fn round_trip() {
        let (policy, mut b, scope) = setup();
        assert_eq!(set(&policy, &mut b, &scope, "api-token", "abc123"), ok(0, ""));
        assert_eq!(get(&policy, &mut b, &scope, "api-token"), ok(1, "abc123"));
        assert_eq!(set(&policy, &mut b, &scope, "a.b_c", "x"), ok(0, ""));
        assert_eq!(names(&policy, &mut b, &scope), ok(2, r#"["a.b_c","api-token"]"#));
        assert_eq!(remove(&policy, &mut b, &scope, "api-token"), ok(0, ""));
        assert_eq!(get(&policy, &mut b, &scope, "api-token"), ok(0, ""));
        assert_eq!(remove(&policy, &mut b, &scope, "api-token"), ok(0, ""));
        assert_eq!(names(&policy, &mut b, &scope), ok(1, r#"["a.b_c"]"#));
    }

    #[test]
    fn an_empty_value_is_a_value() {
        let (policy, mut b, scope) = setup();
        assert_eq!(set(&policy, &mut b, &scope, "e", ""), ok(0, ""));
        assert_eq!(get(&policy, &mut b, &scope, "e"), ok(1, ""));
    }

    #[test]
    fn apps_do_not_see_each_other() {
        let (policy, mut b, one) = setup();
        let two = SecretsScope::new("app.two");
        set(&policy, &mut b, &one, "token", "one");
        set(&policy, &mut b, &two, "token", "two");
        assert_eq!(get(&policy, &mut b, &one, "token"), ok(1, "one"));
        assert_eq!(get(&policy, &mut b, &two, "token"), ok(1, "two"));
        remove(&policy, &mut b, &one, "token");
        assert_eq!(get(&policy, &mut b, &two, "token"), ok(1, "two"));
        assert_eq!(names(&policy, &mut b, &one), ok(0, "[]"));
    }

    #[test]
    fn the_target_has_app_instance_and_name() {
        assert_eq!(SecretsScope::new("a.b").target("t"), "plinth/a.b/default/t");
    }

    #[test]
    fn limits_answer_denied_limit() {
        let (policy, mut b, scope) = setup();
        let limit = failure("denied:limit");
        for bad in ["", "has space", "slash/name", "ünï", &"a".repeat(65)] {
            assert_eq!(set(&policy, &mut b, &scope, bad, "v"), limit, "name {bad:?}");
            assert_eq!(get(&policy, &mut b, &scope, bad), limit);
            assert_eq!(remove(&policy, &mut b, &scope, bad), limit);
        }
        assert_eq!(set(&policy, &mut b, &scope, &"a".repeat(64), "v"), ok(0, ""));
        let big = "x".repeat(MAX_VALUE_BYTES);
        assert_eq!(set(&policy, &mut b, &scope, "big", &big), ok(0, ""));
        assert_eq!(set(&policy, &mut b, &scope, "big2", &format!("{big}x")), limit);
        // The value size is in bytes of UTF-8.
        assert_eq!(set(&policy, &mut b, &scope, "wide", &"é".repeat(MAX_VALUE_BYTES / 2 + 1)), limit);
    }

    #[test]
    fn the_65th_secret_is_denied_but_a_replace_is_not() {
        let (policy, mut b, scope) = setup();
        for i in 0..MAX_SECRETS {
            assert_eq!(set(&policy, &mut b, &scope, &format!("s{i}"), "v"), ok(0, ""));
        }
        assert_eq!(set(&policy, &mut b, &scope, "extra", "v"), failure("denied:limit"));
        assert_eq!(set(&policy, &mut b, &scope, "s0", "new"), ok(0, ""));
        assert_eq!(get(&policy, &mut b, &scope, "s0"), ok(1, "new"));
        // Another app has its own count.
        assert_eq!(set(&policy, &mut b, &SecretsScope::new("other"), "extra", "v"), ok(0, ""));
        remove(&policy, &mut b, &scope, "s1");
        assert_eq!(set(&policy, &mut b, &scope, "extra", "v"), ok(0, ""));
    }

    #[test]
    fn undeclared_calls_are_denied_and_do_not_reach_the_backend() {
        let policy = Policy::new(["store.kv"]);
        let mut b = MemorySecrets::new();
        let scope = SecretsScope::new("a");
        let denied = failure("denied:undeclared");
        assert_eq!(set(&policy, &mut b, &scope, "t", "v"), denied);
        assert_eq!(get(&policy, &mut b, &scope, "t"), denied);
        assert_eq!(remove(&policy, &mut b, &scope, "t"), denied);
        assert_eq!(names(&policy, &mut b, &scope), denied);
        assert_eq!(b.names(&scope), Ok(vec![]));
    }

    #[test]
    fn refused_and_unsupported_policies_are_denied() {
        let mut policy = Policy::new([SECRETS]);
        policy.refuse(SECRETS);
        let mut b = MemorySecrets::new();
        let scope = SecretsScope::new("a");
        assert_eq!(set(&policy, &mut b, &scope, "t", "v"), failure("denied:refused"));
        assert_eq!(names(&policy, &mut b, &scope), failure("denied:refused"));

        let mut policy = Policy::new([SECRETS]);
        policy.mark_unsupported(SECRETS);
        assert_eq!(get(&policy, &mut b, &scope, "t"), failure("denied:unsupported"));
        assert_eq!(b.names(&scope), Ok(vec![]));
    }

    #[test]
    fn the_default_backend_is_unsupported() {
        let policy = Policy::new([SECRETS]);
        let mut b = UnsupportedSecrets;
        let scope = SecretsScope::new("a");
        let denied = failure("denied:unsupported");
        assert_eq!(set(&policy, &mut b, &scope, "t", "v"), denied);
        assert_eq!(get(&policy, &mut b, &scope, "t"), denied);
        assert_eq!(remove(&policy, &mut b, &scope, "t"), denied);
        assert_eq!(names(&policy, &mut b, &scope), denied);
    }

    #[test]
    fn a_failed_backend_does_not_leak_its_text() {
        struct Broken;
        impl SecretsBackend for Broken {
            fn set(&mut self, _: &SecretsScope, _: &str, v: &str) -> Result<(), SecretsError> {
                Err(SecretsError::Failed(format!("write of {v} failed")))
            }
            fn get(&mut self, _: &SecretsScope, _: &str) -> Result<Option<String>, SecretsError> {
                Ok(None)
            }
            fn remove(&mut self, _: &SecretsScope, _: &str) -> Result<(), SecretsError> {
                Ok(())
            }
            fn names(&mut self, _: &SecretsScope) -> Result<Vec<String>, SecretsError> {
                Ok(vec![])
            }
        }
        let policy = Policy::new([SECRETS]);
        let result = set(&policy, &mut Broken, &SecretsScope::new("a"), "t", "topsecret");
        assert_eq!(result, failure("failed"));
        assert!(!format!("{result:?}").contains("topsecret"));
    }
}
