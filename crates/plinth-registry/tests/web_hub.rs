//! The web App Hub server (`plinth registry serve --web`,
//! `docs/web-hub.md`): one origin serves the registry and the embedded web
//! files, `/` redirects to the landing page, and every file has the right
//! `Content-Type` and `Cache-Control: no-cache`.

use plinth_registry::build::{Options, build};
use plinth_registry::serve::{self, Options as ServeOptions};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("plinth-web-hub-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn compile_counter() -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let fs = plinth_compiler::driver::DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("counter has errors:\n{}", diags.join("\n"))
    });
    let cfg = plinth_package::ProjectConfig::parse("id = \"com.example.counter\"\nname = \"Counter\"\nversion = \"0.1.0\"\npublisher = \"me\"\n").unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &artifact.app);
    plinth_package::Package { manifest, component: artifact.app, assets: Vec::new(), signature: None }.write().unwrap()
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A raw HTTP/1.1 GET, so the path reaches the server exactly as written
/// (an HTTP library can normalize `..`).
fn get(port: u16, path: &str) -> Response {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("no header end");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned())).collect();
    Response { status, headers, body: raw[split + 4..].to_vec() }
}

#[test]
fn web_hub_serves_landing_page_index_package_and_core() {
    let dir = temp_dir("serve");
    let bytes = compile_counter();
    std::fs::write(dir.join("counter.plnt"), &bytes).unwrap();
    build(&dir, &Options { with_core: true }).unwrap();
    let digest = plinth_registry::hex_digest(&bytes);

    let port = serve::serve_background_with(&dir, 0, ServeOptions { web: true }).unwrap();

    let root = get(port, "/");
    assert_eq!(root.status, 302);
    assert_eq!(root.header("Location"), Some("/web/hub.html"));

    let page = get(port, "/web/hub.html");
    assert_eq!(page.status, 200);
    assert_eq!(page.header("Content-Type"), Some("text/html; charset=utf-8"));
    assert_eq!(page.header("Cache-Control"), Some("no-cache"));
    assert!(String::from_utf8_lossy(&page.body).contains("hub.js"));

    for (path, ty) in [
        ("/web/hub.js", "text/javascript; charset=utf-8"),
        ("/web/hub-logic.js", "text/javascript; charset=utf-8"),
        ("/web/plinth-web.js", "text/javascript; charset=utf-8"),
        ("/web/index.html", "text/html; charset=utf-8"),
        ("/web/style.css", "text/css; charset=utf-8"),
    ] {
        let r = get(port, path);
        assert_eq!((r.status, r.header("Content-Type")), (200, Some(ty)), "{path}");
    }
    assert_eq!(get(port, "/web/test/run-a11y.mjs").status, 404, "tests are not embedded");

    let index = get(port, "/plinth-registry.json");
    assert_eq!((index.status, index.header("Content-Type")), (200, Some("application/json")));
    let apps = get(port, "/apps/index.json?x=1");
    assert_eq!((apps.status, apps.header("Content-Type")), (200, Some("application/json")));
    let list: plinth_registry::AppList = serde_json::from_slice(&apps.body).unwrap();
    assert_eq!(list.apps[0].id, "com.example.counter");

    let pkg = get(port, &format!("/packages/{digest}.plnt"));
    assert_eq!((pkg.status, pkg.header("Content-Type")), (200, Some("application/octet-stream")));
    assert_eq!(pkg.body, bytes);
    assert_eq!(pkg.header("Cache-Control"), Some("no-cache"));

    let (major, minor) = plinth_link::cores::core_version(plinth_link::link::runtime()).unwrap();
    let core = get(port, &format!("/cores/{major}.{minor}/core.wasm"));
    assert_eq!((core.status, core.header("Content-Type")), (200, Some("application/wasm")));

    // Nothing outside the folder.
    std::fs::write(dir.parent().unwrap().join("plinth-web-hub-secret.txt"), b"secret").unwrap();
    for path in ["/../plinth-web-hub-secret.txt", "/%2e%2e/plinth-web-hub-secret.txt", "/apps/..%2f..%2fplinth-web-hub-secret.txt"] {
        let r = get(port, path);
        assert!(r.status == 403 || r.status == 404, "{path}: {}", r.status);
        assert_ne!(r.body, b"secret");
    }
    let _ = std::fs::remove_file(dir.parent().unwrap().join("plinth-web-hub-secret.txt"));
}

#[test]
fn the_package_icon_is_copied_and_the_app_list_url_resolves() {
    let dir = temp_dir("icon");
    let pkg = plinth_package::Package::read(&compile_counter()).unwrap();
    let png = b"\x89PNG\r\n\x1a\nnot a real image".to_vec();
    let mut manifest = pkg.manifest.clone();
    manifest.id = "com.example.iconic".into();
    manifest.icon = Some("assets/icon.png".into());
    let bytes = plinth_package::Package { manifest, component: pkg.component, assets: vec![("assets/icon.png".into(), png.clone())], signature: None }
        .write()
        .unwrap();
    std::fs::write(dir.join("iconic.plnt"), &bytes).unwrap();
    build(&dir, &Options::default()).unwrap();

    let list: plinth_registry::AppList = serde_json::from_str(&std::fs::read_to_string(dir.join("apps/index.json")).unwrap()).unwrap();
    // Relative to `apps/index.json` (docs/REGISTRY.md §3).
    assert_eq!(list.apps[0].icon.as_deref(), Some("com.example.iconic/assets/icon.png"));

    let port = serve::serve_background_with(&dir, 0, ServeOptions { web: true }).unwrap();
    let icon = get(port, "/apps/com.example.iconic/assets/icon.png");
    assert_eq!((icon.status, icon.header("Content-Type")), (200, Some("image/png")));
    assert_eq!(icon.body, png);
}

#[test]
fn without_web_the_server_serves_only_the_registry() {
    let dir = temp_dir("plain");
    build(&dir, &Options::default()).unwrap();
    let port = serve::serve_background(&dir, 0).unwrap();
    assert_eq!(get(port, "/").status, 404);
    assert_eq!(get(port, "/web/hub.html").status, 404);
    assert_eq!(get(port, "/plinth-registry.json").status, 200);
}

#[test]
fn every_embedded_web_file_is_a_file_of_web() {
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web");
    for (name, bytes) in plinth_registry::web_files::FILES {
        assert_eq!(std::fs::read(web.join(name)).unwrap(), *bytes, "{name}");
    }
    // Every module that a served page imports is also served.
    for (name, bytes) in plinth_registry::web_files::FILES {
        let text = String::from_utf8_lossy(bytes);
        for import in text.split("from \"./").skip(1).chain(text.split("src=\"").skip(1)) {
            let file = import.split('"').next().unwrap();
            if file.ends_with(".js") && !file.contains('/') {
                assert!(plinth_registry::web_files::get(file).is_some(), "{name} needs {file}, which is not embedded");
            }
        }
    }
}
