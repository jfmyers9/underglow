# Notifications over a base visualization

`examples/notifications.toml` layers a focus timer over a comet. Use the normal
config run/preview or engine selection path; there is still only one RGB writer.
Without `[[notifications]]`, nothing extra is constructed, polled, or executed.

Each entry requires a unique nonempty `id`, `duration_seconds` (1–60), explicit
nonempty `zones`, and `[notifications.signal]` using an existing `command-pulse`,
`github-ci`, or `focus-cockpit` configuration. Up to 16 entries are supported.
`priority` is 0–255 (default 0). Zones are `function`, `alpha`, `navigation`,
`arrows`, and `system`; keys absent from a layout are simply unaffected.
`statuses` optionally restricts which state changes trigger (empty means all).

- Command statuses: `pending`, `running`, `success`, `failure`, `timeout`,
  `interrupted`. Commands run **once per live program**, not per notification.
- GitHub: `idle`, `running`, `passing`, `failing`, `reviewrequested`, `approved`,
  `conflict`, `stale`, `error`. Status plus upstream event identity detects a new
  run even when the conclusion is unchanged. Existing polling/backoff applies.
- Focus: `focus`, `break`, `overtime`, `paused`, `meetingsafe`. Phase/cycle changes
  trigger; progress updates do not. Construction state is a baseline, not an alert.

The highest-priority active entry replaces only its configured zones with its
provider's rendering. Equal priorities use declaration order. All other keys
retain the base. The base and all providers keep advancing while obscured.
Durations run from event detection, including time hidden by a higher priority;
there is no delayed queue. A newer distinct event replaces that entry's deadline.
Stable state never extends or replays an expired alert. On expiry the current
base frame returns, not a saved/stale frame. A still-active lower priority alert
can become visible for its remaining time. The base determines program completion.

Pause, switch, stop, errors, and handoff use the shared runner shutdown, cleaning
up every provider even if another cleanup fails. Resume starts a fresh program;
focus timing restarts and explicitly selected command notifications may run again.
Command-enabled configs must be trusted: selecting/resuming them authorizes their
commands. Avoid commands with destructive or non-idempotent side effects.
Constructors, validation, and previews do not execute commands, poll APIs, or open
hardware. Preview shows only the base; deterministic composition tests use mock
events and an injected clock. No new provider, background SDK session, or writer
is introduced. GitHub polling runs on bounded background workers so an HTTP
request does not stall base frames or pause/status controls. Shutdown cancels
new work without waiting for an in-flight request; its result is discarded.
