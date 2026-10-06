//! The `plinth:hub` host backend (`docs/HUB.md` §4.1, §12.2, §12.3).
//!
//! `plinth-hub` implements `HubBackend` on top of the local library (its
//! crate already depends on this one for `Policy`, so the dependency runs
//! one way). A guest reaches these calls only when the host granted
//! `hub.manage`, which it does only for a package signed by a trusted Hub
//! key (`docs/HUB.md` §4.1; the check lives in `plinth-host-desktop`,
//! since it needs the package's signature, not just its bytes).

/// The native side of `plinth:hub`. `plinth-hub`'s `HubService` implements
/// this on the local library; a test double can implement it directly.
pub trait HubBackend: Send {
    /// Every app in the library, as a JSON array of objects (`docs/HUB.md`
    /// §9.1): see `wit/plinth/app.wit`'s `hub.list-apps` doc comment for
    /// the exact shape.
    fn list_apps_json(&self) -> Result<String, String>;
    /// Requests that the host open `id` in a new window (`docs/HUB.md`
    /// §4.2). Returns at once; `take_launches` drains the requests the
    /// host (`plinth-host-desktop`) has not opened yet.
    fn launch(&mut self, id: &str);
    /// Drains the launch requests queued since the last call.
    fn take_launches(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn set_grant(&mut self, id: &str, capability: &str, allowed: bool) -> Result<(), String>;
    fn block(&mut self, id: &str) -> Result<(), String>;
    fn unblock(&mut self, id: &str) -> Result<(), String>;
}
