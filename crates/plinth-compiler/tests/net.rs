//! `plinth:net`'s `fetch` (SPEC.md §8.4, §8.5, §11): a request id returned
//! at once, the result delivered later through a `completion` event, and
//! the capability/private-network policy enforced at call time. Uses a
//! tiny local HTTP server (`std::net::TcpListener`) so these tests need
//! no internet access.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, Runner, kv::Kv};
use plinth_ui::tree::{Node, Tree};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Starts a tiny HTTP/1.1 server on an ephemeral `127.0.0.1` port. It
/// answers exactly `requests` connections, one at a time, each with a
/// fixed-ish reply based on the request line and body, then exits. The
/// server thread is not joined: the test process exits when done, which
/// closes the socket.
fn spawn_server(requests: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..requests {
            let Ok((stream, _)) = listener.accept() else { break };
            handle_one(stream);
        }
    });
    port
}

fn handle_one(mut stream: TcpStream) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let (method, path, content_length) = loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        let text = String::from_utf8_lossy(&buf);
        let Some(header_end) = text.find("\r\n\r\n") else { continue };
        let head = &text[..header_end];
        let mut lines = head.lines();
        let first = lines.next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or("GET").to_owned();
        let path = parts.next().unwrap_or("/").to_owned();
        let cl: usize = lines
            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_owned()))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let body_so_far = buf.len().saturating_sub(header_end + 4);
        if body_so_far >= cl {
            break (method, path, cl);
        }
    };
    // Finish reading the body if the first read did not get all of it.
    let header_end = String::from_utf8_lossy(&buf).find("\r\n\r\n").unwrap();
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    let body = String::from_utf8_lossy(&body).into_owned();

    let (status, text) = match (method.as_str(), path.as_str()) {
        ("GET", "/hello") => (200, "hello world".to_owned()),
        ("POST", "/echo") => (200, format!("echo:{body}")),
        (_, "/missing") => (404, "not found".to_owned()),
        _ => (400, "bad request".to_owned()),
    };
    let resp = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
    let _ = stream.write_all(resp.as_bytes());
}

fn compile(src: &str, caps: &[&str]) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", src);
    let caps: Vec<String> = caps.iter().map(|s| s.to_string()).collect();
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    })
}

fn start(src: &str, caps: &[&str], gc_stress: bool) -> (Guest, Tree) {
    let artifact = compile(src, caps);
    let runner = Runner::new().unwrap();
    let policy = Policy::new(caps.iter().map(|s| s.to_string()));
    let mut guest = runner
        .load_with_policy(&artifact.component, Limits::default(), policy, Kv::in_memory(), Box::new(plinth_runner_wasmtime::MemoryClipboard::default()))
        .unwrap();
    let args = if gc_stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
    let mut tree = Tree::new();
    for commit in guest.init(&args).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    (guest, tree)
}

fn find(tree: &Tree, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
    let mut stack: Vec<NodeId> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(kind) && pred(node) {
            return id;
        }
        stack.extend(node.children.iter());
    }
    panic!("no matching {kind:?} found");
}

fn press(guest: &mut Guest, tree: &mut Tree, button: NodeId) {
    let handler = tree.get(button).and_then(|n| n.handler(event::PRESS)).expect("button has a handler");
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    for commit in guest.on_event(w.as_bytes()).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
}

