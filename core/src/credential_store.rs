//! Bounded access to the OS credential store (Secret Service, Keychain, ...).
//!
//! Platform credential calls are synchronous and may wait indefinitely, for
//! example while a desktop unlock prompt is open. Every call is therefore run on
//! one dedicated worker thread and each caller waits at most a deadline.
//!
//! A deadline does not cancel a platform call that has already started. The
//! gate accounts for that explicitly:
//! - a queued request whose caller gave up is skipped and never executed late;
//! - a request that is already running keeps running; its eventual result only
//!   updates [`CredentialStoreStatus`] and is otherwise discarded, so a late
//!   response can never be applied to a newer session;
//! - while a call is stuck, or after the store reported locked/dismissed or a
//!   deadline elapsed, automatic requests fail immediately instead of queueing
//!   or opening more unlock prompts. Only an explicit [`CredentialAccess::Interactive`]
//!   request (user-initiated retry) may wait on the store again.
//!
//! Failures never fall back to storing secrets anywhere else.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Default deadline for automatic (background/startup) credential access.
pub const AUTOMATIC_TIMEOUT: Duration = Duration::from_secs(5);
/// Default deadline for an explicit, user-initiated retry that may show a prompt.
pub const INTERACTIVE_TIMEOUT: Duration = Duration::from_secs(120);

/// Observable state of the OS credential store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CredentialStoreStatus {
    /// No request has completed yet.
    Unknown,
    /// The store answered the most recent request.
    Available,
    /// The store reported that it is locked.
    Locked,
    /// The user dismissed the unlock prompt.
    Dismissed,
    /// A request did not complete before its deadline; it may still be pending.
    TimedOut,
    /// The store is not reachable on this system.
    Unavailable { reason: String },
}

impl CredentialStoreStatus {
    /// Whether automatic requests are suppressed until an explicit retry.
    fn suppresses_automatic_access(&self) -> bool {
        matches!(self, Self::Locked | Self::Dismissed | Self::TimedOut)
    }

    /// User-facing, actionable description.
    pub fn message(&self) -> &'static str {
        match self {
            Self::Unknown | Self::Available => "Secure credential storage is available.",
            Self::Locked => {
                "Your system keyring is locked. Unlock it, then choose Retry. Saved accounts are kept."
            }
            Self::Dismissed => {
                "The keyring unlock prompt was dismissed. Choose Retry to unlock it. Saved accounts are kept."
            }
            Self::TimedOut => {
                "Your system keyring did not respond. Answer any unlock prompt, then choose Retry. Saved accounts are kept."
            }
            Self::Unavailable { .. } => {
                "Secure credential storage is unavailable on this system. Saved credentials cannot be used until it is available."
            }
        }
    }
}

/// Whether a request may wait on (and prompt from) a store that is not ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialAccess {
    /// Startup, polling or other background work: fail fast when not ready.
    Automatic,
    /// Explicit user action (Retry): may wait for a prompt to be answered.
    Interactive,
}

/// Error used when the gate itself refuses or abandons a request.
#[derive(Debug)]
struct GateError(String);

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GateError {}

fn gate_error(message: impl Into<String>) -> keyring::Error {
    keyring::Error::NoStorageAccess(Box::new(GateError(message.into())))
}

/// Classify a platform result into the store status it implies.
pub fn classify(result: &Result<(), &keyring::Error>) -> CredentialStoreStatus {
    match result {
        Ok(()) => CredentialStoreStatus::Available,
        Err(error) => match error {
            // Entry-level outcomes prove the store itself answered.
            keyring::Error::NoEntry
            | keyring::Error::BadEncoding(_)
            | keyring::Error::TooLong(..)
            | keyring::Error::Invalid(..)
            | keyring::Error::Ambiguous(_) => CredentialStoreStatus::Available,
            other => {
                let text = other.to_string().to_ascii_lowercase();
                if text.contains("dismiss") || text.contains("cancel") {
                    CredentialStoreStatus::Dismissed
                } else if text.contains("locked") || text.contains("unlock") {
                    CredentialStoreStatus::Locked
                } else {
                    CredentialStoreStatus::Unavailable {
                        reason: other.to_string(),
                    }
                }
            }
        },
    }
}

struct GateState {
    status: CredentialStoreStatus,
    busy_since: Option<Instant>,
}

type Job = Box<dyn FnOnce() + Send>;

/// Serialized, deadline-bounded executor for credential-store calls.
pub struct CredentialGate {
    jobs: mpsc::Sender<Job>,
    state: Arc<Mutex<GateState>>,
    stuck_after: Duration,
}

