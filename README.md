# Wooting keyboard playground

Interactive toys and visualizations for Wooting keyboards. Pick a toy, play, stop,
and return to your normal lighting. Wootility remains the configuration tool.

The binary is still named `wooting-signals` for compatibility. Existing status
utilities (build feedback, CI, timers, and API alerts) remain available, but the
main direction is keyboard toys rather than a workstation dashboard.

See the [companion app roadmap](docs/roadmap.md) for the Wootility coexistence
model, macOS/Linux packaging, background engine, configuration, and GUI plan.

## Companion app

The optional native GUI and CLI control the same persistent engine. A fresh
engine starts paused; nothing takes over lighting until you explicitly resume.

```sh
cargo run -- engine
# In another terminal:
cargo run -- control select --preset ripples
cargo run -- control resume
cargo run -- control settings --brightness 180 --palette ocean
cargo run -- control pause
```

Build the optional settings app with `cargo build --features gui --bins`, then
launch `target/debug/wooting-gui`. Closing its window leaves the engine running.
Use Pause to restore/release lighting, or Stop engine to shut it down.

- [Engine, persistence, local control, and recovery](docs/engine.md)
- [Native GUI and service controls](docs/gui.md)
- [macOS/Linux packaging and installation](docs/install.md)
- [Opt-in build/CI/timer notifications over a base effect](docs/notifications.md)

No service or login startup is enabled automatically. Foreground commands refuse
while the engine owns hardware; pause it before using `doctor` or a standalone toy.

## First toy: pressure-driven ripples

Press a key lightly for a gentle ripple; press deeper for brighter expanding
rings. Hold to keep emitting, release to let them fade. "Pressure" means measured
key travel, not force. No audio capture, network access, or Accessibility access
is used by the toy itself.

Preview synthetic key travel without hardware or either native SDK:

```sh
cargo run -- toy ripples --preview
cargo run -- toy ripples --preview --ticks 24 --format svg > ripples.svg
```

