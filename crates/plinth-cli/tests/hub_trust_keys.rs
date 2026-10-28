//! `plinth hub policy trust-hub-key|untrust-hub-key|show` and
//! `plinth hub source trust-hub` (`docs/HUB.md` §4.1): the real `plinth`
//! binary, with `PLINTH_HUB_DIR` in a temp directory and a local test
//! registry server that serves `hub.json`. A second server needs the
//! Cloudflare Access headers, as in `crates/plinth-registry/tests/source_headers.rs`.
//! The headers of that source are literal values in the temp sources file, so
//! the system keychain is not used.

use plinth_hub::{HeaderSpec, Hub};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const INDEX: &str = r#"{"schema":"plinth.registry/1","name":"Test","resources":{"apps":"apps.json","app":"apps/{id}.json","package":"p/{sha256}.plnt"},"generated":"2026-10-28T00:00:00Z"}"#;
const ID: &str = "id-canary-4711";
const SECRET: &str = "secret-canary-9876";

fn key(seed: u8) -> String {
    plinth_package::publisher::key_id(&ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).verifying_key())
}

/// A server on 127.0.0.1. `hub_json` is the body of `/hub.json` (a 404 if
/// `None`); `need_token` makes every path without the Access headers a redirect
/// to a Cloudflare Access login page.
fn serve(hub_json: Option<String>, need_token: bool) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let hub_json = hub_json.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let mut headers = HashMap::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some((n, v)) = h.split_once(':') {
                        headers.insert(n.trim().to_ascii_lowercase(), v.trim().to_owned());
                    }
                }
                let ok = !need_token
                    || (headers.get("cf-access-client-id").is_some_and(|v| v == ID) && headers.get("cf-access-client-secret").is_some_and(|v| v == SECRET));
                let (status, extra, body) = if !ok {
                    (302, "Location: https://team.cloudflareaccess.com/cdn-cgi/access/login/registry\r\n", String::new())
                } else {
                    match (path.as_str(), &hub_json) {
                        ("/plinth-registry.json", _) => (200, "", INDEX.to_owned()),
                        ("/hub.json", Some(body)) => (200, "", body.clone()),
                        _ => (404, "", "not found".to_owned()),
                    }
                };
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            });
        }
    });
    port
}

