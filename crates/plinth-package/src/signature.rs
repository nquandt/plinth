//! Package signatures: `signature.json` (`docs/HUB.md` §6.1, SPEC.md §10.1,
//! phase H1).
//!
//! `signature.json` holds a SHA-256 digest of every other entry in the
//! package (`plinth-profile`, the raw bytes of `manifest.toml`, the entry
//! module, the assets, `source/…` and every other entry; schema
//! `plinth.signature/2`) and an Ed25519 signature over those digests, made with a
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

/// The schema of `signature.json`. `/2` also covers `manifest.toml`,
/// `plinth-profile` and every other entry; `/1` covered only the entry
/// module and the assets, so `verify` refuses it.
pub const SCHEMA: &str = "plinth.signature/2";
/// The old schema, which did not cover the manifest.
const SCHEMA_V1: &str = "plinth.signature/1";

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
/// entry path (`Package::signed_entries`). The manifest digest is over the
/// bytes that `Package::write` puts in the file (`Package::manifest_bytes`).
fn digests_of(pkg: &Package) -> Result<BTreeMap<String, String>> {
    let mut digests = BTreeMap::new();
    for (path, bytes) in pkg.signed_entries()? {
        if digests.insert(path.clone(), hex(&Sha256::digest(&bytes))).is_some() {
            bail!("the package has the entry `{path}` two times");
        }
    }
    Ok(digests)
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
    let statement = Statement { schema: SCHEMA.into(), publisher: identity.name.clone(), key: identity.key_id(), digests: digests_of(pkg)? };
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
    if file.schema == SCHEMA_V1 {
        bail!("this package was signed with an old format ({SCHEMA_V1}); sign it again with plinth sign");
    }
    if file.schema != SCHEMA {
        bail!("signature.json has an unknown schema `{}`", file.schema);
    }
    if file.publisher != pkg.manifest.publisher {
        bail!("the manifest's publisher (`{}`) does not match the signature's (`{}`)", pkg.manifest.publisher, file.publisher);
    }
    let expected = digests_of(pkg)?;
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
        let cfg = ProjectConfig::parse(&format!(
            "id = \"com.example.app\"\nname = \"App\"\nversion = \"1.0.0\"\npublisher = \"{name}\"\n\
             [[capabilities]]\nname = \"net:api.example.com\"\nrationale = \"Sync.\"\n\
             [updates]\nfeed = \"https://example.com/app/feed.json\"\n"
        ))
        .unwrap();
        let manifest = cfg.manifest("1.0", "plinth-rt/test", None, &wasm);
        let pkg = Package::new(manifest, wasm, vec![("assets/a.txt".into(), b"hi".to_vec())]);
        (pkg, identity)
    }

    /// The entries of a zip, in order, as (name, bytes).
    fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        use std::io::Read as _;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let mut data = Vec::new();
                f.read_to_end(&mut data).unwrap();
                (f.name().to_owned(), data)
            })
            .collect()
    }

    fn rezip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        use std::io::Write as _;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            zip.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A signed package file with one text change in its `manifest.toml`
    /// (an attacker who edits the file, not the in-memory value).
    fn with_changed_manifest(file: &[u8], from: &str, to: &str) -> Vec<u8> {
        let mut entries = unzip(file);
        let manifest = entries.iter_mut().find(|(n, _)| n == "manifest.toml").unwrap();
        let text = String::from_utf8(manifest.1.clone()).unwrap();
        assert!(text.contains(from), "{from} not in {text}");
        manifest.1 = text.replacen(from, to, 1).into_bytes();
        rezip(&entries)
    }

    #[test]
    fn a_changed_manifest_in_a_signed_package_is_refused() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        let file = pkg.write().unwrap();
        assert!(verify(&Package::read(&file).unwrap()).unwrap().is_some());
        let changes = [
            ("version = \"1.0.0\"", "version = \"9.0.0\""),
            ("id = \"com.example.app\"", "id = \"com.example.other\""),
            ("name = \"net:api.example.com\"", "name = \"net:*\""),
            ("feed = \"https://example.com/app/feed.json\"", "feed = \"https://evil.example/feed.json\""),
            ("core = \"follow\"", "core = \"pinned\""),
            ("version = \"1.0.0\"", "version = \"1.0.0\"\n"),
        ];
        for (from, to) in changes {
            let changed = Package::read(&with_changed_manifest(&file, from, to)).unwrap();
            let err = verify(&changed).unwrap_err().to_string();
            assert!(err.contains("does not match its signature"), "{from} -> {to}: {err}");
        }
        // An added capability table.
        let mut entries = unzip(&file);
        let m = entries.iter_mut().find(|(n, _)| n == "manifest.toml").unwrap();
        m.1.extend_from_slice(b"\n[[capabilities]]\nname = \"files\"\nrationale = \"x\"\n");
        assert!(verify(&Package::read(&rezip(&entries)).unwrap()).is_err());
        // A changed in-memory manifest is also refused.
        let mut back = Package::read(&file).unwrap();
        back.manifest.version = "9.0.0".into();
        assert!(verify(&back).is_err());
    }

    #[test]
    fn a_changed_profile_is_refused() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        let mut entries = unzip(&pkg.write().unwrap());
        entries.iter_mut().find(|(n, _)| n == "plinth-profile").unwrap().1 = b"plinth/1\n".to_vec();
        assert!(verify(&Package::read(&rezip(&entries)).unwrap()).is_err());
    }

    #[test]
    fn write_puts_the_signed_manifest_bytes_in_the_file() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        let file: SignatureFile = serde_json::from_slice(pkg.signature.as_ref().unwrap()).unwrap();
        let entries = unzip(&pkg.write().unwrap());
        for (name, data) in &entries {
            if name != "signature.json" {
                assert_eq!(file.digests.get(name), Some(&hex(&Sha256::digest(data))), "{name}");
            }
        }
        assert_eq!(file.digests.len(), entries.len() - 1);
        assert!(file.digests.contains_key("manifest.toml") && file.digests.contains_key("plinth-profile"));
    }

    #[test]
    fn signing_a_read_package_keeps_the_raw_manifest_bytes() {
        // A manifest that `manifest_toml` would write in another way (a
        // comment, another key order): `plinth sign` keeps its bytes.
        let (pkg, identity) = signed_package("Acme");
        let mut entries = unzip(&pkg.write().unwrap());
        let m = entries.iter_mut().find(|(n, _)| n == "manifest.toml").unwrap();
        m.1 = [b"# made by another tool\n".as_slice(), &m.1].concat();
        let raw = m.1.clone();
        let mut back = Package::read(&rezip(&entries)).unwrap();
        back.signature = Some(sign(&back, &identity).unwrap());
        let again = Package::read(&back.write().unwrap()).unwrap();
        assert_eq!(&*again.manifest_bytes().unwrap(), &raw[..]);
        assert_eq!(verify(&again).unwrap().unwrap().publisher, "Acme");
    }

    #[test]
    fn source_entries_are_signed() {
        let (mut pkg, identity) = signed_package("Acme");
        pkg.add_other_entry("source/app/main.tsx".into(), b"export {}".to_vec()).unwrap();
        pkg.signature = Some(sign(&pkg, &identity).unwrap());
        let file = pkg.write().unwrap();
        let back = Package::read(&file).unwrap();
        assert_eq!(back.other_entries().len(), 1);
        assert!(verify(&back).unwrap().is_some());
        let mut entries = unzip(&file);
        entries.iter_mut().find(|(n, _)| n == "source/app/main.tsx").unwrap().1 = b"evil".to_vec();
        assert!(verify(&Package::read(&rezip(&entries)).unwrap()).is_err());
        entries.retain(|(n, _)| n != "source/app/main.tsx");
        assert!(verify(&Package::read(&rezip(&entries)).unwrap()).is_err(), "a removed entry");
    }

    #[test]
    fn unknown_entries_are_signed() {
        // A package from a newer tool, with an entry that this version does
        // not know: it reads, `plinth sign` covers the entry, and it verifies.
        let (pkg, identity) = signed_package("Acme");
        let mut entries = unzip(&pkg.write().unwrap());
        entries.push(("future/data.bin".into(), b"new".to_vec()));
        let mut back = Package::read(&rezip(&entries)).unwrap();
        back.signature = Some(sign(&back, &identity).unwrap());
        let file = back.write().unwrap();
        let again = Package::read(&file).unwrap();
        assert_eq!(again.other_entries(), [("future/data.bin".to_owned(), b"new".to_vec())]);
        assert!(again.assets.iter().all(|(n, _)| n.starts_with("assets/")));
        assert!(verify(&again).unwrap().is_some());
        // An unknown entry added after signing is refused.
        let mut entries = unzip(&file);
        entries.insert(0, ("extra.txt".into(), b"x".to_vec()));
        let err = verify(&Package::read(&rezip(&entries)).unwrap()).unwrap_err().to_string();
        assert!(err.contains("does not match its signature"), "{err}");
    }

    #[test]
    fn an_old_signature_schema_is_refused_with_a_clear_message() {
        let (mut pkg, identity) = signed_package("Acme");
        let mut file: SignatureFile = serde_json::from_slice(&sign(&pkg, &identity).unwrap()).unwrap();
        file.schema = "plinth.signature/1".into();
        pkg.signature = Some(serde_json::to_vec(&file).unwrap());
        let err = verify(&pkg).unwrap_err().to_string();
        assert!(err.contains("old format") && err.contains("plinth sign"), "{err}");
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
