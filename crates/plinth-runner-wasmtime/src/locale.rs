//! `plinth:locale` (core 1.13, `docs/I18N.md` §3.1, §4 step 1): the locale
//! of the app as a BCP 47 tag, and its text direction.
//!
//! The desktop host reads the OS locale (`sys-locale`: Windows
//! `GetUserDefaultLocaleName`, macOS `CFLocale`, Linux `LANG`). The
//! environment variable `PLINTH_LOCALE` overrides it, for manual tests. A test
//! gives a fixed locale with `Guest::set_locale`.
//!
//! `is_rtl` uses the same short lists as `isRtlLocale` in
//! `web/plinth-web.js`, so both hosts agree. Keep the two lists the same.

/// The locale when the OS gives none (or gives `C`/`POSIX`).
pub const FALLBACK: &str = "en-US";

/// The environment variable that overrides the OS locale (manual tests).
pub const ENV_OVERRIDE: &str = "PLINTH_LOCALE";

/// Language subtags whose default script is written right to left.
const RTL_LANGUAGES: &[&str] = &[
    "ar", "arc", "azb", "ckb", "dv", "fa", "he", "iw", "ji", "ks", "lrc", "mzn", "nqo", "pnb",
    "ps", "sd", "syr", "ug", "ur", "yi",
];

/// Script subtags that are written right to left.
const RTL_SCRIPTS: &[&str] = &[
    "Adlm", "Arab", "Hebr", "Mand", "Nkoo", "Rohg", "Samr", "Syrc", "Thaa",
];

/// The locale of the OS (or of `PLINTH_LOCALE`), normalized.
pub fn os_locale() -> String {
    if let Ok(tag) = std::env::var(ENV_OVERRIDE)
        && !tag.trim().is_empty()
    {
        return normalize(&tag);
    }
    normalize(&sys_locale::get_locale().unwrap_or_default())
}

/// Normalizes a locale name to a BCP 47 tag: `_` becomes `-`, a POSIX
/// encoding or modifier (`.UTF-8`, `@euro`) goes away, the language is lower
/// case, a 4-letter script is title case, and a region is upper case
/// (`en_us.UTF-8` → `en-US`, `zh_hans_cn` → `zh-Hans-CN`). Other subtags
/// stay as they are (lower case). An empty name, `C` or `POSIX` gives
/// `en-US`.
pub fn normalize(name: &str) -> String {
    let name = name.trim();
    let name = name.split(['.', '@']).next().unwrap_or("");
    if name.is_empty() || name.eq_ignore_ascii_case("c") || name.eq_ignore_ascii_case("posix") {
        return FALLBACK.to_owned();
    }
    let mut out = String::with_capacity(name.len());
    for (i, part) in name.split(['-', '_']).filter(|p| !p.is_empty()).enumerate() {
        if !part.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        if i > 0 {
            out.push('-');
        }
        let lower = part.to_ascii_lowercase();
        if i == 0 {
            out.push_str(&lower);
        } else if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut chars = lower.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.extend(chars);
            }
        } else if (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
            || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()))
        {
            out.push_str(&part.to_ascii_uppercase());
        } else {
            out.push_str(&lower);
        }
    }
    if out.is_empty() || out.starts_with('-') {
        FALLBACK.to_owned()
    } else {
        out
    }
}

/// True when the locale `tag` (normalized) is written right to left: its
/// script subtag is a right-to-left script, or it has no script subtag and
/// its language is in the list (`ar-EG`, `he`, `fa-IR`, `ur`; but
/// `az-Arab` is right to left and `ku` is not).
pub fn is_rtl(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or("").to_ascii_lowercase();
    if let Some(script) = parts
        .next()
        .filter(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return RTL_SCRIPTS.iter().any(|s| s.eq_ignore_ascii_case(script));
    }
    RTL_LANGUAGES.contains(&language.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_gives_bcp47() {
        assert_eq!(normalize("en-US"), "en-US");
        assert_eq!(normalize("en_us.UTF-8"), "en-US");
        assert_eq!(normalize("de_DE@euro"), "de-DE");
        assert_eq!(normalize("zh_hans_cn"), "zh-Hans-CN");
        assert_eq!(normalize("ES-419"), "es-419");
        assert_eq!(normalize("fr"), "fr");
        assert_eq!(normalize(""), "en-US");
        assert_eq!(normalize("C"), "en-US");
        assert_eq!(normalize("POSIX"), "en-US");
        assert_eq!(normalize("C.UTF-8"), "en-US");
    }

    #[test]
    fn rtl_from_language_or_script() {
        for tag in [
            "ar",
            "ar-EG",
            "he",
            "he-IL",
            "fa-IR",
            "ur-PK",
            "ps",
            "sd",
            "ug",
            "yi",
            "dv",
            "ckb",
            "az-Arab",
            "pa-Arab-PK",
        ] {
            assert!(is_rtl(tag), "{tag} is right to left");
        }
        for tag in [
            "en-US",
            "fr",
            "zh-Hans-CN",
            "ku",
            "az-Latn",
            "sr-Cyrl",
            "ja",
            "hi-IN",
            "uz-Latn-UZ",
        ] {
            assert!(!is_rtl(tag), "{tag} is left to right");
        }
    }
}
