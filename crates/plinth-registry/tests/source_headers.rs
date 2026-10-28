//! Request headers of a registry source (`docs/HUB.md` §5.4): a local test
//! server answers with a redirect to a Cloudflare Access login page unless
//! both headers of a service token are present. The tests check the headers
//! with and without, a redirect to another host, and that no secret is in
//! the sources file or in an error text.

use plinth_hub::{HeaderSpec, Hub, SecretsFactory, SourceClient};
use plinth_registry::HubSources;
use plinth_registry::source::{RequestHeaders, Source};
use plinth_runner_wasmtime::secrets::{MemorySecrets, SecretsBackend, SecretsScope};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const ID: &str = "id-canary-4711";
const SECRET: &str = "secret-canary-9876";

const INDEX: &str = r#"{"schema":"plinth.registry/1","name":"Test","resources":{"apps":"apps.json","app":"apps/{id}.json","package":"p/{sha256}.plnt"},"generated":"2026-10-27T00:00:00Z"}"#;
const APPS: &str = r#"{"schema":"plinth.registry/1","apps":[{"id":"com.example.notes","name":"Notes","publisher":"me","latest":"0.1.0","updated":"2026-10-27T00:00:00Z"}]}"#;

type Reply = (u16, Vec<(String, String)>, String);

/// A request as the test server saw it: the path and the lowercase headers.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: HashMap<String, String>,
}

