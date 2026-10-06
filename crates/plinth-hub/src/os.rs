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

/// The Start menu folder for Plinth apps of the current user
/// (`%APPDATA%\Microsoft\Windows\Start Menu\Programs\Plinth`), if this
/// platform has one. No administrator rights are necessary.
pub fn start_menu_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Plinth"))
    } else {
        None
    }
}

/// The program that a shortcut or a `plinth://` link runs: `plinthw.exe`
/// next to `cli` if it is there, else `cli`. `plinthw` is a small program
/// for the Windows subsystem that runs `plinth.exe` with no console
/// window, so a click on a shortcut or a link does not show a console for
/// a moment (`docs/HUB.md` §10).
pub fn gui_launcher(cli: &Path) -> PathBuf {
    let launcher = cli.with_file_name(format!("plinthw{}", std::env::consts::EXE_SUFFIX));
    if launcher.is_file() { launcher } else { cli.to_path_buf() }
}

/// The arguments that a shortcut gives the launcher to open `id`.
pub fn shortcut_arguments(id: &str) -> String {
    format!("hub open {}", app_link(id))
}

// -- Icons --------------------------------------------------------------------

/// The width and the height of a PNG image, from its `IHDR` chunk, or
/// `None` if `png` is not a PNG.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || &png[..8] != SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(png[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

/// A Windows icon (`.ico`) that holds `png` as its one image. Windows
/// Vista and later read PNG images in an icon file. `None` if `png` is not
/// a PNG image.
pub fn png_to_ico(png: &[u8]) -> Option<Vec<u8>> {
    let (width, height) = png_size(png)?;
    // In an icon directory entry, 0 means 256 (or more) pixels.
    let dim = |d: u32| if d >= 256 { 0u8 } else { d as u8 };
    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // reserved, type 1 (icon), one image
    ico.push(dim(width));
    ico.push(dim(height));
    ico.push(0); // no palette
    ico.push(0); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // the image starts after this header
    ico.extend_from_slice(png);
    Some(ico)
}

// -- Windows shell links (`.lnk`) -------------------------------------------

/// What a `.lnk` file starts.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellLink {
    /// The program, as an absolute path.
    pub target: PathBuf,
    pub arguments: String,
    pub working_dir: Option<PathBuf>,
    /// The tooltip text.
    pub description: String,
    /// An `.ico` file or a program with an icon resource.
    pub icon: PathBuf,
    pub icon_index: i32,
}

#[cfg(windows)]
fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    text.encode_wide().chain([0]).collect()
}

/// Calls `body` with COM ready on this thread (`CoInitializeEx`), and
/// balances the call. A thread that already uses another COM mode (for
/// example a UI thread) can still make a shell link.
#[cfg(windows)]
fn with_com<T>(body: impl FnOnce() -> Result<T>) -> Result<T> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    // SAFETY: no reserved pointer; every successful call is balanced below.
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let result = body();
    if init.is_ok() {
        // SAFETY: balances the successful `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    result
}

/// Saves `link` as the `.lnk` file `path` through the Windows shell
/// (`IShellLinkW`), so that the link has the item ID list and the link
/// information that Explorer expects.
#[cfg(windows)]
pub fn save_shell_link(path: &Path, link: &ShellLink) -> Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink as ShellLinkClass};
    use windows::core::{Interface as _, PCWSTR};
    with_com(|| {
        let target = wide(link.target.as_os_str());
        let arguments = wide(std::ffi::OsStr::new(&link.arguments));
        let description = wide(std::ffi::OsStr::new(&link.description));
        let icon = wide(link.icon.as_os_str());
        let file = wide(path.as_os_str());
        // SAFETY: each string is a NUL-terminated UTF-16 buffer that lives
        // until the end of this closure; the interfaces come from COM.
        unsafe {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLinkClass, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(PCWSTR(target.as_ptr()))?;
            shell_link.SetArguments(PCWSTR(arguments.as_ptr()))?;
            shell_link.SetDescription(PCWSTR(description.as_ptr()))?;
            shell_link.SetIconLocation(PCWSTR(icon.as_ptr()), link.icon_index)?;
            if let Some(dir) = &link.working_dir {
                let dir = wide(dir.as_os_str());
                shell_link.SetWorkingDirectory(PCWSTR(dir.as_ptr()))?;
            }
            let persist: IPersistFile = shell_link.cast()?;
            persist.Save(PCWSTR(file.as_ptr()), true)?;
        }
        Ok(())
    })
}

