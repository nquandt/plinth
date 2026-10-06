//! The `store.kv` host API implementation (SPEC.md §8.5, §9.4).
//!
//! The whole store lives in memory and is flushed to one JSON file on
//! every write, keyed by app id: `<data-dir>/apps/<id>/kv.json` on
//! desktop (`%APPDATA%\plinth` on Windows). Tests use a store with no
//! backing file (`Kv::in_memory`).

use anyhow::{Context as _, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A per-app key/value store.
pub struct Kv {
    path: Option<PathBuf>,
    data: BTreeMap<String, String>,
}

impl Kv {
    /// Loads (or creates) the store for `app_id` under `data_dir` (see
    /// `data_dir()` for the platform default).
    pub fn open(data_dir: &Path, app_id: &str) -> Result<Self> {
        let path = data_dir.join("apps").join(app_id).join("kv.json");
        let data = if path.exists() {
            let text = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?
        } else {
            BTreeMap::new()
        };
        Ok(Self { path: Some(path), data })
    }

    /// A store with no backing file, for tests.
    pub fn in_memory() -> Self {
        Self { path: None, data: BTreeMap::new() }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.data.get(key).cloned()
    }

    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        self.data.insert(key, value);
        self.flush()
    }

    pub fn delete(&mut self, key: &str) -> Result<()> {
        self.data.remove(key);
        self.flush()
    }

    pub fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    fn flush(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string(&self.data)?;
        std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
    }
}

/// The platform data directory for Plinth apps (`%APPDATA%\plinth` on
/// Windows; `$XDG_DATA_HOME/plinth` or `~/.local/share/plinth` elsewhere).
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("plinth");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("plinth");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local").join("share").join("plinth");
    }
    PathBuf::from(".plinth")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("plinth-kv-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        {
            let mut kv = Kv::open(&dir, "com.example.notes").unwrap();
            assert_eq!(kv.get("a"), None);
            kv.set("a".into(), "1".into()).unwrap();
            kv.set("b".into(), "2".into()).unwrap();
        }

        let mut kv = Kv::open(&dir, "com.example.notes").unwrap();
        assert_eq!(kv.get("a"), Some("1".into()));
        assert_eq!(kv.get("b"), Some("2".into()));
        let mut keys = kv.keys();
        keys.sort();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);

        kv.delete("a").unwrap();
        assert_eq!(kv.get("a"), None);

        let kv = Kv::open(&dir, "com.example.notes").unwrap();
        assert_eq!(kv.get("a"), None);
        assert_eq!(kv.get("b"), Some("2".into()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_memory_does_not_touch_disk() {
        let mut kv = Kv::in_memory();
        kv.set("x".into(), "y".into()).unwrap();
        assert_eq!(kv.get("x"), Some("y".into()));
    }

    #[test]
    fn apps_are_isolated_by_id() {
        let dir = std::env::temp_dir().join(format!("plinth-kv-test-iso-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut a = Kv::open(&dir, "com.example.a").unwrap();
        a.set("k".into(), "a-value".into()).unwrap();
        let b = Kv::open(&dir, "com.example.b").unwrap();
        assert_eq!(b.get("k"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
