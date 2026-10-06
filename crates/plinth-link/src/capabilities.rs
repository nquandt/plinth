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
];

/// The capability that a runtime function needs, if any.
pub fn for_function(name: &str) -> Option<&'static str> {
    FUNCTION_CAPABILITIES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}
