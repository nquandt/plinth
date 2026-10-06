//! `plinth registry build|serve` (`docs/REGISTRY.md` §8): the static
//! registry generator and a small local HTTP server for the folder it
//! builds.

use anyhow::{Context as _, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

pub fn build(folder: &Path, with_core: bool) -> Result<()> {
    let report = plinth_registry::build::build(folder, &plinth_registry::build::Options { with_core })?;
    println!("built the registry at {}", folder.display());
    for (id, version) in &report.added {
        println!("  added {id}@{version}");
    }
    for (id, version) in &report.unchanged {
        println!("  unchanged {id}@{version}");
    }
    Ok(())
}

/// Serves `folder` as a static registry on `127.0.0.1:<port>`. Blocks
/// forever (`Ctrl+C` to stop).
pub fn serve(folder: &Path, port: u16) -> Result<()> {
    let folder = folder.canonicalize().with_context(|| format!("open {}", folder.display()))?;
    let listener = TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("bind 127.0.0.1:{port}"))?;
    let addr = listener.local_addr()?;
    println!("serving {} on http://{addr}", folder.display());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let folder = folder.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, &folder);
        });
    }
    Ok(())
}

fn handle(mut stream: TcpStream, folder: &Path) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    // "GET /path HTTP/1.1"
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    // Drain the rest of the request headers (we ignore them).
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
        return respond(&mut stream, 404, "text/plain", b"not found", false);
    }
    let full = folder.join(rel);
    if !full.starts_with(folder) {
        return respond(&mut stream, 403, "text/plain", b"forbidden", false);
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
