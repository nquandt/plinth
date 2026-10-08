//! The desktop `NotifyBackend` for `plinth:notify` (core 1.12,
//! `docs/PRODUCTION.md` H3).
//!
//! - Windows 10 and later: a toast in the Action Center, through the WinRT
//!   `ToastNotificationManager`. An app that is not installed as a package
//!   needs an AppUserModelID with a display name; the backend registers one
//!   for each app under `HKCU\Software\Classes\AppUserModelId\Plinth.<id>`
//!   (the same method as the Windows Community Toolkit uses for unpackaged
//!   apps). No administrator rights are necessary.
//! - Linux and macOS: no backend yet. `show` answers `Unsupported`, so the
//!   app can show the message in its own window.
//!
//! The tests use `MemoryNotify` from the runner; this backend changes the
//! user's notification area, so no automatic test calls it.

use plinth_runner_wasmtime::notify::{NotifyBackend, NotifyOutcome};

/// The notification backend of this platform for the app `app_id`.
pub fn system_notify(app_id: &str) -> Box<dyn NotifyBackend> {
    #[cfg(windows)]
    return Box::new(windows_toast::Toasts::new(app_id));
    #[cfg(not(windows))]
    {
        let _ = app_id;
        Box::new(Unsupported)
    }
}

/// A backend for a platform with no notification support yet.
#[cfg(not(windows))]
struct Unsupported;

#[cfg(not(windows))]
impl NotifyBackend for Unsupported {
    fn show(&mut self, _title: &str, _body: Option<&str>, _tag: Option<&str>) -> NotifyOutcome {
        NotifyOutcome::Unsupported
    }
    fn cancel(&mut self, _tag: &str) {}
}

/// The AppUserModelID of an app: `Plinth.` and the app id, with each
/// character that is not a letter, a digit, `.` or `-` changed to `-`, at
/// most 128 characters (the Windows limit is 129).
pub fn app_user_model_id(app_id: &str) -> String {
    let clean: String =
        app_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' }).collect();
    let mut id = format!("Plinth.{clean}");
    id.truncate(128);
    id
}

/// Escapes text for the toast XML.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 does not allow most control characters.
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The toast XML for `title` and `body` (the `ToastGeneric` template).
pub fn toast_xml(title: &str, body: Option<&str>) -> String {
    let mut xml = String::from("<toast><visual><binding template=\"ToastGeneric\"><text>");
    xml.push_str(&xml_escape(title));
    xml.push_str("</text>");
    if let Some(body) = body {
        xml.push_str("<text>");
        xml.push_str(&xml_escape(body));
        xml.push_str("</text>");
    }
    xml.push_str("</binding></visual></toast>");
    xml
}

/// The toast tag for an app tag. Windows allows at most 64 characters.
pub fn toast_tag(tag: &str) -> String {
    tag.chars().take(64).collect()
}

#[cfg(windows)]
mod windows_toast {
    use super::*;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    /// The group of every Plinth toast, so `cancel` finds a tag.
    const GROUP: &str = "plinth";

    pub struct Toasts {
        app_id: String,
        aumid: String,
        /// True after the AppUserModelID is in the registry.
        registered: bool,
    }

    impl Toasts {
        pub fn new(app_id: &str) -> Self {
            Self { app_id: app_id.to_owned(), aumid: app_user_model_id(app_id), registered: false }
        }

        /// Registers the AppUserModelID with a display name (one time).
        fn register(&mut self) {
            if self.registered {
                return;
            }
            let path = format!("Software\\Classes\\AppUserModelId\\{}", self.aumid);
            match windows_registry::CURRENT_USER.create(&path).and_then(|k| k.set_string("DisplayName", &self.app_id)) {
                Ok(()) => self.registered = true,
                Err(e) => log::warn!("notify: cannot register {}: {e}", self.aumid),
            }
        }

        fn try_show(&mut self, title: &str, body: Option<&str>, tag: Option<&str>) -> windows::core::Result<()> {
            init_winrt();
            self.register();
            let doc = XmlDocument::new()?;
            doc.LoadXml(&HSTRING::from(toast_xml(title, body)))?;
            let toast = ToastNotification::CreateToastNotification(&doc)?;
            if let Some(tag) = tag {
                toast.SetTag(&HSTRING::from(toast_tag(tag)))?;
                toast.SetGroup(&HSTRING::from(GROUP))?;
            }
            let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&self.aumid))?;
            notifier.Show(&toast)
        }
    }

    /// WinRT calls need an initialized thread. The gpui main thread is
    /// already initialized; then this call fails with no effect.
    fn init_winrt() {
        // SAFETY: a plain system call with no pointers.
        let _ = unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) };
    }

    impl NotifyBackend for Toasts {
        fn show(&mut self, title: &str, body: Option<&str>, tag: Option<&str>) -> NotifyOutcome {
            match self.try_show(title, body, tag) {
                Ok(()) => NotifyOutcome::Shown,
                Err(e) => {
                    log::warn!("notify: the toast failed: {e}");
                    NotifyOutcome::Unsupported
                }
            }
        }

        fn cancel(&mut self, tag: &str) {
            init_winrt();
            let result = ToastNotificationManager::History().and_then(|h| {
                h.RemoveGroupedTagWithId(&HSTRING::from(toast_tag(tag)), &HSTRING::from(GROUP), &HSTRING::from(&self.aumid))
            });
            if let Err(e) = result {
                log::debug!("notify: cancel {tag}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_user_model_ids_are_clean_and_short() {
        assert_eq!(app_user_model_id("com.example.notes"), "Plinth.com.example.notes");
        assert_eq!(app_user_model_id("a b/c\\d"), "Plinth.a-b-c-d");
        assert_eq!(app_user_model_id(&"x".repeat(300)).len(), 128);
    }

    #[test]
    fn toast_xml_escapes_the_text() {
        assert_eq!(
            toast_xml("A & B", Some("<3 \"x\"")),
            "<toast><visual><binding template=\"ToastGeneric\"><text>A &amp; B</text><text>&lt;3 &quot;x&quot;</text></binding></visual></toast>"
        );
        assert_eq!(toast_xml("T", None), "<toast><visual><binding template=\"ToastGeneric\"><text>T</text></binding></visual></toast>");
        assert_eq!(xml_escape("a\u{1}b"), "a b");
    }

    /// Shows a real toast and removes it again. It changes the user's
    /// notification area, so it runs only by hand:
    /// `cargo test -p plinth-host-desktop --lib notify -- --ignored`.
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn a_real_toast_shows_and_cancels() {
        let mut backend = system_notify("dev.plinth.notify-test");
        assert_eq!(backend.show("Plinth test", Some("This toast goes away at once."), Some("test")), NotifyOutcome::Shown);
        backend.cancel("test");
    }

    #[test]
    fn toast_tags_are_at_most_64_characters() {
        assert_eq!(toast_tag("job"), "job");
        assert_eq!(toast_tag(&"é".repeat(100)).chars().count(), 64);
    }
}
