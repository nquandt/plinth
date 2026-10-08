//! `plinth build --target web` (SPEC.md §10.3): an app for any static web
//! host. The folder form writes `index.html`, `plinth.js`, the core file and
//! the unchanged `.plnt`; `--single-file` writes one HTML file with the
//! loader, the core and the package inline.
//!
//! `store.sql` (docs/SQL.md §2): when the app declares it, the folder form
//! also writes the SQLite Worker (`sql-worker.js`, `sql.js`) and the
//! official SQLite Wasm build (`vendor/sqlite-wasm/`), and the element gets
//! `sql="sql-worker.js"`. A single-file export cannot start a Worker or load
//! the Wasm from a file next to it, so there a `store.sql` call answers
//! `denied:unsupported` (`plinth build` warns).

use anyhow::{Context as _, Result};
use base64::Engine as _;
use std::path::{Path, PathBuf};

/// The web host as one ES module (`web/plinth.js`, made by `scripts/gen-plinth-js.mjs`).
pub fn plinth_js() -> &'static str {
    let bytes = plinth_registry::web_files::get("plinth.js").expect("web/plinth.js is embedded");
    std::str::from_utf8(bytes).expect("web/plinth.js is UTF-8")
}

/// What the page shows: the app name (the title) and the files.
pub struct WebApp<'a> {
    pub name: &'a str,
    /// The package bytes and its file name (for example `todo.plnt`).
    pub plnt: (&'a [u8], &'a str),
    /// The core bytes and its version (`MAJOR.MINOR`).
    pub core: (&'a [u8], &'a str),
    /// The manifest declares `store.sql`.
    pub sql: bool,
}

/// The web files of `store.sql` that the folder form writes when the app
/// declares it: the Worker, its engine module and the SQLite Wasm build.
pub const SQL_FILES: [&str; 4] = ["sql-worker.js", "sql.js", "vendor/sqlite-wasm/sqlite3.mjs", "vendor/sqlite-wasm/sqlite3.wasm"];

/// The name of the SQL capability (SPEC.md §11).
pub const SQL_CAPABILITY: &str = "store.sql";

impl WebApp<'_> {
    pub fn core_file(&self) -> String {
        format!("plinth-core-{}.wasm", self.core.1)
    }
}

/// Text for HTML text and attribute values.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const HEAD: &str = "<meta charset=\"utf-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n    \
<style>\n      :root { color-scheme: light dark; }\n      html, body { margin: 0; height: 100%; }\n    </style>";

/// `index.html` of the folder form.
pub fn index_html(app: &WebApp) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n  <head>\n    {HEAD}\n    <title>{title}</title>\n    <script type=\"module\" src=\"plinth.js\"></script>\n  </head>\n  <body>\n    \
<plinth-app src=\"{src}\" core=\"{core}\"{sql} height=\"fill\"></plinth-app>\n  </body>\n</html>\n",
        title = escape(app.name),
        src = escape(app.plnt.1),
        core = escape(&app.core_file()),
        sql = if app.sql { " sql=\"sql-worker.js\"" } else { "" },
    )
}

/// The single-file form: the package and the core as base64 in data
/// elements, then the loader as an inline module.
pub fn single_file_html(app: &WebApp) -> String {
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n  <head>\n    {HEAD}\n    <title>{title}</title>\n  </head>\n  <body>\n    \
<plinth-app src=\"#plinth-package\" core=\"#plinth-core\" height=\"fill\"></plinth-app>\n    \
<script type=\"application/octet-stream\" id=\"plinth-package\" data-file=\"{file}\">{plnt}</script>\n    \
<script type=\"application/octet-stream\" id=\"plinth-core\" data-version=\"{version}\">{core}</script>\n    \
<script type=\"module\">\n{js}</script>\n  </body>\n</html>\n",
        title = escape(app.name),
        file = escape(app.plnt.1),
        plnt = b64(app.plnt.0),
        version = escape(app.core.1),
        core = b64(app.core.0),
        js = plinth_js(),
    )
}