fn hub_json(keys: &[String]) -> String {
    serde_json::json!({ "schema": "plinth.web-hub/1", "hub": "dev.plinth.hub", "trustedKeys": keys }).to_string()
}

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("plinth-cli-hubkeys-{name}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Env { dir }
    }

    fn plinth(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_plinth"))
            .env("PLINTH_HUB_DIR", &self.dir)
            .env_remove("PLINTH_HUB_TRUSTED_KEYS")
            .args(args)
            .output()
            .unwrap()
    }

    fn hub(&self) -> Hub {
        Hub::open(&self.dir).unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn trust_show_and_untrust_a_hub_key() {
    let env = Env::new("basic");
    let (a, b) = (key(1), key(2));
    assert!(out(&env.plinth(&["hub", "policy", "show"])).contains("no Hub key is trusted"));

    let o = env.plinth(&["hub", "policy", "trust-hub-key", &a]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("trusted") && out(&o).contains(&a));
    assert!(env.plinth(&["hub", "policy", "trust-hub-key", &a]).status.success());
    assert!(out(&env.plinth(&["hub", "policy", "trust-hub-key", &a])).contains("already"));
    assert!(env.plinth(&["hub", "policy", "trust-hub-key", &b]).status.success());
    assert_eq!(env.hub().saved_hub_keys().unwrap(), vec![a.clone(), b.clone()]);

    let show = out(&env.plinth(&["hub", "policy", "show"]));
    assert!(show.contains(&format!("{a}  trusted Hub key (saved)")) && show.contains(&b), "{show}");
    assert!(!show.contains("no Hub key"), "{show}");

    // The environment variable adds to the saved list.
    let o = Command::new(env!("CARGO_BIN_EXE_plinth"))
        .env("PLINTH_HUB_DIR", &env.dir)
        .env("PLINTH_HUB_TRUSTED_KEYS", format!("{}, {a}", key(3)))
        .args(["hub", "policy", "show"])
        .output()
        .unwrap();
    let show = out(&o);
    assert!(show.contains(&format!("{}  trusted Hub key (PLINTH_HUB_TRUSTED_KEYS)", key(3))), "{show}");
    assert_eq!(show.matches(&a).count(), 1, "no key twice: {show}");

    let o = env.plinth(&["hub", "policy", "untrust-hub-key", &a]);
    assert!(o.status.success() && out(&o).contains("not a trusted Hub key"), "{}", out(&o));
    assert!(out(&env.plinth(&["hub", "policy", "untrust-hub-key", &a])).contains("was not a saved"));
    assert_eq!(env.hub().saved_hub_keys().unwrap(), vec![b]);
}

#[test]
fn a_bad_key_id_is_refused_and_nothing_is_saved() {
    let env = Env::new("bad");
    for bad in ["ed25519:nope", "rsa:abc", "abc"] {
        let o = env.plinth(&["hub", "policy", "trust-hub-key", bad]);
        assert!(!o.status.success(), "{bad}");
        assert!(err(&o).contains(bad), "{}", err(&o));
    }
    assert!(!env.plinth(&["hub", "policy", "trust-hub-key"]).status.success());
    assert!(env.hub().saved_hub_keys().unwrap().is_empty());
}

#[test]
fn trust_hub_shows_the_keys_and_saves_them_only_with_yes() {
    let env = Env::new("trusthub");
    let keys = vec![key(4), key(5)];
    let port = serve(Some(hub_json(&keys)), false);
    let base = format!("http://127.0.0.1:{port}");
    assert!(env.plinth(&["hub", "source", "add", "main", &base]).status.success());

    let o = env.plinth(&["hub", "source", "trust-hub", "main"]);
    assert!(!o.status.success(), "without --yes it exits non-zero");
    let text = out(&o);
    assert!(text.contains(&base) && text.contains(&keys[0]) && text.contains(&keys[1]), "{text}");
    assert!(err(&o).contains("--yes"), "{}", err(&o));
    assert!(env.hub().saved_hub_keys().unwrap().is_empty(), "nothing is saved without --yes");

    let o = env.plinth(&["hub", "source", "trust-hub", "main", "--yes"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(env.hub().saved_hub_keys().unwrap(), keys);
    // Again: the same keys, no double.
    assert!(env.plinth(&["hub", "source", "trust-hub", "main", "--yes"]).status.success());
    assert_eq!(env.hub().saved_hub_keys().unwrap().len(), 2);
}

#[test]
fn trust_hub_refuses_a_source_with_no_hub_json_a_bad_key_or_an_unknown_name() {
    let env = Env::new("trusthub-bad");
    let none = serve(None, false);
    let bad = serve(Some(hub_json(&[key(6), "ed25519:nope".to_owned()])), false);
    let empty = serve(Some(hub_json(&[])), false);
    for (name, port) in [("none", none), ("bad", bad), ("empty", empty)] {
        assert!(env.plinth(&["hub", "source", "add", name, &format!("http://127.0.0.1:{port}")]).status.success());
        let o = env.plinth(&["hub", "source", "trust-hub", name, "--yes"]);
        assert!(!o.status.success(), "{name}");
    }
    assert!(!env.plinth(&["hub", "source", "trust-hub", "missing", "--yes"]).status.success());
    assert!(err(&env.plinth(&["hub", "source", "trust-hub", "none", "--yes"])).contains("hub.json"));
    assert!(env.hub().saved_hub_keys().unwrap().is_empty(), "a bad file saves no key, not even the good one");
}

#[test]
fn trust_hub_sends_the_access_headers_of_the_source() {
    let env = Env::new("access");
    let keys = vec![key(7)];
    let port = serve(Some(hub_json(&keys)), true);
    let base = format!("http://127.0.0.1:{port}");
    assert!(env.plinth(&["hub", "source", "add", "corp", &base]).status.success());

    // No token: the Access error, and nothing is saved.
    let o = env.plinth(&["hub", "source", "trust-hub", "corp", "--yes"]);
    assert!(!o.status.success());
    assert!(err(&o).contains("Cloudflare Access"), "{}", err(&o));
    assert!(env.hub().saved_hub_keys().unwrap().is_empty());

    // With the headers (literal values in the temp sources file).
    let specs = vec![
        HeaderSpec { name: "CF-Access-Client-Id".into(), value: Some(ID.into()), secret: None },
        HeaderSpec { name: "CF-Access-Client-Secret".into(), value: Some(SECRET.into()), secret: None },
    ];
    env.hub().source_set_headers("corp", specs).unwrap();
    let o = env.plinth(&["hub", "source", "trust-hub", "corp", "--yes"]);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(env.hub().saved_hub_keys().unwrap(), keys);
    assert!(!out(&o).contains(SECRET) && !err(&o).contains(SECRET));
}
