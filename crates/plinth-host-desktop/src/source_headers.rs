//! Request headers of registry sources, with their secrets in the system
//! keychain (`docs/HUB.md` §5.4, `docs/REGISTRY-HOSTING.md` §6).
//!
//! A secret is in the scope of the app id `plinth.host` (instance `default`),
//! under the name `source.<source>.<header in lowercase>`. The sources file
//! holds the name only. A host without a keychain cannot save a token: the
//! functions here return an error, and they never write a secret to a file.

use anyhow::{Result, bail};
use plinth_hub::{HOST_SECRETS_APP, HeaderSpec, Hub, header_secret_name};
use plinth_runner_wasmtime::secrets::{SecretsBackend, SecretsError, SecretsScope};
use std::sync::Arc;

/// The registry client of the desktop host: it sends the request headers of
/// each source, and reads their secrets from the system keychain.
pub fn hub_sources(hub: &Hub) -> plinth_registry::HubSources {
    plinth_registry::HubSources::with_secrets(hub.clone(), Arc::new(crate::secrets::system_secrets))
}

/// Saves the Cloudflare Access service token for `source`: the two headers
/// `CF-Access-Client-Id` and `CF-Access-Client-Secret`. Both values go to the
/// keychain; the sources file gets the secret names.
pub fn set_access_token(hub: &Hub, source: &str, client_id: &str, client_secret: &str) -> Result<()> {
    set_secret_headers(hub, source, &mut *crate::secrets::system_secrets(), &[("CF-Access-Client-Id", client_id), ("CF-Access-Client-Secret", client_secret)])
}

/// Saves `headers` (name, secret value) for `source` in `store`. A header
/// that the source had before is replaced.
pub fn set_secret_headers(hub: &Hub, source: &str, store: &mut dyn SecretsBackend, headers: &[(&str, &str)]) -> Result<()> {
    if !hub.sources()?.contains_key(source) {
        bail!("no source named `{source}` (add it first with `plinth hub source add`)");
    }
    let scope = SecretsScope::new(HOST_SECRETS_APP);
    let mut specs = Vec::new();
    for (name, value) in headers {
        let secret = header_secret_name(source, name)?;
        match store.set(&scope, &secret, value) {
            Ok(()) => {}
            Err(SecretsError::Unsupported) => {
                bail!("this computer has no secret store (system keychain), so the token was not saved; a token is never written to a file")
            }
            Err(SecretsError::Failed(why)) => bail!("the secret store could not save the header {name} ({why})"),
        }
        specs.push(HeaderSpec { name: (*name).to_owned(), value: None, secret: Some(secret) });
    }
    hub.source_set_headers(source, specs)
}

