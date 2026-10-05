use serde_json::Value;
use std::env;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// Process-wide cap: switching profiles cannot accumulate unbounded detached jobs.
/// There is deliberately no queue; a saturated source retries admission next tick.
const MAX_BACKGROUND_POLLS: usize = 4;

#[derive(Debug)]
struct WorkerBudget {
    active: AtomicUsize,
    limit: usize,
}

impl WorkerBudget {
    fn reserve(self: &Arc<Self>) -> Option<WorkerPermit> {
        self.active
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |active| {
                (active < self.limit).then_some(active + 1)
            })
            .ok()?;
        Some(WorkerPermit(self.clone()))
    }
}

struct WorkerPermit(Arc<WorkerBudget>);
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Construction is inert. Only try_start (called from a live tick) spawns work.
/// Cancellation never joins a worker: an existing bounded request may finish,
/// but its result is discarded and the shared permit remains held until exit.
#[derive(Debug)]
pub struct BackgroundPoll<T> {
    receiver: Option<mpsc::Receiver<Result<T, String>>>,
    cancelled: Arc<AtomicBool>,
    budget: Arc<WorkerBudget>,
}

impl<T> Default for BackgroundPoll<T> {
    fn default() -> Self {
        static BUDGET: OnceLock<Arc<WorkerBudget>> = OnceLock::new();
        Self {
            receiver: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            budget: BUDGET
                .get_or_init(|| {
                    Arc::new(WorkerBudget {
                        active: AtomicUsize::new(0),
                        limit: MAX_BACKGROUND_POLLS,
                    })
                })
                .clone(),
        }
    }
}

impl<T: Send + 'static> BackgroundPoll<T> {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn is_idle(&self) -> bool {
        self.receiver.is_none() && !self.cancelled.load(Ordering::SeqCst)
    }

    pub fn try_start(
        &mut self,
        fetch: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static,
    ) -> bool {
        if !self.is_idle() {
            return false;
        }
        let Some(permit) = self.budget.reserve() else {
            return false;
        };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let cancelled = self.cancelled.clone();
        let failed_spawn = sender.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("signal-http".into())
            .spawn(move || {
                let _permit = permit;
                if cancelled.load(Ordering::SeqCst) {
                    return;
                }
                let result = fetch(&cancelled);
                if !cancelled.load(Ordering::SeqCst) {
                    let _ = sender.send(result);
                }
            })
        {
            let _ = failed_spawn.send(Err(format!("cannot start API worker: {error}")));
        }
        true
    }

    pub fn try_recv(&mut self) -> Option<Result<T, String>> {
        let result = match self.receiver.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("API worker exited without a result".into())
            }
        };
        self.receiver = None;
        Some(result)
    }
}

impl<T> BackgroundPoll<T> {
    pub fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.receiver = None;
    }

    #[cfg(test)]
    pub fn isolated_for_test() -> Self {
        let mut worker = Self::default();
        worker.budget = Arc::new(WorkerBudget {
            active: AtomicUsize::new(0),
            limit: 4,
        });
        worker
    }

    #[cfg(test)]
    pub fn ready_for_test(result: Result<T, String>) -> Self {
        let mut worker = Self::isolated_for_test();
        let (sender, receiver) = mpsc::channel();
        assert!(sender.send(result).is_ok(), "receiver is alive");
        worker.receiver = Some(receiver);
        worker
    }
}

impl<T> Drop for BackgroundPoll<T> {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalStatus {
    Idle,
    Running,
    Positive,
    Negative,
    Alert,
    Stale,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalSnapshot {
    pub status: ExternalStatus,
    pub event_key: String,
    pub message: String,
}

impl ExternalSnapshot {
    pub fn idle(message: impl Into<String>) -> Self {
        Self {
            status: ExternalStatus::Idle,
            event_key: "idle".to_string(),
            message: message.into(),
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            status: ExternalStatus::Error,
            event_key: format!("error:{message}"),
            message,
        }
    }

