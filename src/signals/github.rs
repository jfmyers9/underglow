use crate::layout::Zone;
use crate::render::{Color, Frame, RenderContext, pulse_wave};
use crate::signals::SignalProgram;
use crate::signals::external::{
    BackgroundPoll, ExternalPollState, ExternalSnapshot, ExternalStatus,
};
use serde::Deserialize;
use serde_json::Value;
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GitHubCiConfig {
    pub repo: String,
    pub branch: Option<String>,
    pub pull_request: Option<u64>,
    pub token_env: String,
    pub api_base: String,
    pub poll_seconds: u64,
    pub stale_seconds: u64,
}

impl Default for GitHubCiConfig {
    fn default() -> Self {
        Self {
            repo: String::new(),
            branch: None,
            pull_request: None,
            token_env: "GITHUB_TOKEN".to_string(),
            api_base: "https://api.github.com".to_string(),
            poll_seconds: 60,
            stale_seconds: 300,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GitHubCiError {
    #[error("github-ci requires repo in owner/name form")]
    MissingRepo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHubSnapshot {
    pub status: GitHubCiStatus,
    pub event_key: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHubCiStatus {
    Idle,
    Running,
    Passing,
    Failing,
    ReviewRequested,
    Approved,
    Conflict,
    Stale,
    Error,
}

#[derive(Debug)]
pub struct GitHubCiSignal {
    config: GitHubCiConfig,
    state: GitHubSnapshot,
    poll: ExternalPollState,
    worker: BackgroundPoll<GitHubSnapshot>,
}

impl GitHubCiSignal {
    pub fn new(config: GitHubCiConfig) -> Result<Self, GitHubCiError> {
        if !config.repo.contains('/') {
            return Err(GitHubCiError::MissingRepo);
        }

        Ok(Self {
            config,
            state: GitHubSnapshot {
                status: GitHubCiStatus::Idle,
                event_key: "idle".to_string(),
                message: "waiting for first GitHub poll".to_string(),
            },
            poll: ExternalPollState::default(),
            worker: BackgroundPoll::default(),
        })
    }

    fn poll_if_due(&mut self) {
        if self.worker.is_cancelled() {
            return;
        }
        let now = std::time::Instant::now();
        if let Some(result) = self.worker.try_recv() {
            match result {
                Ok(snapshot) => {
                    self.poll.mark_success(
                        &external_from_github(&snapshot),
                        now,
                        self.config.poll_seconds,
                    );
                    self.state = snapshot;
                }
                Err(error) => {
                    eprintln!("github-ci poll failed for {}: {error}", self.config.repo);
                    let snapshot = GitHubSnapshot {
                        status: GitHubCiStatus::Error,
                        event_key: format!("error:{error}"),
                        message: "GitHub poll failed".to_string(),
                    };
                    self.poll.mark_error(
                        &external_from_github(&snapshot),
                        now,
                        self.config.poll_seconds,
                    );
                    self.state = snapshot;
                }
            }
            return;
        }
        if let Some(snapshot) = self.poll.stale_snapshot(now, self.config.stale_seconds) {
            self.state = github_from_external(snapshot);
        }
        if self.poll.should_poll(now) && self.worker.is_idle() {
            let config = self.config.clone();
            self.worker.try_start(move |cancelled| {
                fetch_snapshot(&config, cancelled).map_err(|error| error.to_string())
            });
        }
    }

    fn status_color(&self) -> Color {
        match self.state.status {
            GitHubCiStatus::Idle => Color::new(0, 48, 96),
            GitHubCiStatus::Running => Color::new(0, 180, 255),
            GitHubCiStatus::Passing | GitHubCiStatus::Approved => Color::new(0, 220, 80),
            GitHubCiStatus::Failing | GitHubCiStatus::Conflict | GitHubCiStatus::Error => {
                Color::new(255, 32, 24)
            }
            GitHubCiStatus::ReviewRequested => Color::new(255, 180, 0),
            GitHubCiStatus::Stale => Color::new(120, 120, 120),
        }
    }
}

fn external_from_github(snapshot: &GitHubSnapshot) -> ExternalSnapshot {
    ExternalSnapshot {
        status: match snapshot.status {
            GitHubCiStatus::Idle => ExternalStatus::Idle,
            GitHubCiStatus::Running => ExternalStatus::Running,
            GitHubCiStatus::Passing | GitHubCiStatus::Approved => ExternalStatus::Positive,
            GitHubCiStatus::Failing
            | GitHubCiStatus::ReviewRequested
            | GitHubCiStatus::Conflict => ExternalStatus::Alert,
            GitHubCiStatus::Stale => ExternalStatus::Stale,
            GitHubCiStatus::Error => ExternalStatus::Error,
        },
        event_key: snapshot.event_key.clone(),
        message: snapshot.message.clone(),
    }
}

fn github_from_external(snapshot: ExternalSnapshot) -> GitHubSnapshot {
    GitHubSnapshot {
        status: match snapshot.status {
            ExternalStatus::Idle => GitHubCiStatus::Idle,
            ExternalStatus::Running => GitHubCiStatus::Running,
            ExternalStatus::Positive => GitHubCiStatus::Passing,
            ExternalStatus::Negative | ExternalStatus::Alert => GitHubCiStatus::Failing,
            ExternalStatus::Stale => GitHubCiStatus::Stale,
            ExternalStatus::Error => GitHubCiStatus::Error,
        },
        event_key: snapshot.event_key,
        message: snapshot.message,
    }
}

impl SignalProgram for GitHubCiSignal {
    fn notification_snapshot(&self) -> Option<crate::signals::SignalSnapshot> {
        let mut snapshot = crate::signals::SignalSnapshot::status(
            "github-ci",
            format!("{:?}", self.state.status).to_lowercase(),
        );
        snapshot.message = self.state.event_key.clone();
        Some(snapshot)
    }
    fn tick(&mut self, interrupted: &AtomicBool) -> crate::signals::ProgramResult {
        if interrupted.load(Ordering::SeqCst) {
            self.worker.cancel();
        } else {
            self.poll_if_due();
        }
        Ok(())
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let mut frame = Frame::black();
        let base = self.status_color();
        let wave = pulse_wave(ctx.tick, 36);
        let pulse = match self.state.status {
            GitHubCiStatus::Running | GitHubCiStatus::ReviewRequested => 96 + (wave / 3),
            GitHubCiStatus::Failing | GitHubCiStatus::Conflict | GitHubCiStatus::Error => {
                128 + (wave / 2)
            }
            _ => 128,
        };
        let primary = base.scale(((u16::from(ctx.brightness) * u16::from(pulse)) / 255) as u8);
        let dim = base.scale(ctx.brightness / 10);

        for key in ctx.layout.keys() {
            frame.set_coord(key.coord, dim);
        }

        for key in ctx.layout.keys() {
            let active = match self.state.status {
                GitHubCiStatus::Running
                | GitHubCiStatus::Passing
                | GitHubCiStatus::Failing
                | GitHubCiStatus::Stale
                | GitHubCiStatus::Error => key.zone == Zone::Function,
                GitHubCiStatus::ReviewRequested | GitHubCiStatus::Approved => {
                    key.zone == Zone::Navigation || key.zone == Zone::Arrows
                }
                GitHubCiStatus::Conflict => {
                    key.zone == Zone::Function || key.zone == Zone::Navigation
                }
                GitHubCiStatus::Idle => key.zone == Zone::Function,
            };
            if active {
                frame.set_coord(key.coord, primary);
            }
        }

        frame
    }

    fn finished(&self) -> bool {
        false
    }

    fn shutdown(&mut self, _interrupted: bool) -> crate::signals::ProgramResult {
        self.worker.cancel();
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
enum FetchError {
    #[error("GitHub poll cancelled")]
    Cancelled,
    #[error("GitHub API request failed: {0}")]
    Request(String),
    #[error("GitHub API response was not valid UTF-8/text: {0}")]
    Body(#[from] std::io::Error),
    #[error("GitHub API response was not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
}

fn fetch_snapshot(
    config: &GitHubCiConfig,
    cancelled: &AtomicBool,
) -> Result<GitHubSnapshot, FetchError> {
    let token = env::var(&config.token_env).ok();
    fetch_snapshot_with(config, cancelled, |path| {
        get_json(config, path, token.as_deref())
    })
}

fn fetch_snapshot_with(
    config: &GitHubCiConfig,
    cancelled: &AtomicBool,
    mut get: impl FnMut(&str) -> Result<Value, FetchError>,
) -> Result<GitHubSnapshot, FetchError> {
    // A switch/pause may happen during any request. Never start the next one
    // after cancellation; the request already in flight keeps its 10s timeout.
    let mut request = |path: &str| {
        if cancelled.load(Ordering::SeqCst) {
            Err(FetchError::Cancelled)
        } else {
            get(path)
        }
    };
    let actions = request(&format!(
        "/repos/{}/actions/runs?per_page=10{}",
        config.repo,
        config
            .branch
            .as_ref()
            .map(|branch| format!("&branch={branch}"))
            .unwrap_or_default(),
    ))?;
    let pr = config
        .pull_request
        .map(|number| request(&format!("/repos/{}/pulls/{number}", config.repo)))
        .transpose()?;
    let reviews = config
        .pull_request
        .map(|number| request(&format!("/repos/{}/pulls/{number}/reviews", config.repo)))
        .transpose()?;
    Ok(normalize_snapshot(&actions, pr.as_ref(), reviews.as_ref()))
}

fn get_json(config: &GitHubCiConfig, path: &str, token: Option<&str>) -> Result<Value, FetchError> {
    let url = format!("{}{}", config.api_base.trim_end_matches('/'), path);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let mut request = agent
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "underglow");
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let body = request
        .call()
        .map_err(|error| FetchError::Request(error.to_string()))?
        .into_string()?;
    Ok(serde_json::from_str(&body)?)
}

pub fn normalize_snapshot(
    actions: &Value,
    pull_request: Option<&Value>,
    reviews: Option<&Value>,
) -> GitHubSnapshot {
    if let Some(pr) = pull_request {
        if pr_has_conflict(pr) {
            return GitHubSnapshot {
                status: GitHubCiStatus::Conflict,
                event_key: pr_event_key("conflict", pr),
                message: "PR has merge conflicts".to_string(),
            };
        }
        if pr_review_requested(pr) {
            return GitHubSnapshot {
                status: GitHubCiStatus::ReviewRequested,
                event_key: pr_event_key("review-requested", pr),
                message: "PR review requested".to_string(),
            };
        }
        if reviews_approved(reviews) {
            return GitHubSnapshot {
                status: GitHubCiStatus::Approved,
                event_key: pr_event_key("approved", pr),
                message: "PR approved".to_string(),
            };
        }
    }

    normalize_actions(actions)
}

fn normalize_actions(actions: &Value) -> GitHubSnapshot {
    let Some(run) = actions
        .get("workflow_runs")
        .and_then(Value::as_array)
        .and_then(|runs| runs.first())
    else {
        return GitHubSnapshot {
            status: GitHubCiStatus::Idle,
            event_key: "actions:none".to_string(),
            message: "No GitHub Actions runs found".to_string(),
        };
    };

    let id = run.get("id").and_then(Value::as_i64).unwrap_or_default();
    let status = run
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let conclusion = run.get("conclusion").and_then(Value::as_str);
    let head_sha = run
        .get("head_sha")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    match (status, conclusion) {
        ("queued" | "in_progress" | "waiting" | "requested" | "pending", _) => GitHubSnapshot {
            status: GitHubCiStatus::Running,
            event_key: format!("actions:{id}:{head_sha}:running"),
            message: "GitHub Actions running".to_string(),
        },
        ("completed", Some("success")) => GitHubSnapshot {
            status: GitHubCiStatus::Passing,
            event_key: format!("actions:{id}:{head_sha}:success"),
            message: "GitHub Actions passing".to_string(),
        },
        ("completed", Some("failure" | "timed_out" | "cancelled" | "action_required")) => {
            GitHubSnapshot {
                status: GitHubCiStatus::Failing,
                event_key: format!("actions:{id}:{head_sha}:failure"),
                message: "GitHub Actions failing".to_string(),
            }
        }
        _ => GitHubSnapshot {
            status: GitHubCiStatus::Stale,
            event_key: format!("actions:{id}:{head_sha}:unknown"),
            message: "GitHub Actions state unknown".to_string(),
        },
    }
}

fn pr_has_conflict(pr: &Value) -> bool {
    pr.get("mergeable").and_then(Value::as_bool) == Some(false)
        || matches!(
            pr.get("mergeable_state").and_then(Value::as_str),
            Some("dirty" | "blocked" | "behind")
        )
}

fn pr_review_requested(pr: &Value) -> bool {
    pr.get("requested_reviewers")
        .and_then(Value::as_array)
        .is_some_and(|reviewers| !reviewers.is_empty())
        || pr
            .get("requested_teams")
            .and_then(Value::as_array)
            .is_some_and(|teams| !teams.is_empty())
}

fn reviews_approved(reviews: Option<&Value>) -> bool {
    reviews.and_then(Value::as_array).is_some_and(|reviews| {
        reviews
            .iter()
            .any(|review| review.get("state").and_then(Value::as_str) == Some("APPROVED"))
    })
}

fn pr_event_key(prefix: &str, pr: &Value) -> String {
    let number = pr.get("number").and_then(Value::as_i64).unwrap_or_default();
    let sha = pr
        .pointer("/head/sha")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    format!("pr:{number}:{sha}:{prefix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::KeyboardLayout;
    use crate::render::{PaletteName, RenderContext};
    use crate::sdk::rgb::{DeviceInfo, DeviceType, Layout};

    fn actions(status: &str, conclusion: Option<&str>) -> Value {
        serde_json::json!({
            "workflow_runs": [{
                "id": 42,
                "status": status,
                "conclusion": conclusion,
                "head_sha": "abc123"
            }]
        })
    }

    fn pr(extra: Value) -> Value {
        let mut value = serde_json::json!({
            "number": 7,
            "mergeable": true,
            "mergeable_state": "clean",
            "requested_reviewers": [],
            "requested_teams": [],
            "head": { "sha": "def456" }
        });
        merge_json(&mut value, extra);
        value
    }

    fn merge_json(target: &mut Value, patch: Value) {
        let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) else {
            return;
        };
        for (key, value) in patch {
            target.insert(key.clone(), value.clone());
        }
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            connected: true,
            model: "test".to_string(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        }
    }

    #[test]
    fn background_tick_keeps_cached_state_and_shutdown_does_not_wait() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        let mut signal = GitHubCiSignal::new(GitHubCiConfig {
            repo: "fixture/repo".into(),
            ..Default::default()
        })
        .unwrap();
        signal.worker = BackgroundPoll::isolated_for_test();
        let original = signal.state.clone();
        let (release, wait) = mpsc::channel();
        let (started, entered) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        assert!(signal.worker.try_start(move |cancelled| {
            started.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(cancelled.load(Ordering::SeqCst));
            done.send(()).unwrap();
            Ok(GitHubSnapshot {
                status: GitHubCiStatus::Passing,
                event_key: "fixture".into(),
                message: "fixture".into(),
            })
        }));
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        for _ in 0..100 {
            signal.tick(&AtomicBool::new(false)).unwrap();
        }
        assert_eq!(signal.state, original);
        signal.shutdown(true).unwrap();
        assert!(start.elapsed() < Duration::from_millis(250));
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        signal.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(signal.state, original);
        assert!(signal.worker.is_cancelled());
    }

    #[test]
    fn background_results_update_cache_and_preserve_failure_backoff() {
        let mut signal = GitHubCiSignal::new(GitHubCiConfig {
            repo: "fixture/repo".into(),
            ..Default::default()
        })
        .unwrap();
        let snapshot = GitHubSnapshot {
            status: GitHubCiStatus::Passing,
            event_key: "fixture".into(),
            message: "fixture".into(),
        };
        signal.worker = BackgroundPoll::ready_for_test(Ok(snapshot.clone()));
        signal.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(signal.state, snapshot);
        assert!(!signal.poll.should_poll(std::time::Instant::now()));
        signal.worker = BackgroundPoll::ready_for_test(Err("fixture failure".into()));
        signal.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(signal.state.status, GitHubCiStatus::Error);
        assert!(!signal.poll.should_poll(std::time::Instant::now()));
        signal.shutdown(true).unwrap();
    }

    #[test]
    fn cancelled_github_poll_never_starts_next_request() {
        let config = GitHubCiConfig {
            repo: "fixture/repo".into(),
            pull_request: Some(1),
            ..Default::default()
        };
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let result = fetch_snapshot_with(&config, &cancelled, |_| {
            calls += 1;
            cancelled.store(true, Ordering::SeqCst);
            Ok(serde_json::json!({"workflow_runs": []}))
        });
        assert!(matches!(result, Err(FetchError::Cancelled)));
        assert_eq!(calls, 1);
        assert!(matches!(
            fetch_snapshot_with(&config, &cancelled, |_| panic!(
                "cancelled before first request"
            )),
            Err(FetchError::Cancelled)
        ));
    }

    #[test]
    fn normalizes_actions_running_success_and_failure() {
        assert_eq!(
            normalize_actions(&actions("in_progress", None)).status,
            GitHubCiStatus::Running
        );
        assert_eq!(
            normalize_actions(&actions("completed", Some("success"))).status,
            GitHubCiStatus::Passing
        );
        assert_eq!(
            normalize_actions(&actions("completed", Some("failure"))).status,
            GitHubCiStatus::Failing
        );
    }

    #[test]
    fn normalizes_pr_review_conflict_and_approval() {
        let requested = pr(serde_json::json!({ "requested_reviewers": [{ "login": "octocat" }] }));
        assert_eq!(
            normalize_snapshot(
                &actions("completed", Some("success")),
                Some(&requested),
                None
            )
            .status,
            GitHubCiStatus::ReviewRequested
        );

        let conflict = pr(serde_json::json!({ "mergeable": false }));
        assert_eq!(
            normalize_snapshot(
                &actions("completed", Some("success")),
                Some(&conflict),
                None
            )
            .status,
            GitHubCiStatus::Conflict
        );

        let approved = pr(serde_json::json!({}));
        let reviews = serde_json::json!([{ "state": "APPROVED" }]);
        assert_eq!(
            normalize_snapshot(
                &actions("completed", Some("success")),
                Some(&approved),
                Some(&reviews)
            )
            .status,
            GitHubCiStatus::Approved
        );
    }

    #[test]
    fn event_keys_are_stable_for_repeated_polls() {
        let first = normalize_actions(&actions("completed", Some("failure")));
        let second = normalize_actions(&actions("completed", Some("failure")));

        assert_eq!(first.event_key, second.event_key);
    }

    #[test]
    fn github_ci_rejects_missing_repo() {
        assert!(GitHubCiSignal::new(GitHubCiConfig::default()).is_err());
    }

    #[test]
    fn github_ci_renders_full_frame() {
        let info = info();
        let layout = KeyboardLayout::for_device(&info);
        let mut signal = GitHubCiSignal::new(GitHubCiConfig {
            repo: "owner/repo".to_string(),
            ..GitHubCiConfig::default()
        })
        .unwrap();
        signal.state = GitHubSnapshot {
            status: GitHubCiStatus::Conflict,
            event_key: "conflict".to_string(),
            message: "conflict".to_string(),
        };

        let frame = signal.render(&RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        });

        assert_eq!(frame.as_bytes().len(), crate::render::FRAME_BYTES);
    }
}