/// Removes the headers of `source` and their secrets from `store`. A source
/// with no headers is not an error. If the store is not available, the
/// sources file still loses the headers and the error says that the secrets
/// may remain.
pub fn clear_headers(hub: &Hub, source: &str, store: &mut dyn SecretsBackend) -> Result<()> {
    let scope = SecretsScope::new(HOST_SECRETS_APP);
    let specs = hub.source_headers(source)?;
    hub.source_set_headers(source, Vec::new())?;
    for secret in specs.iter().filter_map(|h| h.secret.as_deref()) {
        match store.remove(&scope, secret) {
            Ok(()) => {}
            Err(SecretsError::Unsupported) => bail!("this computer has no secret store; the secrets of source `{source}` may remain"),
            Err(SecretsError::Failed(why)) => bail!("the secret store could not remove a secret of source `{source}` ({why})"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plinth_runner_wasmtime::secrets::{MemorySecrets, UnsupportedSecrets};

    fn hub() -> Hub {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Hub::open(std::env::temp_dir().join(format!("plinth-source-headers-{}-{n}", std::process::id()))).unwrap()
    }

    #[test]
    fn token_goes_to_the_store_and_the_file_has_names_only() {
        let hub = hub();
        hub.source_add("corp", "https://registry.example.com").unwrap();
        let mut store = MemorySecrets::new();
        set_secret_headers(&hub, "corp", &mut store, &[("CF-Access-Client-Id", "id-canary-1"), ("CF-Access-Client-Secret", "secret-canary-2")]).unwrap();
        let specs = hub.source_headers("corp").unwrap();
        assert_eq!(specs[1].secret.as_deref(), Some("source.corp.cf-access-client-secret"));
        assert!(specs.iter().all(|h| h.value.is_none()));
        let file = std::fs::read_to_string(hub.sources_path()).unwrap();
        assert!(!file.contains("canary"), "a secret is in the sources file: {file}");
        let headers = hub.request_headers("https://registry.example.com", &mut store).unwrap();
        assert_eq!(headers, vec![("CF-Access-Client-Id".to_owned(), "id-canary-1".to_owned()), ("CF-Access-Client-Secret".to_owned(), "secret-canary-2".to_owned())]);
        // Another base gets nothing.
        assert!(hub.request_headers("https://other.example.com", &mut store).unwrap().is_empty());
        // Clear: the headers and the secrets are gone.
        clear_headers(&hub, "corp", &mut store).unwrap();
        assert!(hub.source_headers("corp").unwrap().is_empty());
        assert_eq!(store.clone().names(&SecretsScope::new(HOST_SECRETS_APP)), Ok(vec![]));
    }

    #[test]
    fn no_store_means_a_clear_error_and_no_file() {
        let hub = hub();
        hub.source_add("corp", "https://registry.example.com").unwrap();
        let e = set_secret_headers(&hub, "corp", &mut UnsupportedSecrets, &[("CF-Access-Client-Id", "id-canary")]).unwrap_err();
        let text = format!("{e:#}");
        assert!(text.contains("no secret store"), "{text}");
        assert!(!text.contains("canary"));
        assert!(hub.source_headers("corp").unwrap().is_empty());
        let file = std::fs::read_to_string(hub.sources_path()).unwrap();
        assert!(!file.contains("canary"));
    }

    #[test]
    fn a_missing_secret_is_an_error_without_a_value() {
        let hub = hub();
        hub.source_add("corp", "https://registry.example.com").unwrap();
        hub.source_set_headers("corp", vec![HeaderSpec { name: "X-Token".into(), value: None, secret: Some("source.corp.x-token".into()) }]).unwrap();
        let e = hub.request_headers("https://registry.example.com", &mut MemorySecrets::new()).unwrap_err();
        assert!(format!("{e:#}").contains("set-token corp"));
    }

    #[test]
    fn the_secret_name_follows_the_name_rule() {
        assert_eq!(header_secret_name("main", "CF-Access-Client-Id").unwrap(), "source.main.cf-access-client-id");
        assert!(header_secret_name("my source", "X").is_err());
        assert!(header_secret_name(&"a".repeat(60), "CF-Access-Client-Secret").is_err());
    }

    /// The real system keychain, with a unique source name. The test removes
    /// its items. It returns early when the host has no usable store.
    #[test]
    fn real_keychain_round_trip() {
        let name = format!("test-{}", std::process::id());
        let probe = SecretsScope::new(HOST_SECRETS_APP);
        let mut store = crate::secrets::system_secrets();
        let secret = header_secret_name(&name, "X-Probe").unwrap();
        if store.set(&probe, &secret, "x").is_err() {
            eprintln!("skip: no usable system keychain on this host");
            return;
        }
        let _ = store.remove(&probe, &secret);
        let hub = hub();
        hub.source_add(&name, "https://registry.example.com").unwrap();
        let result = (|| -> Result<()> {
            set_access_token(&hub, &name, "kc-id-canary", "kc-secret-canary")?;
            let headers = hub.request_headers("https://registry.example.com", &mut *crate::secrets::system_secrets())?;
            assert_eq!(headers[1], ("CF-Access-Client-Secret".to_owned(), "kc-secret-canary".to_owned()));
            let file = std::fs::read_to_string(hub.sources_path())?;
            assert!(!file.contains("canary"));
            Ok(())
        })();
        let cleared = clear_headers(&hub, &name, &mut *crate::secrets::system_secrets());
        result.unwrap();
        cleared.unwrap();
        let mut store = crate::secrets::system_secrets();
        assert_eq!(store.get(&probe, &header_secret_name(&name, "CF-Access-Client-Id").unwrap()), Ok(None));
        assert_eq!(store.get(&probe, &header_secret_name(&name, "CF-Access-Client-Secret").unwrap()), Ok(None));
    }

    #[test]
    fn an_unknown_source_is_an_error() {
        let hub = hub();
        assert!(set_secret_headers(&hub, "none", &mut MemorySecrets::new(), &[("X", "y")]).is_err());
    }
}