    pub fn stale(message: impl Into<String>) -> Self {
        Self {
            status: ExternalStatus::Stale,
            event_key: "stale".to_string(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExternalPollState {
    next_poll: Instant,
    last_success: Option<Instant>,
    last_event_key: Option<String>,
    last_alert: Option<Instant>,
}

impl Default for ExternalPollState {
    fn default() -> Self {
        Self {
            next_poll: Instant::now(),
            last_success: None,
            last_event_key: None,
            last_alert: None,
        }
    }
}

impl ExternalPollState {
    pub fn should_poll(&self, now: Instant) -> bool {
        now >= self.next_poll
    }

    pub fn mark_success(&mut self, snapshot: &ExternalSnapshot, now: Instant, poll_seconds: u64) {
        self.record(snapshot, now);
        self.last_success = Some(now);
        self.next_poll = now + Duration::from_secs(poll_seconds.max(5));
    }

    pub fn mark_error(&mut self, snapshot: &ExternalSnapshot, now: Instant, poll_seconds: u64) {
        self.record(snapshot, now);
        self.next_poll = now + Duration::from_secs(poll_seconds.saturating_mul(2).clamp(10, 300));
    }

    pub fn stale_snapshot(&mut self, now: Instant, stale_seconds: u64) -> Option<ExternalSnapshot> {
        let last_success = self.last_success?;
        if now.saturating_duration_since(last_success) > Duration::from_secs(stale_seconds.max(1)) {
            let snapshot = ExternalSnapshot::stale("external source status is stale");
            self.record(&snapshot, now);
            Some(snapshot)
        } else {
            None
        }
    }

    pub fn record(&mut self, snapshot: &ExternalSnapshot, now: Instant) {
        if self.last_event_key.as_deref() != Some(snapshot.event_key.as_str()) {
            self.last_event_key = Some(snapshot.event_key.clone());
            self.last_alert = Some(now);
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExternalFetchError {
    #[error("external API URL is not configured")]
    MissingUrl,
    #[error("external API request failed: {0}")]
    Request(String),
    #[error("external API response was not valid text: {0}")]
    Body(#[from] std::io::Error),
    #[error("external API response was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn fetch_json(api_url: &str, token_env: &str) -> Result<Value, ExternalFetchError> {
    if api_url.trim().is_empty() {
        return Err(ExternalFetchError::MissingUrl);
    }

    let token = env::var(token_env).ok();
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let mut request = agent
        .get(api_url)
        .set("Accept", "application/json")
        .set("User-Agent", "underglow");
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let body = request
        .call()
        .map_err(|error| ExternalFetchError::Request(error.to_string()))?
        .into_string()?;
    Ok(serde_json::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_budget(budget: Arc<WorkerBudget>) -> BackgroundPoll<u8> {
        let mut worker = BackgroundPoll::isolated_for_test();
        worker.budget = budget;
        worker
    }

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(Instant::now() < deadline, "fake worker did not finish");
            std::thread::yield_now();
        }
    }

    #[test]
    fn background_budget_is_shared_across_result_types() {
        let a = BackgroundPoll::<u8>::default();
        let b = BackgroundPoll::<ExternalSnapshot>::default();
        assert!(Arc::ptr_eq(&a.budget, &b.budget));
        assert_eq!(a.budget.limit, MAX_BACKGROUND_POLLS);
        assert!(a.receiver.is_none() && b.receiver.is_none());
    }

    #[test]
    fn background_cap_survives_cancellation_and_rapid_switches() {
        let budget = Arc::new(WorkerBudget {
            active: AtomicUsize::new(0),
            limit: 2,
        });
        let mut first = with_budget(budget.clone());
        let mut second = with_budget(budget.clone());
        let (release_a, wait_a) = mpsc::channel();
        let (release_b, wait_b) = mpsc::channel();
        let (started, observed) = mpsc::channel();
        let started_b = started.clone();
        assert!(first.try_start(move |_| {
            started.send(()).unwrap();
            wait_a.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(1)
        }));
        assert!(second.try_start(move |_| {
            started_b.send(()).unwrap();
            wait_b.recv_timeout(Duration::from_secs(2)).unwrap();
            Ok(2)
        }));
        for _ in 0..2 {
            observed.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        let before = Instant::now();
        assert!(first.try_recv().is_none());
        first.cancel();
        drop(second); // Drop must also cancel without waiting on the provider.
        assert!(before.elapsed() < Duration::from_millis(250));
        assert_eq!(budget.active.load(Ordering::SeqCst), 2);
        for _ in 0..32 {
            let mut replacement = with_budget(budget.clone());
            assert!(!replacement.try_start(|_| panic!("saturated worker must not run")));
        }
        release_a.send(()).unwrap();
        release_b.send(()).unwrap();
        wait_until(|| budget.active.load(Ordering::SeqCst) == 0);
        assert!(first.try_recv().is_none()); // Completed result was discarded.
        assert!(!first.try_start(|_| panic!("cancelled source must stay stopped")));
        let mut replacement = with_budget(budget);
        assert!(replacement.try_start(|_| Ok(3)));
        let mut result = None;
        wait_until(|| {
            result = replacement.try_recv();
            result.is_some()
        });
        assert_eq!(result.unwrap().unwrap(), 3);
    }

    #[test]
    fn background_error_results_are_delivered_once() {
        let mut worker = BackgroundPoll::<u8>::isolated_for_test();
        assert!(worker.try_start(|_| Err("fixture failure".into())));
        let mut result = None;
        wait_until(|| {
            result = worker.try_recv();
            result.is_some()
        });
        assert_eq!(result.unwrap().unwrap_err(), "fixture failure");
        assert!(worker.try_recv().is_none());
        assert!(worker.is_idle());
    }

    #[test]
    fn poll_timing_staleness_and_error_backoff_use_supplied_clock() {
        let mut poll = ExternalPollState::default();
        let now = Instant::now();
        let success = ExternalSnapshot::idle("fixture");
        poll.mark_success(&success, now, 0);
        assert!(!poll.should_poll(now + Duration::from_secs(4)));
        assert!(poll.should_poll(now + Duration::from_secs(5)));
        assert!(
            poll.stale_snapshot(now + Duration::from_secs(9), 10)
                .is_none()
        );
        assert_eq!(
            poll.stale_snapshot(now + Duration::from_secs(11), 10)
                .unwrap()
                .status,
            ExternalStatus::Stale
        );
        poll.mark_error(&ExternalSnapshot::error("fixture"), now, u64::MAX);
        assert!(!poll.should_poll(now + Duration::from_secs(299)));
        assert!(poll.should_poll(now + Duration::from_secs(300)));
        poll.mark_success(&success, now + Duration::from_secs(301), 10);
        assert!(
            poll.stale_snapshot(now + Duration::from_secs(302), 10)
                .is_none()
        );
    }

    #[test]
    fn poll_state_dedupes_repeated_event_keys() {
        let mut state = ExternalPollState::default();
        let now = Instant::now();
        let snapshot = ExternalSnapshot {
            status: ExternalStatus::Alert,
            event_key: "event-1".to_string(),
            message: "alert".to_string(),
        };

        state.mark_success(&snapshot, now, 60);
        state.mark_success(&snapshot, now + Duration::from_secs(1), 60);

        assert_eq!(state.last_event_key.as_deref(), Some("event-1"));
    }

    #[test]
    fn empty_url_is_missing_url_error() {
        assert!(matches!(
            fetch_json("", "TOKEN"),
            Err(ExternalFetchError::MissingUrl)
        ));
    }
}
