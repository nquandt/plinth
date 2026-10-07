//! `plinth:net`'s host side (SPEC.md §8.5, §11): parses a URL's host for
//! the capability check, runs the actual HTTP request on a worker thread
//! with a blocking client, and builds the 4-field completion result list
//! `plinth-rt`'s `net_result_*` reads (SPEC.md §8.4).

use crate::policy::{is_private_ip, DeniedReason, Policy};
use plinth_protocol::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// How long one request may run before it completes with a network error.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The maximum response body this host reads (SPEC.md §8.5): larger
/// bodies complete with a network error rather than growing without
/// bound.
pub const MAX_BODY: u64 = 8 << 20;

/// The host part of a URL (SPEC.md §11), without scheme, port, path or
/// userinfo. `None` if `url` has no recognizable host (the request then
/// fails before any capability check, as a network error).
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map(|(_, r)| r)?;
    let rest = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
    if rest.starts_with('[') {
        // An IPv6 literal, `[::1]:port`: keep the brackets so `Policy`'s
        // heuristic recognizes `[::1]` directly.
        return Some(rest.split(']').next().map(|h| &rest[..h.len() + 1]).unwrap_or(rest));
    }
    Some(rest.split(':').next().unwrap_or(rest))
}

/// Checks `url`'s host against `policy` (SPEC.md §11).
pub fn check_url(policy: &Policy, url: &str) -> Result<(), DeniedReason> {
    match host_of(url) {
        Some(host) => policy.check_net(host),
        None => Err(DeniedReason::Unsupported),
    }
}

/// The completion result for a denied request.
pub fn denied(reason: DeniedReason) -> Value {
    let text = match reason {
        DeniedReason::Undeclared => "denied:undeclared",
        DeniedReason::Refused => "denied:refused",
        DeniedReason::Unsupported => "denied:unsupported",
    };
    Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(text.to_owned())])
}

/// A resolver that drops private addresses (SPEC.md §11) after the DNS
/// lookup, so a public name that resolves to a local address (a DNS
/// rebinding attack, or `2130706433` for `127.0.0.1`) cannot reach the
/// local network without `net.local`. ureq connects only to the addresses
/// that this resolver returns, so the check and the connection use the
/// same address. The host of the user's proxy is never filtered.
#[derive(Debug)]
struct PublicOnlyResolver {
    allow_private: bool,
    proxy_host: Option<String>,
    blocked: Arc<AtomicBool>,
}

impl Resolver for PublicOnlyResolver {
    fn resolve(&self, uri: &ureq::http::Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = DefaultResolver::default().resolve(uri, config, timeout)?;
        let is_proxy = self.proxy_host.is_some() && uri.host() == self.proxy_host.as_deref();
        if self.allow_private || is_proxy {
            return Ok(all);
        }
        let mut kept = self.empty();
        for addr in public_only(&all) {
            kept.push(addr);
        }
        if kept.is_empty() {
            self.blocked.store(true, Ordering::SeqCst);
            return Err(ureq::Error::HostNotFound);
        }
        Ok(kept)
    }
}

/// The public addresses of `addrs`, in their order (`is_private_ip`).
fn public_only(addrs: &[std::net::SocketAddr]) -> Vec<std::net::SocketAddr> {
    addrs.iter().copied().filter(|a| !is_private_ip(a.ip())).collect()
}

/// Runs one blocking HTTP request (SPEC.md §8.5: "a blocking HTTP client
/// on a worker thread"). Never panics: any failure (bad URL, connection
/// error, a body over `MAX_BODY`, non-UTF-8 text) becomes a `"network:
/// ..."` result, not a trap. Without `allow_private`
/// (`Policy::allows_local_network`), a host that resolves only to private
/// addresses completes as `denied:undeclared`, the same as a private IP
/// literal.
pub fn blocking_fetch(url: &str, method: &str, headers: &[(String, String)], body: Option<&str>, allow_private: bool) -> Value {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // Redirects are allowed only to allowed hosts (SPEC.md §11); this
        // runner does not re-check the policy on each hop yet, so it
        // disables redirects outright rather than follow one blindly.
        .max_redirects(0)
        // `fetch` reports a non-2xx status through `response.status`, not
        // as a transport error (SPEC.md §8.5).
        .http_status_as_error(false)
        .build();
    let proxy_host = config.proxy().and_then(|p| p.uri().host()).map(str::to_owned);
    let blocked = Arc::new(AtomicBool::new(false));
    let resolver = PublicOnlyResolver { allow_private, proxy_host, blocked: blocked.clone() };
    let agent = ureq::Agent::with_parts(config, DefaultConnector::default(), resolver);
    match run(&agent, url, method, headers, body) {
        Err(_) if blocked.load(Ordering::SeqCst) => denied(DeniedReason::Undeclared),
        Ok((status, text)) => {
            let ok = (200..300).contains(&status);
            Value::List(vec![Value::Bool(ok), Value::Int(status), Value::Str(text), Value::Null])
        }
        Err(e) => Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(format!("network: {e}"))]),
    }
}

fn run(agent: &ureq::Agent, url: &str, method: &str, headers: &[(String, String)], body: Option<&str>) -> Result<(i32, String), String> {
    let resp = match method.to_ascii_uppercase().as_str() {
        "POST" | "PUT" | "PATCH" => {
            let mut req = match method.to_ascii_uppercase().as_str() {
                "POST" => agent.post(url),
                "PUT" => agent.put(url),
                _ => agent.patch(url),
            };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.send(body.unwrap_or("").as_bytes())
        }
        m => {
            let mut req = if m == "DELETE" { agent.delete(url) } else { agent.get(url) };
            for (k, v) in headers {
                req = req.header(k, v);
            }
            req.call()
        }
    };
    let mut resp = resp.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16() as i32;
    let text = resp.body_mut().with_config().limit(MAX_BODY).read_to_string().map_err(|e| e.to_string())?;
    Ok((status, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_parses_scheme_and_port() {
        assert_eq!(host_of("http://127.0.0.1:8080/x"), Some("127.0.0.1"));
        assert_eq!(host_of("https://api.example.com/v1?x=1"), Some("api.example.com"));
        assert_eq!(host_of("https://user:pw@example.com/"), Some("example.com"));
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn public_only_drops_every_private_address() {
        let addrs: Vec<std::net::SocketAddr> =
            ["127.0.0.1:80", "93.184.216.34:80", "[::1]:80", "10.0.0.1:80", "[2606:4700::1111]:443", "169.254.169.254:80"]
                .iter()
                .map(|s| s.parse().unwrap())
                .collect();
        let kept = public_only(&addrs);
        assert_eq!(kept, vec![addrs[1], addrs[4]]);
        assert!(public_only(&addrs[..1]).is_empty());
    }

    #[test]
    fn denied_result_has_ok_false_and_a_reason() {
        let v = denied(DeniedReason::Undeclared);
        assert_eq!(v, Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str("denied:undeclared".into())]));
    }
}
