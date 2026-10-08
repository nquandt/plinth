//! The OS settings that the runtime theme follows (SPEC.md §6.5,
//! PRODUCTION U7): dark mode, high contrast, text size and reduced motion.
//!
//! - Dark mode: gpui-ce reports the window appearance and its changes (on
//!   Windows, the `ImmersiveColorSet` setting change); `PlinthRoot` observes
//!   it. This module reads only the overrides for it.
//! - Windows: high contrast from `SystemParametersInfo(SPI_GETHIGHCONTRAST)`
//!   and the high-contrast colors from `GetSysColor`; the text size from
//!   `HKCU\Software\Microsoft\Accessibility\TextScaleFactor` (100 to 225);
//!   reduced motion when "Animation effects" is off
//!   (`SPI_GETCLIENTAREAANIMATION`).
//! - Linux and macOS: only the overrides and the window appearance for now
//!   (`docs/PLATFORM-TODO.md`).
//!
//! gpui-ce does not give the `WM_SETTINGCHANGE` message of the other
//! settings to the app, so the host reads them again each `POLL` (one
//! second). A change refreshes each window, without a
//! restart. The `PLINTH_THEME`, `PLINTH_HIGH_CONTRAST`, `PLINTH_TEXT_SCALE`
//! and `PLINTH_REDUCED_MOTION` environment variables force a setting
//! (`SystemSettings::with_overrides`).

use gpui::App;
use plinth_ui::SystemSettings;
use std::time::Duration;

/// How often the host reads the OS settings again.
const POLL: Duration = Duration::from_secs(1);

/// The OS settings now, with the environment overrides applied.
pub fn current() -> SystemSettings {
    read_os().with_env_overrides()
}

/// Sets the `SystemSettings` global from the OS and keeps it up to date.
/// Call it one time per `gpui::App`, before the first window opens.
pub fn install(cx: &mut App) {
    cx.set_global(current());
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            let next = current();
            cx.update(|cx| apply(next, cx));
        }
    })
    .detach();
}

/// Stores `next` and draws each window again when it differs from the
/// current settings. Returns whether it changed.
pub fn apply(next: SystemSettings, cx: &mut App) -> bool {
    if cx.try_global::<SystemSettings>() == Some(&next) {
        return false;
    }
    log::info!("system settings changed: {next:?}");
    cx.set_global(next);
    cx.refresh_windows();
    true
}

#[cfg(windows)]
fn read_os() -> SystemSettings {
    windows_os::read()
}

#[cfg(not(windows))]
fn read_os() -> SystemSettings {
    SystemSettings::default()
}

#[cfg(windows)]
mod windows_os {
    use gpui::{rgb, rgb_to_hsla};
    use plinth_ui::{SystemColors, SystemSettings};
    use windows::Win32::Graphics::Gdi::{
        COLOR_BTNFACE, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
        GetSysColor, SYS_COLOR_INDEX,
    };
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
        SystemParametersInfoW,
    };
    use windows::core::BOOL;

    pub fn read() -> SystemSettings {
        let high_contrast = high_contrast();
        SystemSettings {
            dark: None,
            high_contrast,
            system_colors: high_contrast.then(system_colors),
            text_scale: text_scale(),
            reduced_motion: !client_area_animation(),
        }
    }

    fn high_contrast() -> bool {
        let mut hc = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        // SAFETY: `hc` is a valid HIGHCONTRASTW with its size set, as the
        // SPI_GETHIGHCONTRAST action needs.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                Some((&raw mut hc).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        ok.is_ok() && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
    }

    /// "Animation effects" (Settings > Accessibility > Visual effects).
    fn client_area_animation() -> bool {
        let mut on = BOOL(1);
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&raw mut on).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        ok.is_err() || on.as_bool()
    }

    /// "Text size" (Settings > Accessibility > Text size): 100 to 225 (%).
    fn text_scale() -> f32 {
        windows_registry::CURRENT_USER
            .open(r"Software\Microsoft\Accessibility")
            .and_then(|key| key.get_u32("TextScaleFactor"))
            .map_or(1.0, |percent| percent as f32 / 100.0)
    }

    fn sys_color(index: SYS_COLOR_INDEX) -> gpui::Hsla {
        // SAFETY: GetSysColor takes any index and returns a COLORREF (0x00BBGGRR).
        let bgr = unsafe { GetSysColor(index) };
        let (r, g, b) = (bgr & 0xff, (bgr >> 8) & 0xff, (bgr >> 16) & 0xff);
        rgb_to_hsla(rgb((r << 16) | (g << 8) | b))
    }

    fn system_colors() -> SystemColors {
        SystemColors {
            window: sys_color(COLOR_WINDOW),
            window_text: sys_color(COLOR_WINDOWTEXT),
            highlight: sys_color(COLOR_HIGHLIGHT),
            highlight_text: sys_color(COLOR_HIGHLIGHTTEXT),
            button_face: sys_color(COLOR_BTNFACE),
        }
    }

    #[cfg(test)]
    mod tests {
        /// Reads each setting of this machine; the values are the user's,
        /// so the test checks only that the read works and is in range.
        #[test]
        fn reads_the_windows_settings() {
            let s = super::read();
            assert!((1.0..=2.25).contains(&s.text_scale), "{s:?}");
            assert_eq!(s.high_contrast, s.system_colors.is_some());
        }
    }
}