impl CredentialGate {
    /// Start a gate with its own worker thread. `stuck_after` is how long a
    /// running call may block before automatic callers stop queueing behind it.
    pub fn new(stuck_after: Duration) -> Self {
        let (jobs, receiver) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("credential-store".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    job();
                }
            });
        if let Err(error) = spawned {
            tracing::error!("credential store worker could not start: {error}");
        }
        Self {
            jobs,
            state: Arc::new(Mutex::new(GateState {
                status: CredentialStoreStatus::Unknown,
                busy_since: None,
            })),
            stuck_after,
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Current store status.
    pub fn status(&self) -> CredentialStoreStatus {
        self.lock_state().status.clone()
    }

    /// Run `op` on the worker, waiting at most `timeout`.
    ///
    /// Gate refusals and deadlines are reported as
    /// `keyring::Error::NoStorageAccess`, which existing callers already treat
    /// as "credential store not available".
    pub fn run<R: Send + 'static>(
        &self,
        access: CredentialAccess,
        timeout: Duration,
        op: impl FnOnce() -> keyring::Result<R> + Send + 'static,
    ) -> keyring::Result<R> {
        {
            let state = self.lock_state();
            if access == CredentialAccess::Automatic {
                if state.status.suppresses_automatic_access() {
                    return Err(gate_error(state.status.message()));
                }
                if state
                    .busy_since
                    .is_some_and(|since| since.elapsed() >= self.stuck_after)
                {
                    return Err(gate_error(CredentialStoreStatus::TimedOut.message()));
                }
            }
        }

        let (reply, response) = mpsc::sync_channel::<keyring::Result<R>>(1);
        let abandoned = Arc::new(AtomicBool::new(false));
        let job_abandoned = Arc::clone(&abandoned);
        let job_state = Arc::clone(&self.state);
        let job: Job = Box::new(move || {
            // A caller that already gave up must not have its request run late.
            if job_abandoned.load(Ordering::SeqCst) {
                return;
            }
            {
                let mut state = job_state.lock().unwrap_or_else(|p| p.into_inner());
                state.busy_since = Some(Instant::now());
            }
            let result = op();
            {
                let mut state = job_state.lock().unwrap_or_else(|p| p.into_inner());
                state.busy_since = None;
                state.status = classify(&result.as_ref().map(|_| ()));
            }
            // Ignored when the caller timed out: late results are discarded.
            let _ = reply.send(result);
        });
        if self.jobs.send(job).is_err() {
            return Err(gate_error(
                "Secure credential storage worker is not running.",
            ));
        }

        match response.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                abandoned.store(true, Ordering::SeqCst);
                let mut state = self.lock_state();
                // Only escalate if no newer request already proved the store ready.
                if state.busy_since.is_some() || state.status != CredentialStoreStatus::Available {
                    state.status = CredentialStoreStatus::TimedOut;
                }
                Err(gate_error(
                    "Secure credential storage did not respond in time. A requested change may or may not have been applied; answer any unlock prompt, then choose Retry.",
                ))
            }
        }
    }
}

fn global() -> &'static CredentialGate {
    static GATE: OnceLock<CredentialGate> = OnceLock::new();
    GATE.get_or_init(|| CredentialGate::new(AUTOMATIC_TIMEOUT))
}

/// Run an automatic (non-prompting, fail-fast) credential-store operation.
pub fn run<R: Send + 'static>(
    op: impl FnOnce() -> keyring::Result<R> + Send + 'static,
) -> keyring::Result<R> {
    global().run(CredentialAccess::Automatic, AUTOMATIC_TIMEOUT, op)
}

/// Async form of [`run`]: the bounded wait happens on a blocking thread, never
/// on an async runtime worker.
pub async fn run_async<R: Send + 'static>(
    op: impl FnOnce() -> keyring::Result<R> + Send + 'static,
) -> keyring::Result<R> {
    match tokio::task::spawn_blocking(move || run(op)).await {
        Ok(result) => result,
        Err(error) => Err(gate_error(format!(
            "Secure credential storage task failed: {error}"
        ))),
    }
}

/// Current status of the process-wide credential store.
pub fn status() -> CredentialStoreStatus {
    global().status()
}

