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
}

impl Policy {
    /// Builds a policy from the manifest's declared capability names.
    pub fn new(declared: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self { declared: declared.into_iter().map(Into::into).collect(), unsupported: HashSet::new(), refused: HashSet::new() }
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
        if !self.declared.contains(capability) {
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

    /// A policy that allows every capability (used by tests and by `time`,
    /// which needs none).
    pub fn allow_all() -> Self {
        Self { declared: HashSet::new(), unsupported: HashSet::new(), refused: HashSet::new() }
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
}
