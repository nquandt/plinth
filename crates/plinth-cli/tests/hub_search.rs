//! `plinth hub search` with one bad source: the other sources still answer,
//! and the command fails only when every source fails.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};

const INDEX: &str = r#"{"schema":"plinth.registry/1","name":"Test","resources":{"apps":"apps.json","app":"apps/{id}.json","package":"p/{sha256}.plnt"},"generated":"2026-10-28T00:00:00Z"}"#;
const APPS: &str = r#"{"schema":"plinth.registry/1","apps":[{"id":"dev.test.counter","name":"Counter","publisher":"T","latest":"1.0.0","updated":"2026-10-28T00:00:00Z"}]}"#;

fn serve() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                }
                let (status, body) = match path.as_str() {
                    "/plinth-registry.json" => (200, INDEX),
                    "/apps.json" => (200, APPS),
                    _ => (404, "not found"),
                };
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            });
        }
    });
    port
}

fn plinth(dir: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_plinth")).env("PLINTH_HUB_DIR", dir).args(args).output().unwrap()
}

#[test]
fn a_bad_source_is_skipped_and_the_good_source_answers() {
    let dir = std::env::temp_dir().join(format!("plinth-cli-search-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = format!("http://127.0.0.1:{}/", serve());
    // Port 9 (discard) refuses the connection.
    assert!(plinth(&dir, &["hub", "source", "add", "bad", "http://127.0.0.1:9/"]).status.success());
    assert!(plinth(&dir, &["hub", "source", "add", "good", &good]).status.success());

    let o = plinth(&dir, &["hub", "search", "counter"]);
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(o.status.success(), "{err}");
    assert!(out.contains("dev.test.counter") && out.contains("[good]"), "{out}");
    assert!(err.contains("source bad skipped"), "{err}");

    // Only the bad source: the command fails.
    assert!(plinth(&dir, &["hub", "source", "remove", "good"]).status.success());
    let o = plinth(&dir, &["hub", "search", "counter"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("every source failed"));
    std::fs::remove_dir_all(&dir).ok();
}