/// Explicit user retry: probe the store, allowing an unlock prompt to be answered.
pub fn retry_interactive(
    probe: impl FnOnce() -> keyring::Result<()> + Send + 'static,
) -> CredentialStoreStatus {
    let _ = global().run(CredentialAccess::Interactive, INTERACTIVE_TIMEOUT, probe);
    status()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const SHORT: Duration = Duration::from_millis(60);

    fn platform(message: &str) -> keyring::Error {
        keyring::Error::NoStorageAccess(Box::new(GateError(message.into())))
    }

    #[test]
    fn successful_and_missing_entries_report_available() {
        let gate = CredentialGate::new(SHORT);
        assert_eq!(
            gate.run(CredentialAccess::Automatic, SHORT, || Ok(7)).ok(),
            Some(7)
        );
        assert_eq!(gate.status(), CredentialStoreStatus::Available);
        let missing: keyring::Result<()> = gate.run(CredentialAccess::Automatic, SHORT, || {
            Err(keyring::Error::NoEntry)
        });
        assert!(matches!(missing, Err(keyring::Error::NoEntry)));
        assert_eq!(gate.status(), CredentialStoreStatus::Available);
    }

    #[test]
    fn hung_call_is_bounded_and_later_calls_fail_fast_without_running() {
        let gate = CredentialGate::new(SHORT);
        let started = Instant::now();
        let hung = gate.run(CredentialAccess::Automatic, SHORT, || {
            std::thread::sleep(Duration::from_millis(400));
            Ok(())
        });
        assert!(hung.is_err());
        assert!(started.elapsed() < Duration::from_millis(300));
        assert_eq!(gate.status(), CredentialStoreStatus::TimedOut);

        let ran = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&ran);
        let started = Instant::now();
        let refused = gate.run(CredentialAccess::Automatic, SHORT, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(refused.is_err());
        assert!(started.elapsed() < Duration::from_millis(30));
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(ran.load(Ordering::SeqCst), 0, "refused request never runs");
    }

    #[test]
    fn abandoned_queued_request_is_never_executed_late() {
        let gate = CredentialGate::new(Duration::from_secs(10));
        let _ = gate.run(CredentialAccess::Interactive, SHORT, || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        });
        let ran = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&ran);
        let queued = gate.run(CredentialAccess::Interactive, SHORT, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(queued.is_err());
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            ran.load(Ordering::SeqCst),
            0,
            "abandoned write must not apply late"
        );
    }

    #[test]
    fn late_completion_only_updates_status() {
        let gate = CredentialGate::new(SHORT);
        let late: keyring::Result<&'static str> =
            gate.run(CredentialAccess::Automatic, SHORT, || {
                std::thread::sleep(Duration::from_millis(200));
                Ok("late secret")
            });
        assert!(late.is_err(), "caller never receives the late value");
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(gate.status(), CredentialStoreStatus::Available);
        assert!(gate
            .run(CredentialAccess::Automatic, SHORT, || Ok(()))
            .is_ok());
    }

    #[test]
    fn dismissed_and_locked_suppress_until_interactive_retry() {
        for (message, expected) in [
            (
                "Secret Service: unlock prompt was dismissed",
                CredentialStoreStatus::Dismissed,
            ),
            ("collection is locked", CredentialStoreStatus::Locked),
        ] {
            let gate = CredentialGate::new(SHORT);
            let owned = message.to_string();
            let first: keyring::Result<()> =
                gate.run(CredentialAccess::Automatic, SHORT, move || {
                    Err(platform(&owned))
                });
            assert!(first.is_err());
            assert_eq!(gate.status(), expected);

            let ran = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&ran);
            let automatic = gate.run(CredentialAccess::Automatic, SHORT, move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
            assert!(automatic.is_err());
            assert_eq!(ran.load(Ordering::SeqCst), 0, "no prompt storm");

            assert!(gate
                .run(CredentialAccess::Interactive, SHORT, || Ok(()))
                .is_ok());
            assert_eq!(gate.status(), CredentialStoreStatus::Available);
            assert!(gate
                .run(CredentialAccess::Automatic, SHORT, || Ok(()))
                .is_ok());
        }
    }

    #[test]
    fn delayed_unlock_within_interactive_deadline_succeeds() {
        let gate = CredentialGate::new(SHORT);
        let unlocked = gate.run(
            CredentialAccess::Interactive,
            Duration::from_secs(2),
            || {
                std::thread::sleep(Duration::from_millis(150));
                Ok("value")
            },
        );
        assert_eq!(unlocked.ok(), Some("value"));
        assert_eq!(gate.status(), CredentialStoreStatus::Available);
    }

    #[test]
    fn unavailable_store_is_reported_without_suppression() {
        let gate = CredentialGate::new(SHORT);
        let result: keyring::Result<()> = gate.run(CredentialAccess::Automatic, SHORT, || {
            Err(platform(
                "org.freedesktop.secrets was not provided by any .service files",
            ))
        });
        assert!(result.is_err());
        assert!(matches!(
            gate.status(),
            CredentialStoreStatus::Unavailable { .. }
        ));
        assert!(gate
            .run(CredentialAccess::Automatic, SHORT, || Ok(()))
            .is_ok());
    }

    #[test]
    fn deadline_error_reports_uncertain_outcome() {
        let gate = CredentialGate::new(SHORT);
        let error = gate
            .run(CredentialAccess::Automatic, SHORT, || {
                std::thread::sleep(Duration::from_millis(200));
                Ok(())
            })
            .expect_err("deadline");
        assert!(error
            .to_string()
            .contains("may or may not have been applied"));
    }
}
