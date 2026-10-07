//! The update feed (`docs/UPDATES.md` §8 step 2): a JSON file an author
//! hosts at a URL, listing the app's versions per channel, plus a
//! detached signature file next to it.
//!
//! **Format choice.** The feed reuses the registry's app-document shape
//! (`docs/REGISTRY.md` §4: `version`, `sha256`, `core`) instead of a new
//! shape, so a static registry's `apps/<id>/index.json` and a feed read
//! almost the same way. The signature is a **detached file**
//! (`feed.json` + `feed.json.sig`), not an envelope that wraps the feed
//! bytes in a signed JSON field: a detached signature signs the exact
//! bytes the author already wrote to disk and a client already
//! downloaded, with no canonicalization step to get wrong on either side
//! (`signature.json`, by contrast, needs canonical JSON because the
//! signed statement and the stored file are two different values; here
//! they are the same bytes). This is the simplest option that is still
//! secure: Ed25519 signs an arbitrary byte string directly, so signing
//! the raw file is not a shortcut, it is the straightforward use of the
//! primitive.

use crate::publisher::PublisherIdentity;
use anyhow::{Context as _, Result, bail};
use ed25519_dalek::{Signature, Signer as _, Verifier as _};
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "plinth.feed/1";

/// `feed.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Feed {
    pub schema: String,
    /// The app id this feed is for (`plinth.toml`'s `id`). A host refuses
    /// a feed for a different app (a mismatched feed URL, or a stale
    /// copy).
    pub id: String,
    /// Channel name (`"stable"`, `"beta"`, ...) -> versions, newest
    /// first. `plinth feed add` keeps this order.
    pub channels: std::collections::BTreeMap<String, Vec<FeedVersion>>,
}

/// One version entry in a feed channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct FeedVersion {
    pub version: String,
    /// The package's URL, relative to the feed's own URL, or absolute.
    pub package: String,
    /// SHA-256 of the package file, hex.
    pub sha256: String,
    /// The core version this app version needs (SPEC.md §10.5), for
    /// example `"1.3"`.
    pub core: String,
    /// Where to get that core, when the host does not already have it.
    /// `None` when the feed assumes every host already has a compatible
    /// core (for example, during development).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_source: Option<CoreSource>,
}

/// Where a feed says a core can be fetched, and its digest, so a host can
/// download and verify it the same way it verifies a package
/// (`docs/UPDATES.md` §4 item 4, UP-Q4: there is no project core key yet,
/// so a host accepts a core only when its digest is the one this *signed*
/// feed lists — the feed's own signature stands in for the missing
/// project key, for now).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct CoreSource {
    /// The core's URL, relative to the feed's own URL, or absolute.
    pub url: String,
    pub version: String,
    /// SHA-256 of the core file, hex.
    pub sha256: String,
}

impl Feed {
    pub fn parse(bytes: &[u8]) -> Result<Feed> {
        let feed: Feed = serde_json::from_slice(bytes).context("feed.json is not valid")?;
        if feed.schema != SCHEMA {
            bail!("feed.json has an unknown schema `{}`; this plinth is too old to read it", feed.schema);
        }
        Ok(feed)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// The versions of one channel, newest first, or an empty slice for a
    /// channel the feed does not list.
    pub fn channel(&self, name: &str) -> &[FeedVersion] {
        self.channels.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The newest version of a channel (the first entry; `plinth feed
    /// add` keeps the list newest-first and `add_version` preserves that).
    pub fn newest(&self, channel: &str) -> Option<&FeedVersion> {
        self.channel(channel).first()
    }

    /// Adds, or replaces, a version in a channel, keeping the list sorted
    /// newest-first by semver. Used by `plinth feed add`.
    pub fn add_version(&mut self, channel: &str, entry: FeedVersion) {
        let list = self.channels.entry(channel.to_owned()).or_default();
        list.retain(|v| v.version != entry.version);
        list.push(entry);
        list.sort_by(|a, b| crate::semver_key(&b.version).cmp(&crate::semver_key(&a.version)));
    }
}

/// `feed.json.sig`: a detached Ed25519 signature over the exact bytes of
/// `feed.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FeedSignature {
    pub schema: String,
    pub publisher: String,
    pub key: String,
    /// Base64 (standard) of the Ed25519 signature over the raw
    /// `feed.json` bytes.
    pub signature: String,
}

/// Signs `feed_bytes` (the exact bytes written to `feed.json`) with
/// `identity`, returning the `feed.json.sig` contents.
pub fn sign(feed_bytes: &[u8], identity: &PublisherIdentity) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let signature: Signature = identity.signing_key.sign(feed_bytes);
    let file = FeedSignature {
        schema: SCHEMA.into(),
        publisher: identity.name.clone(),
        key: identity.key_id(),
        signature: base64::engine::general_purpose::STANDARD.encode(signature.to_bytes()),
    };
    Ok(serde_json::to_vec_pretty(&file)?)
}

