# Persistent engine and local controls

The engine owns one RGB session and every mode/input lifecycle. CLI and GUI
clients use the same versioned, user-private Unix socket; no TCP server runs.

```sh
# Foreground engine. A fresh state starts PAUSED, without loading either SDK.
wooting-signals engine
# In another terminal:
wooting-signals control status
wooting-signals control select --preset ripples
wooting-signals control settings --brightness 180 --palette ocean --fps 30
wooting-signals control resume
wooting-signals control pause
wooting-signals control select --preset comet
wooting-signals control resume
wooting-signals control stop
```

Available built-ins: ripples, comet, rainbow, breath, matrix, focus-cockpit.
For CI or another trusted configuration, use
`control select --config /absolute/path/profile.toml`. Selecting while paused
does not start providers. Selecting while active applies immediately; imported
command presets can execute local commands. Review them first. Settings updates
do not restart the current mode or rerun its command.

## State and ownership

- macOS: `~/Library/Application Support/wooting-signals/runtime`.
- Linux: `$XDG_STATE_HOME/wooting-signals`, falling back to
  `~/.local/state/wooting-signals`.
- `WOOTING_STATE_DIR` overrides the default directory/lock namespace. Use the
  same value for all app processes. `engine/control --state-dir` changes the
  state/socket location, not the default per-user ownership locks.
- Directory mode 0700, socket/state mode 0600. Keep it on a local filesystem
  with advisory locking and atomic rename support. Avoid very long paths:
  Unix socket address lengths are OS-limited.
- One engine per user/namespace. Foreground hardware commands refuse while the
  engine owns hardware. Pausing releases both SDK resources and the hardware
  lock, so foreground diagnostics can run. These locks cannot arbitrate with
  Wootility, other users, or third-party RGB writers.
- Explicit pause/stop persists disabled state. Closing a client does not stop
  the engine. SIGINT/SIGTERM/SIGHUP restore/release but retain enabled intent for
  the next service start. Force-kill cannot guarantee physical restoration.

`state.json` stores a versioned snapshot of the selected TOML and enabled intent.
Input configs are never rewritten. New selections/settings validate before
changing the current mode, then save through a private temporary file, fsync,
and atomic rename. Invalid configs leave the last valid snapshot/runtime intact.
A valid selection whose hardware cannot open remains selected in an error state;
it is not silently rolled back or reported as active. A pause releases hardware
even if disk persistence fails, and reports that failure—disable login startup
until saving works again.

`engine --config PATH` only seeds a previously uninitialized state directory.
Saved settings win on restart. Configuration snapshots are limited to 256 KiB;
socket messages to 1 MiB. Keep tokens in environment references, not TOML.

## Recovery and command safety

SDK/input failures close both resources, report the error, and retry after
1, 2, 4, 8, and 16 seconds. After five retries, explicit resume is required.
If releasing an active session also fails, automatic retries stop rather than
fighting an uncertain device owner; inspect the error before explicitly resuming.
Thirty seconds of stable rendering resets the retry budget. Pause cancels
pending retries. The same path handles a device returning after disconnection;
physical reconnect/sleep behavior still needs platform validation.

Presets containing command-pulse (including notification sources) never retry
automatically and always load paused after a process restart: a build/deploy
must not be replayed just because USB failed. Explicit resume authorizes a new run.
Finite modes pause on completion. The engine does not auto-detect Wootility.

GitHub, market, and sports polling run outside the render/control loop, with a
shared limit of four in-flight jobs and no unbounded queue. Pausing cancels new
work and discards results immediately. A request already in flight may finish
under its existing timeout; it keeps its capacity slot until exit. GitHub checks
cancellation between chained requests. Construction and previews start no jobs.
Native SDK calls remain synchronous and may delay interruption until they return.
The GUI keeps its own event loop responsive and reports controller timeouts.
No keyboard presses or travel buffers are logged.

## Protocol

CLI controls emit JSON on stdout and nonzero exit status on failure:
`{ "ok": true, "error": null, "status": { ... } }`.
Status includes schema_version, state (paused/active/retrying/error), enabled,
mode, brightness, palette, fps, last_error, and retry_attempt. Error text can be
present while paused when release/persistence failed; check it before assuming
restoration succeeded.

One newline-delimited JSON request per socket connection:
`{"schema_version":1,"action":"status"}`. Actions are status, pause, resume,
stop, select (config text), and settings (brightness/palette/fps). Unknown
versions/actions/settings are rejected. Same-user access is trusted local
control, including configured command execution. Do not expose this socket over
a network or place it in a shared directory.

See [notifications](notifications.md), [installation](install.md), and
[GUI](gui.md). Automated engine tests use fake native SDKs, not actual HID devices.