/// Starts a server on 127.0.0.1. `handler` gets the request and gives
/// `(status, extra headers, body)`. Returns the port and the requests seen.
fn serve(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> (u16, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let (log2, handler) = (log.clone(), Arc::new(handler));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (log, handler) = (log2.clone(), handler.clone());
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
                let seen = Seen { path, headers };
                log.lock().unwrap().push(seen.clone());
                let (status, extra, body) = handler(&seen);
                let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                for (n, v) in extra {
                    head.push_str(&format!("{n}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            });
        }
    });
    (port, log)
}

fn registry_body(path: &str) -> Reply {
    match path {
        "/plinth-registry.json" => (200, vec![], INDEX.to_owned()),
        "/apps.json" => (200, vec![], APPS.to_owned()),
        _ => (404, vec![], "not found".to_owned()),
    }
}

/// The Access rule: both headers of the token, else a redirect to the login page.
fn access_server() -> (u16, Arc<Mutex<Vec<Seen>>>) {
    serve(|req| {
        let ok = req.headers.get("cf-access-client-id").is_some_and(|v| v == ID) && req.headers.get("cf-access-client-secret").is_some_and(|v| v == SECRET);
        if ok {
            registry_body(&req.path)
        } else {
            (302, vec![("Location".into(), "https://team.cloudflareaccess.com/cdn-cgi/access/login/registry?kid=abc".into())], String::new())
        }
    })
}

fn token() -> RequestHeaders {
    RequestHeaders::new(vec![("CF-Access-Client-Id".into(), ID.into()), ("CF-Access-Client-Secret".into(), SECRET.into())]).unwrap()
}

fn temp_hub(tag: &str) -> Hub {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    Hub::open(std::env::temp_dir().join(format!("plinth-src-headers-{tag}-{}-{n}", std::process::id()))).unwrap()
}

#[test]
fn without_the_headers_the_error_names_cloudflare_access() {
    let (port, log) = access_server();
    let base = format!("http://127.0.0.1:{port}");
    let e = Source::open(&base).err().expect("must fail");
    let text = format!("{e:#}");
    assert!(text.contains("the registry needs a login or a token (Cloudflare Access)"), "{text}");
    // The client did not follow the redirect to the login page.
    assert_eq!(log.lock().unwrap().len(), 1);
}

#[test]
fn with_the_headers_the_source_opens_and_reads() {
    let (port, log) = access_server();
    let base = format!("http://127.0.0.1:{port}");
    let source = Source::open_with(&base, token()).unwrap();
    let apps = source.apps().unwrap();
    assert_eq!(apps[0].id, "com.example.notes");
    // Every request had the headers (index and apps list).
    assert!(log.lock().unwrap().len() >= 2);
    assert!(log.lock().unwrap().iter().all(|r| r.headers.get("cf-access-client-id").map(String::as_str) == Some(ID)));
}

#[test]
fn a_wrong_token_error_has_no_secret() {
    let (port, _) = access_server();
    let base = format!("http://127.0.0.1:{port}");
    let wrong = RequestHeaders::new(vec![("CF-Access-Client-Id".into(), ID.into()), ("CF-Access-Client-Secret".into(), "wrong-canary".into())]).unwrap();
    let e = Source::open_with(&base, wrong).err().expect("must fail");
    let text = format!("{e:#} {e:?}");
    for canary in [ID, SECRET, "wrong-canary"] {
        assert!(!text.contains(canary), "an error text holds a header value: {text}");
    }
    assert!(!format!("{:?}", token()).contains("canary"), "Debug of the headers shows a value");
}

#[test]
fn a_redirect_to_another_host_does_not_get_the_headers() {
    // Server B has another host name (`localhost`, not `127.0.0.1`) and it
    // serves the registry for any client. It records what it gets.
    let (port_b, log_b) = serve(|req| registry_body(&req.path));
    let (port_a, log_a) = serve(move |req| (302, vec![("Location".into(), format!("http://localhost:{port_b}{}", req.path))], String::new()));
    let base = format!("http://127.0.0.1:{port_a}");
    let source = Source::open_with(&base, token()).unwrap();
    assert_eq!(source.apps().unwrap().len(), 1);
    // A got the headers on its own request; B got none.
    assert!(log_a.lock().unwrap().iter().all(|r| r.headers.contains_key("cf-access-client-id")));
    let b = log_b.lock().unwrap();
    assert!(!b.is_empty());
    assert!(b.iter().all(|r| !r.headers.contains_key("cf-access-client-id") && !r.headers.contains_key("cf-access-client-secret")), "{b:?}");
}

#[test]
fn a_redirect_to_the_same_host_keeps_the_headers() {
    let (port, log) = serve(|req| {
        let ok = req.headers.contains_key("cf-access-client-id");
        match (req.path.as_str(), ok) {
            ("/plinth-registry.json", _) => (302, vec![("Location".into(), "/real/plinth-registry.json".into())], String::new()),
            ("/real/plinth-registry.json", true) => (200, vec![], INDEX.to_owned()),
            ("/apps.json", true) => (200, vec![], APPS.to_owned()),
            _ => (403, vec![], String::new()),
        }
    });
    let source = Source::open_with(&format!("http://127.0.0.1:{port}"), token()).unwrap();
    assert_eq!(source.apps().unwrap().len(), 1);
    assert!(log.lock().unwrap().iter().filter(|r| r.path.starts_with("/real")).all(|r| r.headers.contains_key("cf-access-client-id")));
}

#[test]
fn hub_sources_read_the_token_from_the_store_at_request_time() {
    let (port, _) = access_server();
    let base = format!("http://127.0.0.1:{port}");
    let hub = temp_hub("hubsrc");
    hub.source_add("corp", &base).unwrap();
    let mut store = MemorySecrets::new();
    let scope = SecretsScope::new(plinth_hub::HOST_SECRETS_APP);
    let mut specs = Vec::new();
    for (name, value) in [("CF-Access-Client-Id", ID), ("CF-Access-Client-Secret", SECRET)] {
        let secret = plinth_hub::header_secret_name("corp", name).unwrap();
        store.set(&scope, &secret, value).unwrap();
        specs.push(HeaderSpec { name: name.into(), value: None, secret: Some(secret) });
    }
    hub.source_set_headers("corp", specs).unwrap();

    // The sources file holds the secret names, not the values.
    let file = std::fs::read_to_string(hub.sources_path()).unwrap();
    assert!(!file.contains("canary"), "{file}");
    assert!(file.contains("source.corp.cf-access-client-id"), "{file}");

    let secrets: SecretsFactory = {
        let store = store.clone();
        Arc::new(move || Box::new(store.clone()) as Box<dyn SecretsBackend>)
    };
    let client = HubSources::with_secrets(hub.clone(), secrets);
    let hits = client.search(&base, "notes").unwrap();
    assert_eq!(hits[0].id, "com.example.notes");

    // No headers (a client with no store): the Access error.
    let e = HubSources::new().search(&base, "notes").unwrap_err();
    assert!(format!("{e:#}").contains("Cloudflare Access"));

    // A source whose secret is gone: an error that names no value.
    store.remove(&scope, "source.corp.cf-access-client-secret").unwrap();
    let e = client.search(&base, "notes").unwrap_err();
    let text = format!("{e:#}");
    assert!(text.contains("set-token corp"), "{text}");
    assert!(!text.contains("canary"), "{text}");
}
