//! A small static HTTP server for a registry folder (`docs/REGISTRY.md`
//! §8: `plinth registry serve`). Used by the CLI, and directly by this
//! crate's end-to-end tests (`tests/e2e.rs`).
//!
//! With `Options::web` it is also the web App Hub (`docs/web-hub.md`): it
//! serves the embedded browser files (`web_files`) under `/web/` on the
//! same origin as the registry, and `/` redirects to `/web/hub.html`.

use anyhow::{Context as _, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

/// Binds `127.0.0.1:<port>` (`0` picks a free port).
pub fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("bind 127.0.0.1:{port}"))
}

/// Serves `folder` on `listener`, one thread per connection. Blocks
/// forever; the caller runs it on its own thread to serve in the
/// background.
pub fn accept_loop(listener: TcpListener, folder: &Path) -> Result<()> {
    accept_loop_with(listener, folder, Options::default())
}

/// What the server serves in addition to the registry folder.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Also serve the web App Hub and the web host under `/web/`, and
    /// redirect `/` to `/web/hub.html` (`docs/web-hub.md`).
    pub web: bool,
    /// The publisher key ids (`ed25519:…`) that the web App Hub trusts as
    /// Hub keys (`docs/HUB.md` §4.1): the page gives `hub.manage` only to a
    /// Hub package that one of them signed. Served as `/web/hub-config.json`.
    pub trusted_keys: Vec<String>,
}

impl Options {
    /// The web App Hub's configuration (`/web/hub-config.json`).
    pub fn hub_config_json(&self) -> String {
        serde_json::json!({ "schema": "plinth.web-hub/1", "hub": "dev.plinth.hub", "trustedKeys": self.trusted_keys }).to_string()
    }
}

/// `accept_loop` with `options`.
pub fn accept_loop_with(listener: TcpListener, folder: &Path, options: Options) -> Result<()> {
    let folder = folder.canonicalize().with_context(|| format!("open {}", folder.display()))?;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let folder = folder.clone();
        let options = options.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, &folder, &options);
        });
    }
    Ok(())
}

/// Binds and serves in a background thread; returns the port actually
/// bound (useful with `port: 0`) right away.
pub fn serve_background(folder: &Path, port: u16) -> Result<u16> {
    serve_background_with(folder, port, Options::default())
}

/// `serve_background` with `options`.
pub fn serve_background_with(folder: &Path, port: u16, options: Options) -> Result<u16> {
    let listener = bind(port)?;
    let bound = listener.local_addr()?.port();
    let folder = folder.to_path_buf();
    std::thread::spawn(move || {
        let _ = accept_loop_with(listener, &folder, options);
    });
    Ok(bound)
}

fn handle(mut stream: TcpStream, folder: &Path, options: &Options) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // "GET /path HTTP/1.1"
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    // Drain the rest of the request headers (ignored).
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
    }
    let head = method == "HEAD";
    if method != "GET" && !head {
        return respond(&mut stream, 405, "text/plain", b"method not allowed", head, &[]);
    }
    let raw_path = path.split(['?', '#']).next().unwrap_or("/");
    if options.web && (raw_path == "/" || raw_path == "/web" || raw_path == "/web/") {
        return respond(&mut stream, 302, "text/plain", b"see /web/hub.html", head, &[("Location", "/web/hub.html")]);
    }
    let rel = percent_decode(raw_path);
    let rel = rel.trim_start_matches('/');
    if options.web
        && let Some(name) = rel.strip_prefix("web/")
    {
        // The app frame (`app-frame.html`) runs in a sandboxed iframe with an
        // opaque origin (`docs/web-hub.md` §4). A module script of such a
        // page is a CORS request with `Origin: null`, so the web files allow
        // any origin (they are public and need no credentials). The frame
        // page itself is also sandboxed by its header, so it has an opaque
        // origin even if a browser opens it directly.
        const CORS: (&str, &str) = ("Access-Control-Allow-Origin", "*");
        const FRAME_CSP: (&str, &str) = ("Content-Security-Policy", "sandbox allow-scripts");
        let extra: &[(&str, &str)] = if name == "app-frame.html" { &[CORS, FRAME_CSP] } else { &[CORS] };
        if name == "hub-config.json" {
            return respond(&mut stream, 200, "application/json", options.hub_config_json().as_bytes(), head, &[]);
        }
        return match crate::web_files::get(name) {
            Some(bytes) => respond(&mut stream, 200, content_type(Path::new(name)), bytes, head, extra),
            None => respond(&mut stream, 404, "text/plain", b"not found", head, &[]),
        };
    }
    if rel.is_empty() {
        return respond(&mut stream, 404, "text/plain", b"not found", head, &[]);
    }
    // Only plain relative names: no `..`, no root, no drive prefix. The
    // canonical path must also stay in the folder (a link can point out).
    let plain = Path::new(rel).components().all(|c| matches!(c, std::path::Component::Normal(_)));
    let full = folder.join(rel);
    let inside = plain && full.canonicalize().map(|c| c.starts_with(folder)).unwrap_or(true);
    if !inside {
        return respond(&mut stream, 403, "text/plain", b"forbidden", head, &[]);
    }
    match std::fs::read(&full) {
        Ok(bytes) => respond(&mut stream, 200, content_type(&full), &bytes, head, &[]),
        Err(_) => respond(&mut stream, 404, "text/plain", b"not found", head, &[]),
    }
}

fn respond(stream: &mut TcpStream, code: u16, content_type: &str, body: &[u8], head_only: bool, extra: &[(&str, &str)]) -> Result<()> {
    let reason = match code {
        200 => "OK",
        302 => "Found",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    // `no-cache`: the client checks again on each use, so a rebuilt app or
    // registry shows up at once (a package file never changes, but the
    // indexes and the web files do).
    let mut header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra {
        header.push_str(&format!("{name}: {value}\r\n"));
    }
    header.push_str("\r\n");
    stream.write_all(header.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

/// The `Content-Type` for a file name (`application/wasm` is needed for
/// `WebAssembly.instantiateStreaming`; ES modules need a JavaScript type).
pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("plnt") => "application/octet-stream",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