/// Waits (polling `poll_net_results`) for the one request `press` just
/// opened to finish, then delivers its completion. Fails the test after a
/// few seconds rather than hanging forever if a bug drops the result.
fn wait_and_answer(guest: &mut Guest, tree: &mut Tree) {
    let start = Instant::now();
    loop {
        let results = guest.poll_net_results();
        if let Some((id, result)) = results.into_iter().next() {
            for commit in guest.answer_dialog(id, result).unwrap() {
                let errors = tree.apply(&commit).unwrap();
                assert!(errors.is_empty(), "op errors: {errors:?}");
            }
            return;
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("net.fetch result never arrived");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn text(tree: &Tree) -> String {
    let id = find(tree, ControlKind::Text, |_| true);
    tree.get(id).unwrap().text.clone().unwrap()
}

fn app_src(call: &str) -> String {
    format!(
        r#"import {{ app, Screen, Text, Button, signal }} from "plinth:ui";
import {{ fetch }} from "plinth:net";
const result = signal("waiting");
function Home() {{
  return <Screen title="Home">
    <Text>{{result()}}</Text>
    <Button label="go" onPress={{() => {call}}} />
  </Screen>;
}}
export default app({{ screens: {{ home: {{ title: "Home", component: Home }} }} }});
"#
    )
}

#[test]
fn get_round_trip() {
    let port = spawn_server(1);
    let url = format!("http://127.0.0.1:{port}/hello");
    let src = app_src(&format!(
        r#"fetch("{url}", null, (r) => {{ result.set(r.ok + ":" + r.status + ":" + r.text); }})"#
    ));
    let (mut guest, mut tree) = start(&src, &["net.local"], false);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    wait_and_answer(&mut guest, &mut tree);
    assert_eq!(text(&tree), "true:200:hello world");
}

#[test]
fn post_round_trip() {
    let port = spawn_server(1);
    let url = format!("http://127.0.0.1:{port}/echo");
    let src = app_src(&format!(
        r#"fetch("{url}", {{ method: "POST", body: "hi" }}, (r) => {{ result.set(r.ok + ":" + r.text); }})"#
    ));
    let (mut guest, mut tree) = start(&src, &["net.local"], false);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    wait_and_answer(&mut guest, &mut tree);
    assert_eq!(text(&tree), "true:echo:hi");
}

#[test]
fn a_non_200_status_is_not_ok_but_has_no_error() {
    let port = spawn_server(1);
    let url = format!("http://127.0.0.1:{port}/missing");
    let src = app_src(&format!(r#"fetch("{url}", null, (r) => {{ result.set(r.ok + ":" + r.status + ":" + (r.error ?? "null")); }})"#));
    let (mut guest, mut tree) = start(&src, &["net.local"], false);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    wait_and_answer(&mut guest, &mut tree);
    assert_eq!(text(&tree), "false:404:null");
}

#[test]
fn an_undeclared_host_is_denied_without_any_network_access() {
    // A public (non-private) address, and the manifest declares only
    // `net.local`: denied before any connection is attempted.
    let src = app_src(r#"fetch("http://198.51.100.1/x", null, (r) => { result.set(r.ok + ":" + (r.error ?? "null")); })"#);
    let (mut guest, mut tree) = start(&src, &["net.local"], false);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    wait_and_answer(&mut guest, &mut tree);
    assert_eq!(text(&tree), "false:denied:undeclared");
}

#[test]
fn a_private_address_without_net_local_is_denied() {
    let port = spawn_server(0);
    let url = format!("http://127.0.0.1:{port}/hello");
    // Declares a net host, but not `net.local`: a loopback address is
    // still denied (SPEC.md §11).
    let src = app_src(&format!(r#"fetch("{url}", null, (r) => {{ result.set(r.ok + ":" + (r.error ?? "null")); }})"#));
    let (mut guest, mut tree) = start(&src, &["net:example.com"], false);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    wait_and_answer(&mut guest, &mut tree);
    assert_eq!(text(&tree), "false:denied:undeclared");
}

#[test]
fn the_done_callback_survives_a_gc_between_the_call_and_its_answer() {
    let port = spawn_server(1);
    let url = format!("http://127.0.0.1:{port}/hello");
    let src = app_src(&format!(r#"fetch("{url}", null, (r) => {{ result.set(r.text); }})"#));
    let (mut guest, mut tree) = start(&src, &["net.local"], true);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    // `press` already ran a collection (stress mode runs one after every
    // on-event): the pending request's wrapper closure must have
    // survived it (SPEC.md §5.4) for the completion below to still work.
    wait_and_answer(&mut guest, &mut tree);
    assert_eq!(text(&tree), "hello world");
}