For live input, first build the RGB SDK using the quick-start instructions below.
Then download **Analog SDK v0.9.1** from the
[official releases](https://github.com/WootingKb/wooting-analog-sdk/releases/tag/v0.9.1)
and extract it somewhere you control. Use the archive matching your machine:

| Platform | Archive suffix | Library inside the archive |
| --- | --- | --- |
| Apple Silicon | `aarch64-apple-darwin.tar.gz` | `release/libwooting_analog_sdk_dist.dylib` |
| Intel Mac | `x86_64-apple-darwin.tar.gz` | `release/libwooting_analog_sdk_dist.dylib` |
| Linux x86-64 | `x86_64-unknown-linux-gnu.tar.gz` | `release/libwooting_analog_sdk_dist.so` |

No system-wide installation is required when using the distributable library.
The upstream Homebrew Analog SDK package is documented as outdated. On macOS,
follow the official SDK security/permission guidance if loading or HID access is
blocked. Linux needs permission to access the keyboard's HID interfaces (udev).

```sh
export WOOTING_ANALOG_SDK_PATH="/absolute/path/to/release/libwooting_analog_sdk_dist.dylib"
cargo run -- toy ripples --palette ocean
# Or use --analog-sdk-path; optionally stop automatically:
cargo run -- toy ripples --seconds 20 --brightness 180 --fps 30
```

`toy ripples` defaults to brightness **180/255**. Use `--brightness` to tune it; this
is a fixed cap, not a reading of or synchronization with Wootility's brightness.

**First-version limits:** one connected analog keyboard, **80HE ANSI**. The typing
block (letters, digits, punctuation, Tab, Caps Lock, Enter, Backspace, Shift and
Space) launches ripples. Fn/custom keys, function keys and navigation keys do not
yet launch them. Rendering uses the existing approximate spatial grid; the light
bar has no dedicated mapping yet. Other models/layouts are rejected, not guessed.
The public Analog SDK C API does not expose v2 physical-position metadata, so
unusual firmware remappings/layers need hardware validation. OS text layouts are
not used. Preview input is simulated, not a hardware compatibility test.

### Living alongside Wootility

- The toy **owns the RGB frame while running**; idle keys are dark. This is not a
  transparent layer over your Wootility animation.
- Ctrl-C or `--seconds` stops it and calls the RGB SDK's lighting restore/close
  operation. Runtime read/write failures also attempt restoration and release
  analog resources. Failed restoration produces a warning; force-kill/crashes
  cannot guarantee cleanup.
- No profile, binding, actuation or firmware settings are written. **Typing still
  reaches the focused application**: use a blank editor while playing.
- Wootility and its Background Service may remain running on the tested setup.
  Stop the toy before editing live lighting in Wootility. Pause competing RGB
  apps or automatic profile switching if they interfere; there is no ownership
  arbitration or automatic Wootility detection yet.
- Standalone toy commands install no login item and start no background service.
  The optional GUI and engine use this same start/stop lifecycle.

macOS is the primary target; the loader also supports Linux. A timed ripple
session on an attached 80HE completed without SDK errors and the effect was
visually confirmed. Subsequent comet probes confirmed stable lighting and visual
restoration with Wootility alone and with its Background Service; Wootility could
edit lighting afterward. Pressing Mode changed the profile indicator while our
key animation continued; stopping revealed the newly selected profile's lighting.
See [hardware findings](docs/roadmap.md#hardware-findings) for the evidence limits.
App Linking, simultaneous live lighting edits, exact key/LED alignment,
disconnect/sleep recovery, and Linux behavior remain unverified. Automated tests
cover deterministic rendering and simulated SDK failures, not visual behavior.

### Repeatable coexistence checks

Use `doctor` to distinguish SDK failures from observations that require a person:

```sh
# Open/check/close RGB, without painting an animation. Analog SDK is not required.
cargo run -- doctor

# Explicit three-second comet probe at low brightness, followed by restore/close.
cargo run -- doctor --probe-rgb --seconds 3 --brightness 48

# Also test the ripple analog backend while the RGB session is open.
# Set WOOTING_ANALOG_SDK_PATH first, or pass --analog-sdk-path.
cargo run -- doctor --probe-rgb --analog --json
```

**This is not a read-only monitor:** opening the RGB SDK initializes a lighting
session, and closing it requests restoration, even without `--probe-rgb`. Do not
run it alongside another instance of this app. It does not write keyboard
configuration, change service settings, or stop other processes. The analog check
uses the current single-80HE backend and never logs pressed keys. Other RGB
effects and RGB-only diagnostics do not require the Analog SDK.

The report separates enumeration, usable metadata, optional frame writes,
optional analog access, and SDK cleanup acknowledgements. `--json` emits the
report on stdout; application diagnostics go to stderr. Failure or interruption
exits nonzero, with cleanup attempted before the report. An acknowledged reset
is **not** proof of visual restoration or exclusive access. SDK calls may have
their own timeout, so interruption is best-effort rather than instantaneous.
An explicit `--sdk-path` or `WOOTING_RGB_SDK_PATH` is authoritative: a missing
library fails rather than silently selecting a different installed SDK.

For milestone 1, repeat the same probe under each condition below. Start with
Wootility idle; separately test live lighting edits and App Linking/profile
switches once the idle combination works. Manage Wootility/Background Service
manually and verify actual process state: a start-at-login preference is not a
stop button. Do not infer a conflict just because one happens to be running.

| Scenario | What to observe |
| --- | --- |
| Wootility and Background Service stopped | Baseline communication and restoration |
| Only Wootility running, initially idle | SDK results, stable effect, post-exit editing |
| Only Background Service running | Stable effect; profile switches as a separate case |
| Both running, initially idle | Same checks, then live edits/profile changes separately |
| Ctrl-C during a longer probe | Incomplete probe reported; cleanup acknowledged; normal lighting returns |
| Unplug during a probe, then reconnect | Clear error and cleanup attempt; a fresh command works after reconnect |
| Sleep/wake during a longer probe | Record actual behavior; automatic recovery is not implemented yet |

Use `--seconds 30` for interruption/reconnection checks. Do not force a host to
sleep or change profiles without coordinating with the person using it. After
every run, verify both **normal lighting returned** and **Wootility can edit it**.
Then separately run ripples to check physical key alignment; `doctor` does not
validate analog-to-LED mapping or light-bar coverage.

When comparing results, record OS/architecture, keyboard/firmware, RGB SDK
revision, Wootility version, process state, command/report, and the person's
observations. Repeat on Linux with appropriate HID permissions before claiming
Linux compatibility. These are test scenarios, not a list of verified supported
combinations. Keep the SDK response-size fix in place throughout these tests.

## What it does

- **Ripples**: travel-driven expanding rings on an 80HE, plus offline previews.

- **Command Pulse**: wraps a command and shows running / success / failure lighting.
- **GitHub / CI Beacon**: maps Actions and PR status to keyboard zones.
- **Focus Cockpit**: shows focus, break, paused, and overtime states.
- **Market Pulse** and **Sports / Racing Alerts**: poll provider APIs and render alert states.
- **App Aura**: manually switch workstation profiles such as terminal, meeting, game, or late night.
- **Soundwave**: opt-in desk-toy prototype driven by manual audio levels.
- **Static effects**: run comet, rainbow, matrix, breath, and row test scenes.

Wooting Signals is **not** a Wootility replacement. Configure firmware, key maps, actuation, rapid trigger, onboard profiles, and baseline lighting in Wootility; run Wooting Signals when you want host-driven overlays.

## Quick start

### 1. Build the RGB SDK

From the repository root, build the official [`WootingKb/wooting-rgb-sdk`](https://github.com/WootingKb/wooting-rgb-sdk). Wooting Signals loads this library at runtime.

Use the pinned submodule revision (`451f40d` or later), not the v1.8.0 tag.
It includes the upstream fix for variable-length multi-report responses. Without
that fix, an 80HE on newer firmware can enumerate but report `layout: Unknown`
and time out on RGB commands. After updating the submodule, rebuild the native
library; updating the Rust binary alone does not update the SDK.

macOS:

```sh
brew install automake pkg-config hidapi libusb
git submodule update --init --recursive
(cd external/wooting-rgb-sdk/mac && make)
```

Linux:

```sh
# Install your distro packages for pkg-config, gcc, make, and hidapi/hidapi-hidraw headers.
git submodule update --init --recursive
(cd external/wooting-rgb-sdk/linux && make)
```

Linux may also need udev rules or permissions that allow access to the keyboard HID interface.

If the SDK library is not found automatically, pass `--sdk-path` or set:

```sh
export WOOTING_RGB_SDK_PATH="$PWD/external/wooting-rgb-sdk/mac/libwooting-rgb-sdk.dylib"
```

Use the matching `.so` path on Linux.

### 2. Check the keyboard

```sh
cargo run -- info
cargo run -- test --brightness 96 --seconds 3
```

### 3. Wrap a build or test command

```sh
cargo run -- signal run command-pulse --palette wooting -- make check
cargo run -- signal run command-pulse --timeout-seconds 120 -- cargo test
```

Command output is inherited by default, so your build/test logs remain visible. Add `--summary` for a short completion line or `--output quiet` when lighting feedback is enough.

## Common commands

```sh
# Device and layout
cargo run -- info
cargo run -- layout-info

# Effects
cargo run -- rainbow --brightness 128 --seconds 10 --fps 30
cargo run -- effect comet --palette cyberpunk --brightness 128 --seconds 10 --fps 30

# Direct signals
cargo run -- signal run static-effect --effect comet --palette cyberpunk --seconds 10
cargo run -- signal run focus-cockpit --focus-minutes 25 --break-minutes 5 --cycles 4 --dim
cargo run -- signal run github-ci --repo owner/repo --branch main
cargo run -- signal run app-aura --profile terminal --dim

# Offline previews: no keyboard SDK, network, or token required.
cargo run -- preview effect comet --ticks 3 --format ansi
cargo run -- preview effect comet --ticks 3 --format json
cargo run -- run --config examples/fixture-replay.toml --dry-run --preview --preview-format svg
```

## Run from a profile

Profiles are TOML files that let you save a signal, brightness, FPS, timing, and integration settings.

Ripples uses the same runtime and configuration as animations and status modes:

```sh
cargo run -- run --config examples/ripples.toml --dry-run
cargo run -- run --config examples/ripples.toml --preview --preview-format json
# Live keyboard input; Ctrl-C restores/releases lighting:
cargo run -- run --config examples/ripples.toml
```

The example selects `[signal] kind = "ripples"`, brightness 180, palette `ocean`,
30 FPS, and continuous operation. Set `[signal.ripples] analog_sdk_path` to
override `WOOTING_ANALOG_SDK_PATH`. RGB-only modes never initialize analog input.
`toy ripples` remains available and shares the same renderer and lifecycle.

Configuration compatibility and validation:

- `schema_version = 1` is optional; existing unversioned profiles mean version 1.
  Unsupported versions are rejected, without rewriting files.
- Existing `[signal]`, `[extension]` alias, and source/rule/scene profiles remain
  supported. Legacy root defaults remain unchanged (including brightness 96);
  use the ripple example for the toy's brighter settings.
- Configured/live frame rates must be 1–120 FPS. A profile may contain at most
  one ripple source because the Analog SDK is process-global.
- Dry-run validates configuration and constructs the selected mode without
  loading SDKs, polling APIs, or starting commands. Preview uses synthetic input;
  JSON/SVG stdout contains only the requested preview, not a config preamble.
- Foreground `run` reads configuration once. For live selection/settings,
  persisted pause, and atomic configuration snapshots, use the engine controls.
  Invalid engine selections leave its previous configuration and mode intact.

Validate without touching the keyboard:

```sh
cargo run -- run --config examples/wooting-signals.toml --dry-run
cargo run -- run --config examples/command-pulse.toml --dry-run
cargo run -- run --config examples/github-ci.toml --dry-run
cargo run -- run --config examples/focus-cockpit.toml --dry-run
```

Preview profile frames without touching the keyboard:

```sh
cargo run -- run --config examples/fixture-replay.toml --dry-run --preview
cargo run -- run --config examples/fixture-replay.toml --dry-run --preview --preview-format json
cargo run -- run --config examples/fixture-replay.toml --dry-run --preview --preview-format svg > preview.svg
```

Run a profile:

```sh
cargo run -- run --config examples/wooting-signals.toml
```

Minimal Command Pulse profile:

```toml
palette = "wooting"
brightness = 128
fps = 30
continuous = true

[signal]
kind = "command-pulse"
command = ["make", "check"]
output = "inherit"
summary = true
timeout_seconds = 600
success_hold_seconds = 3
failure_hold_seconds = 6
interrupted_hold_seconds = 2
```

See [`examples/`](examples/) for ready-to-edit profiles.

## Signal guide

### Command Pulse

Use this around local work that has a clear exit code: tests, builds, linters, deploy scripts, or long-running commands.

```sh
cargo run -- signal run command-pulse -- make check
cargo run -- signal run command-pulse --cwd "$PWD" --env RUST_LOG=info --summary -- cargo test
```

Lighting states:

- running: animated progress
- success: success hold
- failure: failure hold
- timeout: timeout alert
- interrupted: Ctrl-C/interrupted hold

### GitHub / CI Beacon

Poll GitHub and show Actions / PR state on keyboard zones.

```sh
export GITHUB_TOKEN=ghp_... # optional for public repos; recommended for private repos/rate limits
cargo run -- signal run github-ci --repo owner/repo --branch main
cargo run -- signal run github-ci --repo owner/repo --pull-request 123 --poll-seconds 60
```

Dry-run output prints token environment variable names only, never token values.

### Focus Cockpit

Render focus and break progress on the function row, with paused, dim, meeting-safe, and overtime modes.

```sh
cargo run -- signal run focus-cockpit --focus-minutes 25 --break-minutes 5 --cycles 4 --dim
cargo run -- signal run focus-cockpit --meeting-safe --dim
```

### Market Pulse and Sports / Racing Alerts

These poll provider-neutral JSON APIs. Keep tokens in environment variables, not config files. Dry-run output redacts query strings and token values.

```sh
cargo run -- run --config examples/market-pulse.toml --dry-run
cargo run -- run --config examples/sports-alerts.toml --dry-run
```

Expected market shape:

```json
{
  "market_open": true,
  "tickers": [
    {
      "symbol": "WOO",
      "price": 101.0,
      "previous_close": 100.0,
      "change_percent": 1.0
    }
  ]
}
```

Expected sports/racing shape:

```json
{
  "events": [
    {
      "id": "race-1",
      "favorite": "WOO",
      "status": "live",
      "score": 2,
      "opponent_score": 1,
      "previous_score": 1
    }
  ]
}
```

### App Aura and Soundwave

App Aura currently uses manual profiles and requires no macOS Accessibility permission:

```sh
cargo run -- signal run app-aura --profile terminal --dim
cargo run -- signal run app-aura --profile meeting --dim
```

Soundwave is disabled unless explicitly enabled and currently uses manual levels:

```sh
cargo run -- signal run soundwave --enabled --level 0.7 --bass 0.4
```

## Install on macOS or Linux

Build or obtain a reviewed release directory, then run the platform installer:

```sh
# Dry-run first; --apply installs only, never enables login startup.
scripts/install-macos.sh --package /path/to/release
scripts/install-macos.sh --package /path/to/release --apply
# Linux equivalent:
scripts/install-linux.sh --package /path/to/release --apply
```

Packages include the selected native SDKs and relocated dependencies, the CLI,
optional GUI, and a user-service helper. Installed binaries do not depend on a
checkout. Upgrade/uninstall preserve external config and runtime state. Python 3
is required for installation/service management, not for foreground CLI usage.

See [installation and release requirements](docs/install.md) for SDK inputs,
license-review gates, platform prerequisites, explicit login enable/start,
uninstall, and signing/notarization limitations. No signed release or Linux
hardware certification is implied by the packaging tests.

## Safety and coexistence

- Start with moderate brightness, for example `--brightness 96`.
- Wooting Signals opens an RGB session and attempts to reset/close it on normal exit and Ctrl-C.
- If Wootility or Wootility Background Service writes RGB at the same time, lighting is effectively last-writer-wins.
- Long-running profiles should use conservative brightness and polling intervals.

## Troubleshooting

- `failed to load Wooting RGB SDK`: build the SDK and pass `--sdk-path`, or set `WOOTING_RGB_SDK_PATH`.
- `no Wooting RGB keyboard found`: confirm the keyboard is connected and supported by the RGB SDK.
- No lighting change: try `cargo run -- info`, then a short `test`, and verify OS HID permissions.
- Close/reset warning after an effect: the SDK did not acknowledge `wooting_rgb_close()`. If lighting is not restored, rerun `cargo run -- info`, try a short `test`, or unplug/replug the keyboard.

## Development

```sh
make check
make test
make run-info
make run-effect
make config-dry-run
```

Profile v2 support is executable when a config uses typed `[[sources]]`, `[[rules]]`, and `[scenes]` without an overriding `[signal]`. Dry-run output prints `runtime: profile-v2` or `runtime: single-signal`.

```sh
cargo run -- run --config examples/profile-v2.toml --dry-run
cargo run -- run --config examples/profile-v2.toml --dry-run --preview --preview-format svg > profile-preview.svg
```

Showcase profiles are preview-safe and use fixture/replay sources:

```sh
cargo run -- run --config examples/visual-build-lane.toml --dry-run --preview
cargo run -- run --config examples/visual-ci-stack.toml --dry-run --preview
cargo run -- run --config examples/visual-focus-sprint.toml --dry-run --preview
cargo run -- run --config examples/visual-market-heatline.toml --dry-run --preview
cargo run -- run --config examples/visual-sports-burst.toml --dry-run --preview
cargo run -- run --config examples/visual-meeting-safe.toml --dry-run --preview
cargo run -- run --config examples/visual-app-aura.toml --dry-run --preview
```

Compatibility aliases `wooting-extension` and `wooting-hack` are retained during migration, but `wooting-signals` is the primary binary name.
