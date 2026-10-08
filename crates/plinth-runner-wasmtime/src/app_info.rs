//! `plinth:app-info` (core 1.13, `PRODUCTION.md` D4): the app's stored
//! upgrade state, `data-version()`/`mark-data-version()`, and semver
//! compare for the "first install / downgrade / no hook" rules.
//!
//! Storage: a small text file `data-version`, in the same folder as the
//! app's `store.kv` file (`crate::kv::Kv::open`'s `<data-dir>/apps/<id>/`).
//! A runner with no kv folder (tests, `plinth test`) keeps the data version
//! in memory only.

use std::cmp::Ordering;
use std::path::Path;

/// Reads the stored data version from `path` (the `data-version` file next
/// to `kv.json`, `crate::kv::Kv::folder`), or `None` if there is no file,
/// it could not be read, or it is empty.
pub fn read(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() { None } else { Some(text.to_owned()) }
}

/// Writes `version` as the stored data version to `path`, creating its
/// parent folder first if needed.
pub fn write(path: &Path, version: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, version)
}

/// Whether `mark-data-version`/`note_start_without_hook` should store
/// `version` as the data version, given the version currently stored
/// (`None` on first install): true unless the stored one is higher than
/// `version` (a downgrade keeps the higher, already-stored version).
pub fn should_store(stored: Option<&str>, version: &str) -> bool {
    match stored {
        None => true,
        Some(s) => cmp_versions(version, s) != Ordering::Less,
    }
}

/// Numeric `MAJOR.MINOR.PATCH` compare. A pre-release tag (`-rc.1`) is
/// lower than the same release with none; two pre-releases of the same
/// release compare equal (this is intentionally simple: it does not order
/// pre-release identifiers against each other, SPEC.md keeps that out of
/// scope for `onUpgrade`). A missing or non-numeric component reads as 0.
pub fn cmp_versions(a: &str, b: &str) -> Ordering {
    let (a_core, a_pre) = split_version(a);
    let (b_core, b_pre) = split_version(b);
    a_core.cmp(&b_core).then_with(|| match (a_pre, b_pre) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        _ => Ordering::Equal,
    })
}

/// `(MAJOR, MINOR, PATCH)` and whether `version` has a pre-release suffix.
fn split_version(version: &str) -> ([u64; 3], bool) {
    let (core, pre) = match version.split_once('-') {
        Some((core, _)) => (core, true),
        None => (version, false),
    };
    let mut parts = [0u64; 3];
    for (i, part) in core.split('.').take(3).enumerate() {
        parts[i] = part.parse().unwrap_or(0);
    }
    (parts, pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_compare() {
        assert_eq!(cmp_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(cmp_versions("1.2.0", "1.10.0"), Ordering::Less);
        assert_eq!(cmp_versions("2.0.0", "1.9.9"), Ordering::Greater);
        assert_eq!(cmp_versions("1.0.10", "1.0.2"), Ordering::Greater);
    }

    #[test]
    fn pre_release_is_lower() {
        assert_eq!(cmp_versions("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(cmp_versions("1.0.0", "1.0.0-rc.1"), Ordering::Greater);
        assert_eq!(cmp_versions("1.0.0-rc.1", "1.0.0-rc.2"), Ordering::Equal);
    }

    #[test]
    fn missing_or_bad_components_read_as_zero() {
        assert_eq!(cmp_versions("1.2", "1.2.0"), Ordering::Equal);
        assert_eq!(cmp_versions("1", "1.0.0"), Ordering::Equal);
        assert_eq!(cmp_versions("x.y.z", "0.0.0"), Ordering::Equal);
    }

    #[test]
    fn should_store_first_install_and_upgrade() {
        // First install: nothing stored yet.
        assert!(should_store(None, "1.0.0"));
        // An upgrade or the same version: store it.
        assert!(should_store(Some("1.0.0"), "1.1.0"));
        assert!(should_store(Some("1.0.0"), "1.0.0"));
    }

    #[test]
    fn should_store_keeps_the_higher_version_on_downgrade() {
        assert!(!should_store(Some("2.0.0"), "1.5.0"));
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = std::env::temp_dir().join(format!("plinth-app-info-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("apps").join("com.example.notes").join("data-version");

        assert_eq!(read(&path), None);
        write(&path, "1.2.0").unwrap();
        assert_eq!(read(&path), Some("1.2.0".to_owned()));
        write(&path, "1.3.0").unwrap();
        assert_eq!(read(&path), Some("1.3.0".to_owned()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_of_an_empty_file_is_none() {
        let dir = std::env::temp_dir().join(format!("plinth-app-info-test-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("data-version");
        write(&path, "   \n").unwrap();
        assert_eq!(read(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The file lands in the same folder as `kv.json`, keyed the same way
    /// (`crate::kv::Kv::folder`): `<data-dir>/apps/<id>/data-version`.
    #[test]
    fn file_lands_next_to_kv_json() {
        use crate::kv::Kv;

        let dir = std::env::temp_dir().join(format!("plinth-app-info-test-kv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let kv = Kv::open(&dir, "com.example.notes").unwrap();
        let folder = kv.folder().expect("a real folder").to_owned();
        assert_eq!(folder, dir.join("apps").join("com.example.notes"));

        let path = folder.join("data-version");
        write(&path, "0.1.0").unwrap();
        assert_eq!(read(&path), Some("0.1.0".to_owned()));
        assert!(path.starts_with(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `Kv::in_memory` has no folder: `note_start_without_hook`/
    /// `mark-data-version` then keep the data version in memory only.
    #[test]
    fn in_memory_kv_has_no_folder() {
        use crate::kv::Kv;

        let kv = Kv::in_memory();
        assert_eq!(kv.folder(), None);
    }
}
