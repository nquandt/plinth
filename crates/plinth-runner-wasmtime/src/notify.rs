//! `plinth:notify` (core 1.12, `docs/PRODUCTION.md` H3): system
//! notifications.
//!
//! The runner checks the capability (`notify`, Medium risk) with the
//! `Policy`, then gives the call to a `NotifyBackend`. A host with a
//! notification area (for example a desktop host) supplies its own backend
//! with `Guest::set_notify`. Until then, the guest has a `MemoryNotify`,
//! which only records the calls (for tests).
//!
//! `show` completes through the same queue as `net.fetch` and `plinth:files`,
//! with the same 4-element list: `[ok, 0, text, error]`. `text` is `"shown"`
//! or `"unsupported"`; a denied call has `ok` false and `error` =
//! `"denied:<reason>"`. A denied call never traps.

use crate::policy::{DeniedReason, Policy};
use plinth_protocol::Value;
use std::sync::{Arc, Mutex};

pub use plinth_link::capabilities::NOTIFY;

/// What a backend did with a `show` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyOutcome {
    /// The host showed the notification.
    Shown,
    /// This host has no notification area. Not an error: the app can show
    /// the message in its own window instead.
    Unsupported,
}

/// A host's notification area. A desktop or mobile host implements this on
/// the system notification API; tests use `MemoryNotify`.
pub trait NotifyBackend: Send {
    /// Shows a notification. A `tag` replaces an earlier notification with
    /// the same tag.
    fn show(&mut self, title: &str, body: Option<&str>, tag: Option<&str>) -> NotifyOutcome;
    /// Removes the notification with `tag`. An unknown tag is not an error.
    fn cancel(&mut self, tag: &str);
}

/// One call that a `MemoryNotify` recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyCall {
    Show { title: String, body: Option<String>, tag: Option<String> },
    Cancel { tag: String },
}

/// A notification backend that only records the calls. A test keeps a
/// clone of `calls()` to read them after it gives the backend to the guest.
#[derive(Clone)]
pub struct MemoryNotify {
    calls: Arc<Mutex<Vec<NotifyCall>>>,
    outcome: NotifyOutcome,
}

impl Default for MemoryNotify {
    fn default() -> Self {
        Self::new(NotifyOutcome::Shown)
    }
}

impl MemoryNotify {
    /// A backend that answers each `show` with `outcome`.
    pub fn new(outcome: NotifyOutcome) -> Self {
        Self { calls: Arc::new(Mutex::new(Vec::new())), outcome }
    }

    /// The list of recorded calls, shared with this backend.
    pub fn calls(&self) -> Arc<Mutex<Vec<NotifyCall>>> {
        self.calls.clone()
    }
}

impl NotifyBackend for MemoryNotify {
    fn show(&mut self, title: &str, body: Option<&str>, tag: Option<&str>) -> NotifyOutcome {
        self.calls.lock().unwrap().push(NotifyCall::Show {
            title: title.to_owned(),
            body: body.map(str::to_owned),
            tag: tag.map(str::to_owned),
        });
        self.outcome
    }

    fn cancel(&mut self, tag: &str) {
        self.calls.lock().unwrap().push(NotifyCall::Cancel { tag: tag.to_owned() });
    }
}

/// The `"denied:<reason>"` text of a denied call.
fn denied_text(reason: DeniedReason) -> &'static str {
    match reason {
        DeniedReason::Undeclared => "denied:undeclared",
        DeniedReason::Refused => "denied:refused",
        DeniedReason::Unsupported => "denied:unsupported",
    }
}

/// Runs a `show` call: checks the policy, then calls the backend. Returns
/// the completion result, `[ok, 0, text, error]`. The backend is not called
/// when the policy denies the call.
pub fn show(policy: &Policy, backend: &mut dyn NotifyBackend, title: &str, body: Option<&str>, tag: Option<&str>) -> Value {
    match policy.check(NOTIFY) {
        Err(reason) => {
            Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(denied_text(reason).to_owned())])
        }
        Ok(()) => {
            let text = match backend.show(title, body, tag) {
                NotifyOutcome::Shown => "shown",
                NotifyOutcome::Unsupported => "unsupported",
            };
            Value::List(vec![Value::Bool(true), Value::Int(0), Value::Str(text.to_owned()), Value::Null])
        }
    }
}

/// Runs a `cancel` call: checks the policy, then calls the backend.
pub fn cancel(policy: &Policy, backend: &mut dyn NotifyBackend, tag: &str) -> Result<(), DeniedReason> {
    policy.check(NOTIFY)?;
    backend.cancel(tag);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(ok: bool, text: &str, error: Option<&str>) -> Value {
        Value::List(vec![
            Value::Bool(ok),
            Value::Int(0),
            Value::Str(text.to_owned()),
            error.map_or(Value::Null, |e| Value::Str(e.to_owned())),
        ])
    }

    #[test]
    fn allowed_show_and_cancel_reach_the_backend() {
        let policy = Policy::new([NOTIFY]);
        let mut backend = MemoryNotify::default();
        let calls = backend.calls();
        assert_eq!(show(&policy, &mut backend, "Done", Some("3 files"), Some("job")), result(true, "shown", None));
        assert_eq!(show(&policy, &mut backend, "Hi", None, None), result(true, "shown", None));
        assert_eq!(cancel(&policy, &mut backend, "job"), Ok(()));
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                NotifyCall::Show { title: "Done".into(), body: Some("3 files".into()), tag: Some("job".into()) },
                NotifyCall::Show { title: "Hi".into(), body: None, tag: None },
                NotifyCall::Cancel { tag: "job".into() },
            ]
        );
    }

    #[test]
    fn a_host_with_no_notification_area_answers_unsupported() {
        let policy = Policy::new([NOTIFY]);
        let mut backend = MemoryNotify::new(NotifyOutcome::Unsupported);
        assert_eq!(show(&policy, &mut backend, "Hi", None, None), result(true, "unsupported", None));
    }

    #[test]
    fn undeclared_calls_are_denied_and_do_not_reach_the_backend() {
        let policy = Policy::new(["store.kv"]);
        let mut backend = MemoryNotify::default();
        let calls = backend.calls();
        assert_eq!(show(&policy, &mut backend, "Hi", None, None), result(false, "", Some("denied:undeclared")));
        assert_eq!(cancel(&policy, &mut backend, "job"), Err(DeniedReason::Undeclared));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn refused_and_unsupported_calls_are_denied() {
        let mut policy = Policy::new([NOTIFY]);
        policy.refuse(NOTIFY);
        let mut backend = MemoryNotify::default();
        let calls = backend.calls();
        assert_eq!(show(&policy, &mut backend, "Hi", None, None), result(false, "", Some("denied:refused")));
        assert_eq!(cancel(&policy, &mut backend, "job"), Err(DeniedReason::Refused));

        let mut policy = Policy::new([NOTIFY]);
        policy.mark_unsupported(NOTIFY);
        assert_eq!(show(&policy, &mut backend, "Hi", None, None), result(false, "", Some("denied:unsupported")));
        assert_eq!(cancel(&policy, &mut backend, "job"), Err(DeniedReason::Unsupported));
        assert!(calls.lock().unwrap().is_empty());
    }
}