/// Who signed a feed, once `verify` has checked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedSigner {
    pub publisher: String,
    pub key: String,
}

/// Verifies `feed_bytes` against `sig_bytes` (the parsed contents of
/// `feed.json.sig`). The caller then checks `FeedSigner::key` against the
/// key continuity rule (`docs/UPDATES.md` §4 item 4: an update must carry
/// the same publisher key as the embedded package).
pub fn verify(feed_bytes: &[u8], sig_bytes: &[u8]) -> Result<FeedSigner> {
    let file: FeedSignature = serde_json::from_slice(sig_bytes).context("feed.json.sig is not valid")?;
    if file.schema != SCHEMA {
        bail!("feed.json.sig has an unknown schema `{}`", file.schema);
    }
    let key = crate::publisher::parse_key_id(&file.key)?;
    use base64::Engine as _;
    let sig_bytes = base64::engine::general_purpose::STANDARD.decode(&file.signature).context("feed.json.sig's signature is not valid base64")?;
    let sig_bytes: [u8; 64] = sig_bytes.try_into().map_err(|_| anyhow::anyhow!("feed.json.sig's signature is not 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_bytes);
    key.verify(feed_bytes, &signature).context("feed.json's signature does not verify")?;
    Ok(FeedSigner { publisher: file.publisher, key: file.key })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_identity() -> PublisherIdentity {
        PublisherIdentity { name: "Acme".into(), signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng) }
    }

    fn sample_feed() -> Feed {
        let mut feed = Feed { schema: SCHEMA.into(), id: "com.example.notes".into(), channels: Default::default() };
        feed.add_version(
            "stable",
            FeedVersion { version: "1.0.0".into(), package: "notes-1.0.0.plnt".into(), sha256: "aa".repeat(32), core: "1.2".into(), core_source: None },
        );
        feed
    }

    #[test]
    fn parse_round_trips() {
        let feed = sample_feed();
        let bytes = feed.to_bytes().unwrap();
        let back = Feed::parse(&bytes).unwrap();
        assert_eq!(feed, back);
    }

    #[test]
    fn unknown_schema_is_refused() {
        let bad = br#"{"schema":"plinth.feed/99","id":"x","channels":{}}"#;
        assert!(Feed::parse(bad).is_err());
    }

    #[test]
    fn newest_is_the_highest_version() {
        let mut feed = sample_feed();
        feed.add_version(
            "stable",
            FeedVersion { version: "1.2.0".into(), package: "notes-1.2.0.plnt".into(), sha256: "bb".repeat(32), core: "1.2".into(), core_source: None },
        );
        feed.add_version(
            "stable",
            FeedVersion { version: "1.1.0".into(), package: "notes-1.1.0.plnt".into(), sha256: "cc".repeat(32), core: "1.2".into(), core_source: None },
        );
        assert_eq!(feed.newest("stable").unwrap().version, "1.2.0");
        assert_eq!(feed.channel("beta").len(), 0);
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let identity = new_identity();
        let feed = sample_feed();
        let bytes = feed.to_bytes().unwrap();
        let sig = sign(&bytes, &identity).unwrap();
        let signer = verify(&bytes, &sig).unwrap();
        assert_eq!(signer.publisher, "Acme");
        assert_eq!(signer.key, identity.key_id());
    }

    #[test]
    fn wrong_key_is_refused() {
        let identity = new_identity();
        let other = new_identity();
        let feed = sample_feed();
        let bytes = feed.to_bytes().unwrap();
        let sig = sign(&bytes, &other).unwrap();
        // Tamper with the signature file's declared key so it claims to
        // be `identity`'s, but the bytes were signed by `other`: the
        // signature must fail to verify against the claimed key.
        let mut file: FeedSignature = serde_json::from_slice(&sig).unwrap();
        file.key = identity.key_id();
        let sig = serde_json::to_vec(&file).unwrap();
        assert!(verify(&bytes, &sig).is_err());
    }

    #[test]
    fn changed_bytes_are_refused() {
        let identity = new_identity();
        let feed = sample_feed();
        let bytes = feed.to_bytes().unwrap();
        let sig = sign(&bytes, &identity).unwrap();
        let mut tampered = bytes.clone();
        tampered[0] ^= 1;
        assert!(verify(&tampered, &sig).is_err());
    }
}