/// Writes the folder form into `dir` and returns the files that it wrote.
pub fn write_folder(dir: &Path, app: &WebApp) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let mut files = vec![
        ("index.html".to_owned(), index_html(app).into_bytes()),
        ("plinth.js".to_owned(), plinth_js().as_bytes().to_vec()),
        (app.core_file(), app.core.0.to_vec()),
        (app.plnt.1.to_owned(), app.plnt.0.to_vec()),
    ];
    if app.sql {
        for name in SQL_FILES {
            let bytes = plinth_registry::web_files::get(name).with_context(|| format!("web/{name} is not embedded"))?;
            files.push((name.to_owned(), bytes.to_vec()));
        }
    }
    let mut out = Vec::new();
    for (name, bytes) in files {
        let path = dir.join(&name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        out.push(path);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> WebApp<'static> {
        WebApp { name: "Tom & <Jerry>", plnt: (b"PK\x03\x04", "tom.plnt"), core: (b"\0asm", "1.10"), sql: false }
    }

    /// A new empty directory under the temp directory.
    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("plinth-web-export-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The files under `dir`, as sorted paths with `/`.
    fn written(dir: &Path) -> Vec<String> {
        let mut names = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    names.push(e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        names.sort();
        names
    }

    #[test]
    fn folder_without_sql_has_no_sqlite_files() {
        let dir = temp_dir("kv");
        let out = write_folder(&dir, &app()).unwrap();
        assert_eq!(out.len(), 4);
        assert_eq!(written(&dir), ["index.html", "plinth-core-1.10.wasm", "plinth.js", "tom.plnt"]);
        assert!(!index_html(&app()).contains("sql="));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn folder_with_sql_writes_the_worker_and_the_wasm() {
        let dir = temp_dir("sql");
        let sql_app = WebApp { sql: true, ..app() };
        let out = write_folder(&dir, &sql_app).unwrap();
        assert_eq!(out.len(), 8);
        let expected = [
            "index.html",
            "plinth-core-1.10.wasm",
            "plinth.js",
            "sql-worker.js",
            "sql.js",
            "tom.plnt",
            "vendor/sqlite-wasm/sqlite3.mjs",
            "vendor/sqlite-wasm/sqlite3.wasm",
        ];
        assert_eq!(written(&dir), expected);
        for name in SQL_FILES {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(bytes, plinth_registry::web_files::get(name).unwrap(), "{name}");
        }
        // The Worker imports its two modules by these paths.
        let worker = std::fs::read_to_string(dir.join("sql-worker.js")).unwrap();
        assert!(worker.contains("\"./vendor/sqlite-wasm/sqlite3.mjs\"") && worker.contains("\"./sql.js\""), "{worker}");
        assert!(std::fs::read(dir.join("vendor/sqlite-wasm/sqlite3.wasm")).unwrap().starts_with(b"\0asm"));
        let html = std::fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(html.contains("<plinth-app src=\"tom.plnt\" core=\"plinth-core-1.10.wasm\" sql=\"sql-worker.js\" height=\"fill\">"), "{html}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn single_file_has_no_sql_worker() {
        let html = single_file_html(&WebApp { sql: true, ..app() });
        assert!(html.contains("<plinth-app src=\"#plinth-package\" core=\"#plinth-core\" height=\"fill\">"), "a single file cannot start the SQL Worker");
    }

    #[test]
    fn index_page_loads_the_element() {
        let html = index_html(&app());
        assert!(html.contains("<title>Tom &amp; &lt;Jerry&gt;</title>"), "{html}");
        assert!(html.contains("<script type=\"module\" src=\"plinth.js\"></script>"));
        assert!(html.contains("<plinth-app src=\"tom.plnt\" core=\"plinth-core-1.10.wasm\" height=\"fill\">"));
    }

    #[test]
    fn single_file_has_everything_inline() {
        let html = single_file_html(&app());
        assert!(html.contains("id=\"plinth-package\" data-file=\"tom.plnt\">UEsDBA==</script>"));
        assert!(html.contains("id=\"plinth-core\" data-version=\"1.10\">AGFzbQ==</script>"));
        assert!(html.contains("customElements"), "the loader is inline");
        // The inline loader must not end its script element early.
        let start = html.find("<script type=\"module\">").unwrap() + "<script type=\"module\">".len();
        let end = html.rfind("</script>").unwrap();
        assert!(!html[start..end].to_ascii_lowercase().contains("</script"));
        assert!(!html.contains("src=\"plinth.js\""), "no file next to it");
    }

    #[test]
    fn plinth_js_is_the_bundle() {
        assert!(plinth_js().starts_with("// GENERATED by scripts/gen-plinth-js.mjs"));
        assert!(plinth_js().contains("function plinthBundle(mode, baseUrl)"));
    }
}
