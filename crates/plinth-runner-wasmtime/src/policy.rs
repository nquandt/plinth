//! The capability policy (SPEC.md §11).
//!
//! Every host API call passes through `Policy::check`. A denied call
//! returns `host-error.denied(reason)` and never traps (SPEC.md §8.5).
//!
//! Capability names follow SPEC.md §11, for example `store.kv`,
//! `clipboard.read`, `clipboard.write`. `time` needs no capability: it is
//! always allowed.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};

/// Why a call was denied (mirrors `plinth:app/error.denied-reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeniedReason {
    /// The manifest does not declare this capability.
    Undeclared,
    /// The capability is declared, but the user has not granted it.
    Refused,
    /// This host build has no implementation for the capability.
    Unsupported,
}

/// The policy for one running app: which capabilities it declared, and
/// which of those the user granted.
///
/// This milestone implements grants as "allowed when declared" (SPEC.md
/// §11 stretch: the consent UI is not built yet; see the final report).
#[derive(Debug, Clone, Default)]
pub struct Policy {
    declared: HashSet<String>,
    /// Capabilities the host build does not implement, regardless of the
    /// manifest (SPEC.md §9.4: "A missing implementation returns
    /// `error.unsupported`").
    unsupported: HashSet<String>,
    /// Capabilities the user explicitly refused. Empty until the consent
    /// UI exists; a declared, non-refused capability is granted.
    refused: HashSet<String>,
    /// Bypasses the "undeclared" check (used by `Runner::load` and tests
    /// that do not care about capabilities).
    allow_all: bool,
}

impl Policy {
    /// Builds a policy from the manifest's declared capability names.
    pub fn new(declared: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            declared: declared.into_iter().map(Into::into).collect(),
            unsupported: HashSet::new(),
            refused: HashSet::new(),
            allow_all: false,
        }
    }

    /// Marks a capability as unsupported on this host build.
    pub fn mark_unsupported(&mut self, capability: impl Into<String>) {
        self.unsupported.insert(capability.into());
    }

    /// Marks a capability as refused by the user.
    pub fn refuse(&mut self, capability: impl Into<String>) {
        self.refused.insert(capability.into());
    }

    /// Checks whether `capability` is allowed. `Ok(())` means the call may
    /// proceed.
    pub fn check(&self, capability: &str) -> Result<(), DeniedReason> {
        if !self.allow_all && !self.declared.contains(capability) {
            return Err(DeniedReason::Undeclared);
        }
        if self.unsupported.contains(capability) {
            return Err(DeniedReason::Unsupported);
        }
        if self.refused.contains(capability) {
            return Err(DeniedReason::Refused);
        }
        Ok(())
    }

    /// A policy that allows every capability (used by `Runner::load` and
    /// by tests that do not care about capabilities).
    pub fn allow_all() -> Self {
        Self { declared: HashSet::new(), unsupported: HashSet::new(), refused: HashSet::new(), allow_all: true }
    }

    /// True if a `plinth:net` request may connect to a private address
    /// (SPEC.md §11): `net.local` is declared and neither it nor `net` is
    /// refused. `net::blocking_fetch` uses it to filter resolved
    /// addresses.
    pub fn allows_local_network(&self) -> bool {
        self.allow_all
            || (self.declared.contains("net.local") && !self.refused.contains("net.local") && !self.refused.contains("net"))
    }

    /// Checks a `plinth:net` request against `host` (SPEC.md §11): a
    /// private/loopback host needs `net.local` declared (regardless of
    /// any `net:<host>` entry); a public host needs `net:<host>` (an
    /// exact match) or `net:*` declared. `net` itself must not be marked
    /// unsupported or refused.
    pub fn check_net(&self, host: &str) -> Result<(), DeniedReason> {
        if self.allow_all {
            return Ok(());
        }
        if is_private_host(host) {
            if !self.declared.contains("net.local") {
                return Err(DeniedReason::Undeclared);
            }
        } else {
            let specific = format!("net:{host}");
            if !self.declared.contains(&specific) && !self.declared.contains("net:*") {
                return Err(DeniedReason::Undeclared);
            }
        }
        if self.unsupported.contains("net") {
            return Err(DeniedReason::Unsupported);
        }
        if self.refused.contains("net") || self.refused.contains(&format!("net:{host}")) {
            return Err(DeniedReason::Refused);
        }
        Ok(())
    }
}

/// True for a host that names the local network rather than the public
/// internet (SPEC.md §11): `localhost` and `*.localhost`, a `.local`
/// mDNS name, or an IP literal that `is_private_ip` matches. A trailing
/// dot and IPv6 brackets are ignored. This check sees only the host text;
/// a public name that *resolves* to a private address is stopped later,
/// at connect time, by `net::blocking_fetch`'s resolver.
fn is_private_host(host: &str) -> bool {
    let host = if host.starts_with('[') {
        host.trim_start_matches('[').split(']').next().unwrap_or(host)
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or(host) // strip a port
    } else {
        host // a bare IPv6 literal or a plain name
    };
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => is_private_ip(ip),
        Err(_) => false,
    }
}

