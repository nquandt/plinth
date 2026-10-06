//! Publisher key pairs (`docs/HUB.md` §6.1, phase H1).
//!
//! A publisher identity is one Ed25519 key pair and a name, stored outside
//! any project: `<publisher dir>/key.ed25519` (the 32-byte seed) and
//! `<publisher dir>/publisher.toml` (`name = "..."`). `plinth publisher
//! init` writes it; `plinth build --sign`/`plinth sign` read it.

use anyhow::{Context as _, Result, bail};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::OsRng;
use std::path::PathBuf;

/// The publisher key directory: `PLINTH_PUBLISHER_DIR`, or
/// `%APPDATA%\plinth\publisher` on Windows and
/// `$XDG_CONFIG_HOME/plinth/publisher` (or `~/.config/plinth/publisher`)
/// elsewhere. Never inside a project.
pub fn publisher_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PLINTH_PUBLISHER_DIR") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("plinth").join("publisher")
}

fn key_path() -> PathBuf {
    publisher_dir().join("key.ed25519")
}

fn name_path() -> PathBuf {
    publisher_dir().join("publisher.toml")
}

/// A publisher's name and signing key, loaded from the publisher directory.
pub struct PublisherIdentity {
    pub name: String,
    pub signing_key: SigningKey,
}

impl PublisherIdentity {
    /// The public key id shown to users and stored in signatures:
    /// `ed25519:<base64url of the 32-byte public key, no padding>`.
    pub fn key_id(&self) -> String {
        key_id(&self.signing_key.verifying_key())
    }
}

/// The key id of a verifying key: `ed25519:<base64url, no padding>`.
pub fn key_id(key: &VerifyingKey) -> String {
    use base64::Engine as _;
    format!("ed25519:{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.as_bytes()))
}

/// Parses a key id written by `key_id` back into a verifying key.
pub fn parse_key_id(id: &str) -> Result<VerifyingKey> {
    use base64::Engine as _;
    let b64 = id.strip_prefix("ed25519:").with_context(|| format!("`{id}` is not an ed25519 key id"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b64).with_context(|| format!("`{id}` is not valid base64url"))?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("`{id}` is not a 32-byte key"))?;
    VerifyingKey::from_bytes(&bytes).with_context(|| format!("`{id}` is not a valid ed25519 key"))
}

/// Generates a new key pair and writes it to the publisher directory.
/// Fails if a key already exists there, so `init` never silently
/// overwrites one (a user who wants a new key removes the directory, or
/// sets `PLINTH_PUBLISHER_DIR` to a new one).
pub fn init(name: &str) -> Result<PublisherIdentity> {
    let dir = publisher_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    if key_path().exists() {
        bail!("a publisher key already exists at {}", key_path().display());
    }
    let signing_key = SigningKey::generate(&mut OsRng);
    std::fs::write(key_path(), signing_key.to_bytes()).with_context(|| format!("write {}", key_path().display()))?;
    std::fs::write(name_path(), toml::to_string(&NameFile { name: name.to_owned() })?).with_context(|| format!("write {}", name_path().display()))?;
    Ok(PublisherIdentity { name: name.to_owned(), signing_key })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct NameFile {
    name: String,
}

/// Loads the publisher identity from the publisher directory.
pub fn load() -> Result<PublisherIdentity> {
    let key_bytes = std::fs::read(key_path())
        .with_context(|| format!("no publisher key at {}; run `plinth publisher init`", key_path().display()))?;
    let key_bytes: [u8; 32] = key_bytes.try_into().map_err(|_| anyhow::anyhow!("{} is not a 32-byte key", key_path().display()))?;
    let signing_key = SigningKey::from_bytes(&key_bytes);
    let name_text = std::fs::read_to_string(name_path()).with_context(|| format!("read {}", name_path().display()))?;
    let name_file: NameFile = toml::from_str(&name_text).with_context(|| format!("parse {}", name_path().display()))?;
    Ok(PublisherIdentity { name: name_file.name, signing_key })
}
