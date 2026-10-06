//! A small static HTTP server for a registry folder (`docs/REGISTRY.md`
//! §8: `plinth registry serve`). Used by the CLI, and directly by this
//! crate's end-to-end tests (`tests/e2e.rs`).

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
    let folder = folder.canonicalize().with_context(|| format!("open {}", folder.display()))?;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let folder = folder.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, &folder);
        });
    }
    Ok(())
}

/// Binds and serves in a background thread; returns the port actually
/// bound (useful with `port: 0`) right away.
pub fn serve_background(folder: &Path, port: u16) -> Result<u16> {
    let listener = bind(port)?;
    let bound = listener.local_addr()?.port();
    let folder = folder.to_path_buf();
    std::thread::spawn(move || {
        let _ = accept_loop(listener, &folder);
    });
    Ok(bound)
}

fn handle(mut stream: TcpStream, folder: &Path) -> Result<()> {
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
    if method != "GET" && method != "HEAD" {
        return respond(&mut stream, 405, "text/plain", b"method not allowed", method == "HEAD");
    }
    let rel = percent_decode(path.split('?').next().unwrap_or("/"));
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return respond(&mut stream, 404, "text/plain", b"not found", method == "HEAD");
    }
    let full = folder.join(rel);
    if !full.starts_with(folder) {
        return respond(&mut stream, 403, "text/plain", b"forbidden", method == "HEAD");
    }
    match std::fs::read(&full) {
        Ok(bytes) => respond(&mut stream, 200, content_type(&full), &bytes, method == "HEAD"),
        Err(_) => respond(&mut stream, 404, "text/plain", b"not found", method == "HEAD"),
    }
}

fn respond(stream: &mut TcpStream, code: u16, content_type: &str, body: &[u8], head_only: bool) -> Result<()> {
    let reason = match code {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(header.as_bytes())?;
    if !head_only {
        stream.write_all(body)?;
    }
    Ok(())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("plnt") => "application/octet-stream",
        Some("png") => "image/png",
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