/// Reads the `.lnk` file `path` back through the Windows shell: the
/// target, the arguments and the icon location. For tests and checks.
#[cfg(windows)]
pub fn read_shell_link(path: &Path) -> Result<ShellLink> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink as ShellLinkClass};
    use windows::core::{Interface as _, PCWSTR};
    fn text(buffer: &[u16]) -> String {
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }
    with_com(|| {
        let file = wide(path.as_os_str());
        // SAFETY: the buffers are large enough for each call (MAX_PATH and
        // the 1024 characters that `IShellLinkW` allows for arguments).
        unsafe {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLinkClass, None, CLSCTX_INPROC_SERVER)?;
            let persist: IPersistFile = shell_link.cast()?;
            persist.Load(PCWSTR(file.as_ptr()), STGM_READ)?;
            let mut target = [0u16; 1024];
            shell_link.GetPath(&mut target, std::ptr::null_mut(), 0)?;
            let mut arguments = [0u16; 1024];
            shell_link.GetArguments(&mut arguments)?;
            let mut description = [0u16; 1024];
            shell_link.GetDescription(&mut description)?;
            let mut dir = [0u16; 1024];
            shell_link.GetWorkingDirectory(&mut dir)?;
            let mut icon = [0u16; 1024];
            let mut icon_index = 0;
            shell_link.GetIconLocation(&mut icon, &mut icon_index)?;
            let dir = text(&dir);
            Ok(ShellLink {
                target: PathBuf::from(text(&target)),
                arguments: text(&arguments),
                working_dir: (!dir.is_empty()).then(|| PathBuf::from(dir)),
                description: text(&description),
                icon: PathBuf::from(text(&icon)),
                icon_index,
            })
        }
    })
}

#[cfg(not(windows))]
pub fn save_shell_link(_path: &Path, _link: &ShellLink) -> Result<()> {
    bail!("`.lnk` shortcuts are for Windows only (docs/HUB.md §10); use --url")
}

/// A file name for a `.lnk` shortcut to the app `name`.
pub fn link_file_name(name: &str) -> String {
    let url = shortcut_file_name(name);
    format!("{}.lnk", url.trim_end_matches(".url"))
}

/// Writes a `.lnk` shortcut (`docs/HUB.md` §10) into `dir` that runs
/// `launcher hub open plinth://app/<id>`, with `icon` (the app's `.ico`)
/// or else the icon of `launcher`. Unlike `write_shortcut`, it does not
/// need the `plinth://` scheme. Returns the file path.
pub fn write_link_shortcut(dir: &Path, id: &str, name: &str, launcher: &Path, icon: Option<&Path>) -> Result<PathBuf> {
    if !is_safe(id) {
        bail!("`{id}` is not a valid app id");
    }
    std::fs::create_dir_all(dir)?;
    let path = dir.join(link_file_name(name));
    let link = ShellLink {
        target: launcher.to_path_buf(),
        arguments: shortcut_arguments(id),
        working_dir: launcher.parent().map(Path::to_path_buf),
        description: format!("Open {} (Plinth app {id})", name.trim()),
        icon: icon.unwrap_or(launcher).to_path_buf(),
        icon_index: 0,
    };
    save_shell_link(&path, &link)?;
    Ok(path)
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

    /// A tiny 2x3 PNG header (the image data does not matter here).
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 1, 2, 3, 4]);
        png
    }

    #[test]
    fn makes_an_icon_from_a_png() {
        let image = png(48, 32);
        assert_eq!(png_size(&image), Some((48, 32)));
        let ico = png_to_ico(&image).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!((ico[6], ico[7]), (48, 32));
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize, image.len());
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 22);
        assert_eq!(&ico[22..], &image[..]);
        // 256 pixels or more is 0 in the directory entry.
        let big = png_to_ico(&png(512, 256)).unwrap();
        assert_eq!((big[6], big[7]), (0, 0));
        assert!(png_to_ico(b"GIF89a....................").is_none());
    }

    /// The shortcut goes only into the folder that the caller gives
    /// (`--dir`, or the Start menu folder that the CLI looks up), never a
    /// real user folder in a test. The Windows shell reads it back.
    #[cfg(windows)]
    #[test]
    fn writes_a_link_shortcut_with_the_app_icon_or_the_launcher_icon() {
        let dir = std::env::temp_dir().join(format!("plinth-lnk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A real file as the target, so that the shell can resolve it.
        let launcher = dir.join("plinthw.exe");
        std::fs::write(&launcher, b"MZ").unwrap();
        let icon = dir.join("com.example.notes.ico");
        std::fs::write(&icon, png_to_ico(&png(32, 32)).unwrap()).unwrap();

        let path = write_link_shortcut(&dir, "com.example.notes", "Notes: daily", &launcher, Some(&icon)).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Notes_ daily.lnk");
        let link = read_shell_link(&path).unwrap();
        assert_eq!(link.target, launcher);
        assert_eq!(link.arguments, "hub open plinth://app/com.example.notes");
        assert_eq!(link.working_dir.as_deref(), Some(dir.as_path()));
        assert_eq!(link.icon, icon);
        assert_eq!(link.description, "Open Notes: daily (Plinth app com.example.notes)");

        let path = write_link_shortcut(&dir, "com.example.other", "Other", &launcher, None).unwrap();
        assert_eq!(read_shell_link(&path).unwrap().icon, launcher);
        assert!(write_link_shortcut(&dir, "bad id", "x", &launcher, None).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_launcher_is_plinthw_when_it_is_next_to_the_cli() {
        let dir = std::env::temp_dir().join(format!("plinth-launcher-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cli = dir.join(format!("plinth{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(gui_launcher(&cli), cli);
        let launcher = dir.join(format!("plinthw{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&launcher, b"").unwrap();
        assert_eq!(gui_launcher(&cli), launcher);
        std::fs::remove_dir_all(&dir).ok();
    }
}
