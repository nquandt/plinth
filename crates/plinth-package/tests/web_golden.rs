//! The web host and the desktop host give the same colors for the same
//! manifest. `web/test/fixtures/theme-golden.json` lists the inputs and the
//! CSS variables that the Rust rules (`plinth_package::theme`) give; the
//! Node test `web/test/run-theme.mjs` checks that `web/theme.js` gives the
//! same. Run with `PLINTH_BLESS=1` to write the variables of the `ok` cases.

use plinth_package::theme::{ThemeOverride, web_vars};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/test/fixtures/theme-golden.json")
}

fn theme_of(case: &Value) -> Result<(ThemeOverride, Option<String>), String> {
    let table: toml::Value = toml::from_str(case["theme_toml"].as_str().unwrap()).map_err(|e| e.to_string())?;
    let theme = ThemeOverride::from_toml(table.get("theme").ok_or("no [theme]")?).map_err(|e| e.to_string())?;
    Ok((theme, case["accent"].as_str().map(str::to_owned)))
}

fn vars(p: &plinth_package::theme::Palette, dark: bool) -> Value {
    Value::Object(web_vars(p, dark).into_iter().map(|(k, v)| (k.to_owned(), Value::String(v))).collect::<Map<_, _>>())
}

#[test]
fn rust_rules_match_the_golden_file() {
    let text = std::fs::read_to_string(path()).unwrap();
    let mut golden: Value = serde_json::from_str(&text).unwrap();
    let bless = std::env::var("PLINTH_BLESS").is_ok_and(|v| v == "1");
    for case in golden["ok"].as_array_mut().unwrap() {
        let name = case["name"].as_str().unwrap().to_owned();
        let (theme, accent) = theme_of(case).unwrap_or_else(|e| panic!("{name}: {e}"));
        let resolved = theme.resolve(accent.as_deref()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (light, dark) = (vars(&resolved.light, false), vars(&resolved.dark, true));
        if bless {
            case["light"] = light;
            case["dark"] = dark;
        } else {
            assert_eq!(case["light"], light, "{name}: light");
            assert_eq!(case["dark"], dark, "{name}: dark");
        }
    }
    for case in golden["fail"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let err = match theme_of(case) {
            Err(e) => e,
            Ok((theme, accent)) => theme.resolve(accent.as_deref()).expect_err(name).to_string(),
        };
        for needle in case["error_contains"].as_array().unwrap() {
            let needle = needle.as_str().unwrap();
            assert!(err.contains(needle), "{name}: `{err}` lacks `{needle}`");
        }
    }
    if bless {
        let mut out = serde_json::to_string_pretty(&golden).unwrap();
        out.push('\n');
        std::fs::write(path(), out).unwrap();
    }
    // The blessed output is the same shape: the file has `light` and `dark` for each case.
    let _ = json!(null);
}
