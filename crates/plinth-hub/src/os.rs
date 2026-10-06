//! Operating system integration for the Hub (`docs/HUB.md` §5.3, §10):
//! `plinth://` app links, the URL scheme registration on Windows, and
//! desktop shortcuts.
//!
//! The registration writes to a `Registry` trait, so a test uses
//! `MemoryRegistry` and never touches the real registry. On Windows,
//! `CurrentUserRegistry` writes to `HKEY_CURRENT_USER\Software\Classes`
//! (per user, no administrator rights).

use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The URL scheme of Plinth app links.
pub const SCHEME: &str = "plinth";

/// The registry key of the scheme, under `HKEY_CURRENT_USER`.
pub const SCHEME_KEY: &str = r"Software\Classes\plinth";

/// What a `plinth://` link opens (`docs/HUB.md` §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// `plinth://` or `plinth://hub`: the Hub UI.
    Hub,
    /// `plinth://app/<app id>` or `plinth://app/<app id>@<version>`.
    App { id: String, version: Option<String> },
}

/// Whether `id` is an app id or a version that is safe in a link, a file
/// name and a command line: letters, digits, `.`, `-`, `_` and `+`.
fn is_safe(text: &str) -> bool {
    !text.is_empty() && text.len() <= 200 && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Parses a `plinth://` link. Windows and browsers can add a trailing `/`;
/// it is ignored. Any other form is an error.
pub fn parse_link(url: &str) -> Result<Link> {
    let url = url.trim();
    let Some(rest) = url.strip_prefix("plinth://").or_else(|| url.strip_prefix("plinth:")) else {
        bail!("`{url}` is not a plinth:// link");
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest == "hub" {
        return Ok(Link::Hub);
    }
    let Some(app) = rest.strip_prefix("app/") else {
        bail!("`{url}` is not a plinth:// app link (use plinth://app/<app id>)");
    };
    let (id, version) = match app.split_once('@') {
        Some((id, version)) => (id, Some(version)),
        None => (app, None),
    };
    if !is_safe(id) || version.is_some_and(|v| !is_safe(v)) {
        bail!("`{url}` has an app id or a version with characters that a link cannot have");
    }
    Ok(Link::App { id: id.to_owned(), version: version.map(str::to_owned) })
}

/// The link that opens `id` (`docs/HUB.md` §5.3).
pub fn app_link(id: &str) -> String {
    format!("{SCHEME}://app/{id}")
}

/// The per-user registry, as much of it as the scheme registration needs.
pub trait Registry {
    /// Sets the string value `name` of `key` (`""` is the default value),
    /// and makes the key if necessary.
    fn set_string(&mut self, key: &str, name: &str, value: &str) -> Result<()>;
    /// Removes `key` and every key below it. A missing key is not an error.
    fn remove_tree(&mut self, key: &str) -> Result<()>;
}

/// A registry in memory, for tests and dry runs.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MemoryRegistry {
    /// key → (value name → value).
    pub keys: BTreeMap<String, BTreeMap<String, String>>,
}

impl Registry for MemoryRegistry {
    fn set_string(&mut self, key: &str, name: &str, value: &str) -> Result<()> {
        self.keys.entry(key.to_owned()).or_default().insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove_tree(&mut self, key: &str) -> Result<()> {
        let prefix = format!("{key}\\");
        self.keys.retain(|k, _| k != key && !k.starts_with(&prefix));
        Ok(())
    }
}

/// `HKEY_CURRENT_USER` on Windows.
#[cfg(windows)]
pub struct CurrentUserRegistry;

#[cfg(windows)]
impl Registry for CurrentUserRegistry {
    fn set_string(&mut self, key: &str, name: &str, value: &str) -> Result<()> {
        let key = windows_registry::CURRENT_USER.create(key)?;
        key.set_string(name, value)?;
        Ok(())
    }

    fn remove_tree(&mut self, key: &str) -> Result<()> {
        match windows_registry::CURRENT_USER.remove_tree(key) {
            Ok(()) => Ok(()),
            // ERROR_FILE_NOT_FOUND: there is nothing to remove.
            Err(e) if e.code().0 as u32 == 0x8007_0002 => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// The registry values that register `plinth://` so that Windows runs
/// `"<exe>" hub open "<link>"` (`docs/HUB.md` §10): `(key, value name,
/// value)`.
pub fn scheme_entries(exe: &Path) -> Vec<(String, String, String)> {
    let exe = exe.display().to_string();
    vec![
        (SCHEME_KEY.to_owned(), String::new(), "URL:Plinth app link".to_owned()),
        (SCHEME_KEY.to_owned(), "URL Protocol".to_owned(), String::new()),
        (format!(r"{SCHEME_KEY}\DefaultIcon"), String::new(), format!("\"{exe}\",0")),
        (format!(r"{SCHEME_KEY}\shell\open\command"), String::new(), format!("\"{exe}\" hub open \"%1\"")),
    ]
}

/// Registers the `plinth://` scheme for the current user. A second call
/// replaces the first (for example after the executable moved).
pub fn register_scheme(registry: &mut dyn Registry, exe: &Path) -> Result<()> {
    registry.remove_tree(SCHEME_KEY)?;
    for (key, name, value) in scheme_entries(exe) {
        registry.set_string(&key, &name, &value)?;
    }
    Ok(())
}

/// Removes the `plinth://` registration of the current user.
pub fn unregister_scheme(registry: &mut dyn Registry) -> Result<()> {
    registry.remove_tree(SCHEME_KEY)
}

/// The text of a Windows Internet Shortcut (`.url`) file that opens
/// `id` through the `plinth://` scheme, with the icon of `exe`.
pub fn shortcut_text(id: &str, exe: &Path) -> String {
    format!("[InternetShortcut]\r\nURL={}\r\nIconFile={}\r\nIconIndex=0\r\n", app_link(id), exe.display())
}

/// A file name for a shortcut to the app `name`: the characters that
/// Windows refuses in a file name become `_`.
pub fn shortcut_file_name(name: &str) -> String {
    let clean: String = name
        .trim()
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let clean = clean.trim_end_matches(['.', ' ']).to_owned();
    format!("{}.url", if clean.is_empty() { "Plinth app".to_owned() } else { clean })
}

/// Writes a shortcut (`docs/HUB.md` §10) that starts app `id` into `dir`,
/// named after the app. The shortcut uses the `plinth://` scheme, so
/// register it first (`register_scheme`). Returns the file path.
pub fn write_shortcut(dir: &Path, id: &str, name: &str, exe: &Path) -> Result<PathBuf> {
    if !is_safe(id) {
        bail!("`{id}` is not a valid app id");
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(shortcut_file_name(name));
    std::fs::write(&path, shortcut_text(id, exe))?;
    Ok(path)
}

/// The desktop folder of the current user, if this platform has one that
/// this crate knows.
pub fn desktop_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Desktop"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        assert_eq!(parse_link("plinth://").unwrap(), Link::Hub);
        assert_eq!(parse_link("plinth://hub/").unwrap(), Link::Hub);
        assert_eq!(parse_link("plinth://app/com.example.notes").unwrap(), Link::App { id: "com.example.notes".into(), version: None });
        assert_eq!(
            parse_link("plinth://app/com.example.notes@1.2.0/").unwrap(),
            Link::App { id: "com.example.notes".into(), version: Some("1.2.0".into()) }
        );
        assert!(parse_link("https://example.com").is_err());
        assert!(parse_link("plinth://other/x").is_err());
        // Nothing that could escape a command line or a path.
        assert!(parse_link("plinth://app/a\"b").is_err());
        assert!(parse_link("plinth://app/..\\x").is_err());
        assert!(parse_link("plinth://app/a b").is_err());
        assert!(parse_link("plinth://app/").is_err());
    }

    #[test]
    fn registers_and_unregisters_the_scheme_in_a_memory_registry() {
        let mut reg = MemoryRegistry::default();
        reg.set_string(r"Software\Classes\other", "", "keep me").unwrap();
        let exe = Path::new(r"C:\Program Files\Plinth\plinth.exe");
        register_scheme(&mut reg, exe).unwrap();
        assert_eq!(reg.keys[SCHEME_KEY][""], "URL:Plinth app link");
        assert_eq!(reg.keys[SCHEME_KEY]["URL Protocol"], "");
        assert_eq!(reg.keys[r"Software\Classes\plinth\shell\open\command"][""], r#""C:\Program Files\Plinth\plinth.exe" hub open "%1""#);
        assert_eq!(reg.keys[r"Software\Classes\plinth\DefaultIcon"][""], r#""C:\Program Files\Plinth\plinth.exe",0"#);

        // Registering again replaces the old command.
        register_scheme(&mut reg, Path::new(r"D:\plinth.exe")).unwrap();
        assert_eq!(reg.keys[r"Software\Classes\plinth\shell\open\command"][""], r#""D:\plinth.exe" hub open "%1""#);

        unregister_scheme(&mut reg).unwrap();
        assert!(!reg.keys.keys().any(|k| k.starts_with(SCHEME_KEY)));
        assert_eq!(reg.keys[r"Software\Classes\other"][""], "keep me");
    }

    #[test]
    fn writes_a_shortcut_file() {
        let dir = std::env::temp_dir().join(format!("plinth-shortcut-test-{}", std::process::id()));
        let path = write_shortcut(&dir, "com.example.notes", "Notes: daily/work?", Path::new(r"C:\plinth.exe")).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Notes_ daily_work_.url");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("URL=plinth://app/com.example.notes\r\n"), "{text}");
        assert!(text.contains(r"IconFile=C:\plinth.exe"), "{text}");
        assert!(write_shortcut(&dir, "bad id", "x", Path::new("x")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
