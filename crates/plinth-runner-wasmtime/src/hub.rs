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

    // -- Core 1.8 (`docs/HUB.md` §9.1, §5.2, phase H3 step 2). Each has a
    // default, so a test double that does not need them stays small.

    /// The library's groups, as a JSON array of strings.
    fn list_groups_json(&self) -> Result<String, String> {
        Ok("[]".to_owned())
    }
    fn create_group(&mut self, name: &str) -> Result<(), String> {
        let _ = name;
        Err("unsupported".to_owned())
    }
    /// Puts `id` in `group` (`member`) or takes it out.
    fn set_group(&mut self, id: &str, group: &str, member: bool) -> Result<(), String> {
        let _ = (id, group, member);
        Err("unsupported".to_owned())
    }
    /// Removes `id` from the library (its data and grants stay).
    fn remove(&mut self, id: &str) -> Result<(), String> {
        let _ = id;
        Err("unsupported".to_owned())
    }
    /// A job that searches the configured sources for `query`. The runner
    /// runs it on a worker thread; it must not touch the guest. `Ok` is the
    /// JSON text that `wit/plinth/app.wit`'s `hub.search` describes.
    fn search(&self, query: &str) -> HubJob {
        let _ = query;
        Box::new(|| Err("unsupported".to_owned()))
    }
    /// A job that installs the latest version of `id` from the first
    /// source that lists it. The runner runs it on a worker thread. `Ok` is
    /// the installed app id.
    fn install(&self, id: &str) -> HubJob {
        let _ = id;
        Box::new(|| Err("unsupported".to_owned()))
    }
}

/// Work that a `plinth:hub` call does off the UI thread (network, disk):
/// `search` and `install`. The runner delivers its result to the guest as
/// a `completion` event (SPEC.md §8.4).
pub type HubJob = Box<dyn FnOnce() -> Result<String, String> + Send>;
