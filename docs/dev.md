# Fast development loop

```sh
make dev
```

Requires Python 3 and the normal Rust GUI build dependencies; no watcher package,
DMG, installation, or login-service changes. The first build may be slow; later
builds reuse Cargo's incremental `target` cache.

After updating from Wooting Signals to Underglow, stop your existing watcher with
Ctrl-C and restart `make dev` once (with `DEV_HARDWARE=1` if needed). The running
Python watcher cannot reload its renamed binary paths. Existing development
settings and the per-checkout lock namespace are preserved; do not kill unrelated
processes or delete their locks.

The default is **simulation**: SDK access is blocked. Use the GUI keyboard preview
to tune effects without taking over the physical keyboard. Saved selections,
brightness, speed, FPS, and colors are private to this checkout and mode.
The preview uses the shared renderers with synthetic time/input; see
[adding visualizations](visualizations.md) for module ownership and registration.

Save Rust/Cargo inputs to rebuild automatically:

- `src/bin/underglow-gui.rs` or `src/bin/gui/**`: restart only the GUI.
- `assets/icon/underglow-256.png`: update the embedded app/Dock icon and restart
  only the GUI. The development app uses the same artwork as the Mac bundle.
- Shared code, engine code, Cargo files, or build inputs: restart both owned processes.
- Compile errors appear in the terminal; the last working processes remain running.
- Each engine startup/restart is paused; lighting never auto-resumes. GUI-only
  reloads leave the engine and its enabled state untouched. Applied settings persist;
  unsaved GUI edits do not.
- Closing/quitting the GUI ends the session when its process exits. Ctrl-C always
  stops the supervisor's GUI, engine, and in-progress build, with bounded cleanup.

The watcher hashes file contents and debounces saves. Failed builds retry after a
further content change, not continuously. It watches `src/**`, `.cargo/**`,
`examples/**` (example/test profiles), Cargo
manifest/lockfile, Rust toolchain files, `build.rs`, and the SDK header referenced
by that build script. External dependencies/toolchain/environment changes require
restarting `make dev`. Builds target the host's normal `target/debug` output; do
not set Cargo cross-compilation target overrides for this workflow.

## Testing the physical keyboard

```sh
make dev DEV_HARDWARE=1
# equivalently: python3 scripts/dev.py --hardware
```

This is explicit hardware opt-in, **not** a separate hardware ownership namespace.
The runner unsets `WOOTING_STATE_DIR` so real hardware and engine leases remain
shared with the installed app. If the installed engine owns the lease, startup
fails without stopping it: explicitly stop the installed engine in the app, then
retry. If login startup immediately relaunches it, explicitly stop that service
before retrying. The runner never edits login settings or stops installed engines.
The development engine starts paused; enable lighting yourself after every engine restart.
Hardware settings are separate from simulation and installed settings.
macOS may require permissions for the development process separately from the
installed app; review any system prompt rather than changing permissions globally.

On macOS, hardware mode discovers the exact SDK libraries in
`/Applications/Underglow.app/Contents/Frameworks` (then `~/Applications/Underglow.app`),
falling back to `Wooting Signals.app` in those locations for each missing library.
It prints its choices and never changes the installed app. Existing
`WOOTING_RGB_SDK_PATH` and `WOOTING_ANALOG_SDK_PATH` overrides take precedence,
even if invalid. Without a bundle or overrides, normal SDK discovery applies.

## State, logs, and process ownership

Startup prints the state/log directories. They live under a mode-0700 short path:
`/tmp/wsdev-<uid>-<checkout-hash>/`. This avoids macOS Unix-socket path limits.
The legacy `wsdev` namespace and internal `WOOTING_*` environment variables remain
unchanged for compatibility and shared hardware ownership safety.
Settings survive supervisor restarts but may be removed by OS temporary-file
cleanup. Remove this directory only while the supervisor is stopped to reset the
development settings. It contains:

- `simulation/` and `hardware/`: saved engine state and control socket.
- `logs/engine.log`, `logs/gui.log`: appended child output.
- `generations/`: private copies of each successful binary set. The GUI's sibling
  CLI always matches its generation; Cargo never overwrites running executables.
- `supervisor.lock`: advisory lock prevents duplicate supervisors for one checkout.

The supervisor tracks only its own child processes; it does not use `pkill`, a
service manager, or detached engines. Engine spawning in the supervised GUI is
disabled. GUI-only rebuilds retain the old engine generation until a shared-code
change requires replacement. No true Rust hot reload: each affected process
restarts after compilation.

## Runner tests

```sh
make test-dev
```

Tests use fake Python executables and temporary state, never a real GUI, SDK,
keyboard, or login service. Packaging remains a separate release-candidate check.
