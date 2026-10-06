//! The capability policy (SPEC.md §11).
//!
//! Every host API call passes through `Policy::check`. A denied call
//! returns `host-error.denied(reason)` and never traps (SPEC.md §8.5).
//!
//! Capability names follow SPEC.md §11, for example `store.kv`,
//! `clipboard.read`, `clipboard.write`. `time` needs no capability: it is
//! always allowed.

use std::collections::HashSet;

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

/// True for `localhost`, a loopback/private IPv4 or IPv6 literal, or a
/// `.local` mDNS name (SPEC.md §11). This is a literal-address heuristic,
/// not a DNS lookup: a hostname that *resolves* to a private address
/// still needs `net:<host>`, not `net.local` (resolving every host up
/// front would add a network round trip to every policy check).
fn is_private_host(host: &str) -> bool {
    let host = host.split(':').next().unwrap_or(host); // strip a port
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".local") {
        return true;
    }
    if host == "::1" || host == "0.0.0.0" || host == "[::1]" {
        return true;
    }
    let octets: Vec<&str> = host.split('.').collect();
    if octets.len() == 4 && octets.iter().all(|o| o.parse::<u8>().is_ok()) {
        let n: Vec<u8> = octets.iter().map(|o| o.parse().unwrap()).collect();
        return n[0] == 127 || n[0] == 10 || (n[0] == 172 && (16..=31).contains(&n[1])) || (n[0] == 192 && n[1] == 168);
    }
    false
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
