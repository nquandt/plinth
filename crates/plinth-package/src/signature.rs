//! Package signatures: `signature.json` (`docs/HUB.md` §6.1, SPEC.md §10.1,
//! phase H1).
//!
//! `signature.json` holds a SHA-256 digest of every other entry in the
//! package and an Ed25519 signature over those digests, made with a
//! publisher key (`crate::publisher`). `sign` adds it to a `Package`;
//! `verify` checks it. A package with no `signature.json` is unsigned
//! (`verify` returns `Ok(None)`); a package whose signature does not match
//! is refused (`verify` returns `Err`) — callers must treat that the same
//! as a corrupt package, not as "unsigned".

use crate::{Package, publisher::PublisherIdentity};
use anyhow::{Context as _, Result, bail};
use ed25519_dalek::{Signature, Signer as _, Verifier as _};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "plinth.signature/1";

/// The signed statement: `signature.json` minus the `signature` field
/// itself. Signing and verifying both build this, then canonicalize it.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub struct Statement {
    pub schema: String,
    pub publisher: String,
    pub key: String,
    /// Entry path (for example `app.wasm`, `assets/icon.png`) -> SHA-256
    /// hex digest. Every package entry except `signature.json` itself.
    pub digests: BTreeMap<String, String>,
}

/// `signature.json`'s full contents.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct SignatureFile {
    pub schema: String,
    pub publisher: String,
    pub key: String,
    pub digests: BTreeMap<String, String>,
    /// Base64 (standard) of the Ed25519 signature over the canonical JSON
    /// of the statement above.
    pub signature: String,
}

/// Who signed a package, once `verify` has checked the signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub publisher: String,
    pub key: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest of every entry in `pkg` except `signature.json`, keyed by
/// entry path.
fn digests_of(pkg: &Package) -> BTreeMap<String, String> {
    let mut digests = BTreeMap::new();
    digests.insert(pkg.manifest.entry.clone(), hex(&Sha256::digest(&pkg.component)));
    for (path, bytes) in &pkg.assets {
        digests.insert(path.clone(), hex(&Sha256::digest(bytes)));
    }
    digests
}

/// Canonical JSON: an object's keys sorted, no whitespace. `serde_json`
/// already sorts `BTreeMap` keys and `Statement`'s fields serialize in a
/// fixed, already-alphabetical order (`digests`, `key`, `publisher`,
/// `schema` would not be alphabetical as written, so this sorts the whole
/// value through `serde_json::Value`, which orders object keys with its
/// `preserve_order` feature off — the default, used here).
fn canonical_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let v = serde_json::to_value(value)?;
    let sorted = sort_value(v);
    Ok(serde_json::to_vec(&sorted)?)
}

fn sort_value(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut sorted: BTreeMap<String, serde_json::Value> = BTreeMap::new();
            for (k, val) in map {
                sorted.insert(k, sort_value(val));
            }
            let mut out = serde_json::Map::new();
            for (k, val) in sorted {
                out.insert(k, val);
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(items.into_iter().map(sort_value).collect()),
        other => other,
    }
}

/// Signs `pkg` with `identity`, returning the `signature.json` bytes.
/// `pkg.manifest.publisher` must equal `identity.name` (the manifest's
/// publisher must match the signature's; `docs/HUB.md` §6.1).
pub fn sign(pkg: &Package, identity: &PublisherIdentity) -> Result<Vec<u8>> {
    if pkg.manifest.publisher != identity.name {
        bail!(
            "the manifest's publisher (`{}`) does not match the signing identity (`{}`)",
            pkg.manifest.publisher,
            identity.name
        );
    }
    let statement = Statement { schema: SCHEMA.into(), publisher: identity.name.clone(), key: identity.key_id(), digests: digests_of(pkg) };
    let message = canonical_json(&statement)?;
    let signature: Signature = identity.signing_key.sign(&message);
    use base64::Engine as _;
    let file = SignatureFile {
        schema: statement.schema,
        publisher: statement.publisher,
        key: statement.key,
        digests: statement.digests,
        signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
    };
    Ok(serde_json::to_vec_pretty(&file)?)
}

/// Verifies `pkg`'s signature, if it has one. `Ok(None)` means the package
/// is unsigned. `Err` means it claims to be signed but the signature, the
/// digests or the publisher name do not check out — a caller must refuse
/// the package, not treat it as merely unsigned.
pub fn verify(pkg: &Package) -> Result<Option<Signer>> {
    let Some(bytes) = &pkg.signature else { return Ok(None) };
    let file: SignatureFile = serde_json::from_slice(bytes).context("signature.json is not valid")?;
    if file.schema != SCHEMA {
        bail!("signature.json has an unknown schema `{}`", file.schema);
    }
    if file.publisher != pkg.manifest.publisher {
        bail!("the manifest's publisher (`{}`) does not match the signature's (`{}`)", pkg.manifest.publisher, file.publisher);
    }
    let expected = digests_of(pkg);
    if file.digests != expected {
        bail!("the package does not match its signature (a byte changed, or an entry was added or removed)");
    }
    let key = crate::publisher::parse_key_id(&file.key)?;
    let statement = Statement { schema: file.schema.clone(), publisher: file.publisher.clone(), key: file.key.clone(), digests: file.digests.clone() };
    let message = canonical_json(&statement)?;
    use base64::Engine as _;
    let sig_bytes = base64::engine::general_purpose::STANDARD.decode(&file.signature).context("signature.json's signature is not valid base64")?;
    let sig_bytes: [u8; 64] = sig_bytes.try_into().map_err(|_| anyhow::anyhow!("signature.json's signature is not 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_bytes);
    key.verify(&message, &signature).context("the package signature does not verify")?;
    Ok(Some(Signer { publisher: file.publisher, key: file.key }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Package, ProjectConfig};

    fn signed_package(name: &str) -> (Package, crate::publisher::PublisherIdentity) {
        let identity = crate::publisher::PublisherIdentity { name: name.to_owned(), signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng) };
        let wasm = b"\0asm fake component".to_vec();
        let cfg = ProjectConfig::parse(&format!("id = \"com.example.app\"\nname = \"App\"\nversion = \"1.0.0\"\npublisher = \"{name}\"\n")).unwrap();
        let manifest = cfg.manifest("1.0", "plinth-rt/test", None, &wasm);
        let pkg = Package { manifest, component: wasm, assets: vec![("assets/a.txt".into(), b"hi".to_vec())], signature: None };
        (pkg, identity)
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        let signer = verify(&pkg).unwrap().unwrap();
        assert_eq!(signer.publisher, "Acme");
        assert_eq!(signer.key, identity.key_id());
    }

    #[test]
    fn unsigned_package_is_none() {
        let (pkg, _identity) = signed_package("Acme");
        assert!(verify(&pkg).unwrap().is_none());
    }

    #[test]
    fn tampered_component_is_refused() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        pkg.component[0] ^= 1;
        assert!(verify(&pkg).is_err());
    }

    #[test]
    fn wrong_publisher_name_is_refused() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        pkg.manifest.publisher = "Someone Else".into();
        assert!(verify(&pkg).is_err());
    }

    #[test]
    fn round_trip_through_write_read() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        let bytes = pkg.write().unwrap();
        let back = Package::read(&bytes).unwrap();
        let signer = verify(&back).unwrap().unwrap();
        assert_eq!(signer.publisher, "Acme");
    }
}