/// True for every address that is not a public unicast address: the
/// unspecified, loopback, private, shared (CGNAT), link-local (this
/// includes the cloud metadata address `169.254.169.254`), multicast,
/// broadcast and reserved ranges, IPv6 unique-local and site-local
/// addresses, and an IPv4 address embedded in IPv6 (mapped, compatible,
/// NAT64 `64:ff9b::/96`, 6to4 `2002::/16`) that is one of these.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            let s = v6.segments();
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_v4(v4);
            }
            if s[..6] == [0, 0, 0, 0, 0, 0] && !v6.is_unspecified() && !v6.is_loopback() {
                // IPv4-compatible `::a.b.c.d` (deprecated, still routed by some stacks).
                return is_private_v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
            }
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                return is_private_v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
            }
            if s[0] == 0x2002 {
                return is_private_v4(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8));
            }
            v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (s[0] & 0xffc0) == 0xfec0 // site-local fec0::/10 (deprecated)
        }
    }
}

fn is_private_v4(v4: Ipv4Addr) -> bool {
    let [a, b, _, _] = v4.octets();
    a == 0 // "this network" 0.0.0.0/8
        || v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || (a == 100 && (64..=127).contains(&b)) // shared address space 100.64.0.0/10
        || (a == 192 && b == 0 && v4.octets()[2] == 0) // IETF protocol assignments 192.0.0.0/24
        || (a == 198 && (18..=19).contains(&b)) // benchmarking 198.18.0.0/15
        || v4.is_multicast()
        || a >= 240 // reserved 240.0.0.0/4 and broadcast
}

#[cfg(test)]
mod net_tests {
    use super::*;

    #[test]
    fn private_hosts_need_net_local() {
        let policy = Policy::new(["net:example.com"]);
        assert_eq!(policy.check_net("127.0.0.1"), Err(DeniedReason::Undeclared));
        assert_eq!(policy.check_net("localhost"), Err(DeniedReason::Undeclared));
        let policy = Policy::new(["net.local"]);
        assert_eq!(policy.check_net("127.0.0.1"), Ok(()));
        assert_eq!(policy.check_net("192.168.1.5"), Ok(()));
    }

    #[test]
    fn public_hosts_need_the_exact_or_wildcard_entry() {
        let policy = Policy::new(["net:api.example.com"]);
        assert_eq!(policy.check_net("api.example.com"), Ok(()));
        assert_eq!(policy.check_net("other.example.com"), Err(DeniedReason::Undeclared));
        let policy = Policy::new(["net:*"]);
        assert_eq!(policy.check_net("anything.example.com"), Ok(()));
    }

    #[test]
    fn private_host_forms_are_recognized() {
        for h in [
            "localhost", "LOCALHOST", "localhost.", "app.localhost", "printer.local", "127.0.0.1", "127.255.0.1",
            "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0",
            "0.1.2.3", "224.0.0.1", "255.255.255.255", "198.18.0.1", "192.0.0.8", "[::1]", "::1", "[::]", "[fd00::1]",
            "[fe80::1]", "[fec0::1]", "[::ffff:127.0.0.1]", "[::ffff:10.0.0.1]", "[::127.0.0.1]", "[64:ff9b::a9fe:a9fe]",
            "[2002:c0a8:0101::1]", "[ff02::1]", "127.0.0.1:8080", "[::1]:8080",
        ] {
            assert!(is_private_host(h), "{h} should be private");
        }
        for h in [
            "example.com", "8.8.8.8", "1.1.1.1", "172.32.0.1", "100.128.0.1", "[2606:4700::1111]",
            "[::ffff:8.8.8.8]", "[2002:0808:0808::1]", "localhost.example.com", "example.com:443",
        ] {
            assert!(!is_private_host(h), "{h} should be public");
        }
    }

    #[test]
    fn local_network_needs_net_local_and_no_refusal() {
        assert!(!Policy::new(["net:*"]).allows_local_network());
        assert!(Policy::new(["net.local"]).allows_local_network());
        let mut refused = Policy::new(["net.local"]);
        refused.refuse("net.local");
        assert!(!refused.allows_local_network());
        assert!(Policy::allow_all().allows_local_network());
    }

    #[test]
    fn net_local_alone_does_not_grant_a_public_host() {
        let policy = Policy::new(["net.local"]);
        assert_eq!(policy.check_net("example.com"), Err(DeniedReason::Undeclared));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undeclared_is_denied() {
        let policy = Policy::new(["clipboard.write"]);
        assert_eq!(policy.check("store.kv"), Err(DeniedReason::Undeclared));
    }

    #[test]
    fn declared_is_allowed() {
        let policy = Policy::new(["store.kv"]);
        assert_eq!(policy.check("store.kv"), Ok(()));
    }

    #[test]
    fn unsupported_overrides_declared() {
        let mut policy = Policy::new(["clipboard.read"]);
        policy.mark_unsupported("clipboard.read");
        assert_eq!(policy.check("clipboard.read"), Err(DeniedReason::Unsupported));
    }

    #[test]
    fn refused_overrides_declared() {
        let mut policy = Policy::new(["clipboard.read"]);
        policy.refuse("clipboard.read");
        assert_eq!(policy.check("clipboard.read"), Err(DeniedReason::Refused));
    }

    #[test]
    fn allow_all_grants_an_undeclared_capability() {
        let policy = Policy::allow_all();
        assert_eq!(policy.check("store.kv"), Ok(()));
    }
}
