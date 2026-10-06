//! The shared capability map (`docs/HUB.md` §12.3, §7.1, §7.2; `SPEC.md`
//! §11).
//!
//! One table maps each runtime function name (as in `rt_abi::FUNCTIONS`) to
//! the capability that it needs. The compiler's `PL1007` check, the runner's
//! `Policy`, and (later) the Hub and the registry all read this same table,
//! so an app cannot reach a capability that it does not declare (`SPEC.md`
//! §11, `docs/HUB.md` §7.1 rule 1).
//!
//! A runtime function that is not in this table needs no capability (for
//! example everything in `plinth:ui` and `plinth:time`).

/// A capability name (`SPEC.md` §11), for example `store.kv`.
pub const STORE_KV: &str = "store.kv";
pub const CLIPBOARD_READ: &str = "clipboard.read";
pub const CLIPBOARD_WRITE: &str = "clipboard.write";
/// `plinth:net` (SPEC.md §11). Capability names are dynamic: a manifest
/// declares `net:<host>` for one host, or `net.local` for private network
/// ranges. The compiler's reachability check (`PL1007`) only knows that
/// `net_fetch` needs *some* capability starting with `net:` or equal to
/// `net.local`; it cannot know the URL's host (often a runtime value), so
/// this constant is a generic marker for that reachability check only.
/// The real per-host decision happens at call time in the runner's
/// `Policy` (`crates/plinth-runner-wasmtime/src/policy.rs`), which checks
/// the actual URL against the manifest's declared `net:`/`net.local`
/// entries (`docs/HUB.md` §7.1).
pub const NET: &str = "net";
/// Private network ranges (SPEC.md §11): a manifest must declare this
/// fixed name, in addition to any `net:<host>` entry, before a request to
/// a private/loopback address is allowed.
pub const NET_LOCAL: &str = "net.local";
/// Privileged Hub management (`docs/HUB.md` §4.1, §12.2): `list-apps`,
/// `launch`, `set-grant`, `block`, `unblock`. The host grants this only to
/// a package signed by a trusted Hub key (`docs/HUB.md` §4.1); it is not
/// something an ordinary app can ever be granted by consent alone.
pub const HUB_MANAGE: &str = "hub.manage";

/// How much a capability can do, for the consent screen's wording and
/// default (`docs/HUB.md` §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    /// UI only, `time`: allowed, no question.
    None,
    /// Allowed at install, shown in the label.
    Low,
    /// Asked one time.
    Medium,
    /// Asked each time, or for a session.
    High,
}

/// One row of the capability map: a capability name, its risk level, and a
/// short plain-language description for the consent screen and the
/// `plinth validate` report.
pub struct CapabilityInfo {
    pub name: &'static str,
    pub risk: Risk,
    /// A short, plain-language description, in the words of `docs/HUB.md`
    /// §7.2 (for example "save data on this device").
    pub description: &'static str,
}

/// The capability map: every capability a runtime function can need, with
/// its risk level and description (`docs/HUB.md` §7.1, §7.2).
pub const CAPABILITIES: &[CapabilityInfo] = &[
    CapabilityInfo { name: STORE_KV, risk: Risk::Low, description: "save data on this device" },
    CapabilityInfo { name: CLIPBOARD_WRITE, risk: Risk::Low, description: "write to the clipboard" },
    CapabilityInfo { name: CLIPBOARD_READ, risk: Risk::Medium, description: "read the clipboard" },
    CapabilityInfo { name: NET_LOCAL, risk: Risk::Medium, description: "connect to devices on your local network" },
    CapabilityInfo {
        name: HUB_MANAGE,
        risk: Risk::High,
        description: "manage the Hub library: list, launch, grant and block apps",
    },
];

/// The capability info for `name`, if it is a known capability.
pub fn info(name: &str) -> Option<&'static CapabilityInfo> {
    CAPABILITIES.iter().find(|c| c.name == name)
}

/// `(runtime function name without the "__plinth_rt_" prefix, capability)`.
/// A function missing from this table needs no capability.
pub const FUNCTION_CAPABILITIES: &[(&str, &str)] = &[
    ("kv_get", STORE_KV),
    ("kv_set", STORE_KV),
    ("kv_delete", STORE_KV),
    ("kv_keys", STORE_KV),
    ("clipboard_write_text", CLIPBOARD_WRITE),
    ("clipboard_read_text", CLIPBOARD_READ),
    // `kv_last_error` and `clipboard_last_error` read a local, harmless
    // status flag; the compiler does not gate them (`stdlib.rs`), so they
    // need no capability here either.
    // `net_fetch` needs a `net:<host>` or `net.local` capability (see
    // `NET` above); this generic marker only drives reachability.
    ("net_fetch", NET),
    ("hub_list_apps", HUB_MANAGE),
    ("hub_launch", HUB_MANAGE),
    ("hub_set_grant", HUB_MANAGE),
    ("hub_block", HUB_MANAGE),
    ("hub_unblock", HUB_MANAGE),
    ("hub_list_groups", HUB_MANAGE),
    ("hub_create_group", HUB_MANAGE),
    ("hub_set_group", HUB_MANAGE),
    ("hub_remove", HUB_MANAGE),
    ("hub_search", HUB_MANAGE),
    ("hub_install", HUB_MANAGE),
    ("hub_app_info", HUB_MANAGE),
    ("hub_pin", HUB_MANAGE),
    ("hub_block_publisher", HUB_MANAGE),
    ("hub_unblock_publisher", HUB_MANAGE),
    ("hub_check_updates", HUB_MANAGE),
    ("hub_update", HUB_MANAGE),
    // `hub_last_error` needs no capability, like `kv_last_error` above.
];

/// True if the declared capability `declared` covers the reachable
/// capability `reachable` (`docs/HUB.md` §7.1): the same name, or any
/// `net:<host>` or `net.local` for the generic `net` marker (the host is a
/// run-time value; the runner checks it at call time).
pub fn covers(declared: &str, reachable: &str) -> bool {
    declared == reachable || (reachable == NET && (declared.starts_with("net:") || declared == NET_LOCAL))
}

/// The capability that a runtime function needs, if any.
pub fn for_function(name: &str) -> Option<&'static str> {
    FUNCTION_CAPABILITIES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_hosts_cover_the_net_marker() {
        assert!(covers("store.kv", "store.kv"));
        assert!(covers("net:api.example.com", NET));
        assert!(covers("net:*", NET));
        assert!(covers(NET_LOCAL, NET));
        assert!(!covers("store.kv", NET));
        assert!(!covers("net:api.example.com", STORE_KV));
    }
}
