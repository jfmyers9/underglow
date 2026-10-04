# Wooting companion lighting app: roadmap

Updated: 2026-10-03

This document captures the agreed product direction and implementation order.
Planned capabilities below are not claims about what the current CLI supports.
The initial product is a companion lighting app, not a Wootility replacement.

## Product promise

**Configure your keyboard in Wootility. Use this app to make it interactive,
expressive, and useful.**

Install once, choose a visualization, and let it run without a terminal. Always
provide an obvious way to return to the keyboard's normal lighting. Interactive
toys are the center of the product; CI, build feedback, and timers remain useful
optional modes rather than defining a workstation dashboard.

Keep the existing `wooting-signals` binary and configuration workflows compatible
where practical. A product rename is not a prerequisite for this roadmap.

## Division of responsibility

| Wootility | This app |
| --- | --- |
| Firmware updates, calibration, switch settings | Host-driven RGB visualizations |
| Remapping, actuation, Rapid Trigger, advanced keys | Analog-reactive keyboard toys |
| Onboard profiles and baseline lighting | CI/build notifications and timers |
| Keyboard behavior without a host app running | Presets, runtime, CLI, and lighting GUI |

This app replaces Wootility's **lighting role while custom lighting is active**,
not its keyboard-configuration role. It must not rewrite firmware, bindings,
actuation settings, or onboard profiles as a side effect of running an effect.

Non-goals for the first product release:

- Firmware management, key remapping, or a complete Wootility replacement.
- Installing arbitrary effects inside Wootility or keyboard firmware.
- Blending over Wootility's live animation; no shared compositor is established.
- Automatically matching Wootility brightness; our brightness remains independent.
- A plugin marketplace, remote-control server, or broad new integration catalog.
- Windows packaging, multi-keyboard orchestration, or unverified support for every
  Wooting layout/model. Begin with the 80HE ANSI and expand deliberately.

## Evidence and starting point

As of this document:

- Rust CLI with RGB animations, status utilities, and ANSI/JSON/SVG previews.
- A pressure-driven ripple toy using the official RGB and Analog SDKs. Its input
  mapping covers the 80HE ANSI typing block; spatial layout is approximate and
  the light bar has no dedicated mapping.
- Versioned TOML selects ripples, effects, signals, and source/rule/scene profiles.
  All live modes share resource startup, frame execution, and shutdown; the toy
  CLI remains available. Dry-run and previews acquire no hardware resources.
- A single-instance engine supports live selection/settings, persisted pause,
  local status/control, SIGTERM cleanup, and bounded recovery. Foreground commands
  coordinate through a hardware lease. Command presets are never auto-replayed.
- Optional native GUI, macOS/Linux installers, user-service controls, and a
  dependency-bundling release builder are implemented. Release inputs, dependency
  notices, signing, and physical cross-platform verification remain release gates.
- Opt-in bounded build/CI/timer notifications compose over the base effect without
  another hardware writer. Default toy presets start no network or command sources.
- Existing API polls run off the render/control loop with a shared four-job cap;
  switching or pausing cancels new work without waiting on HTTP responses.
- Hardware-free tests cover configuration, previews, SDK/IPC failure lifecycles,
  GUI control construction, packaging with fake service managers, and native
  dependency relocation. These do not substitute for physical acceptance tests.

### Hardware findings

The initial RGB timeout persisted after Wootility and its background service were
stopped. Updating the RGB SDK from v1.8.0 to upstream commit
[`451f40d`](https://github.com/WootingKb/wooting-rgb-sdk/commit/451f40dc056e0edefccdaba7807a72b694520e92)
fixed variable-length response handling and restored ANSI layout detection.

Timed ripple sessions then exited successfully without reported RGB/reset errors;
the user confirmed seeing the effect on an 80HE connected to macOS. The observed
brightness was 96/255; the default was subsequently raised to 180/255 without a
new hardware run.

Subsequent `doctor --probe-rgb --analog --json` comet probes on this macOS/80HE
setup passed RGB metadata/write/restore and analog open/read/close checks:

| Condition | User-confirmed observation |
| --- | --- |
| Wootility alone, initially idle | Stable effect, normal lighting returned, Wootility could edit afterward |
| Wootility and Background Service, initially idle | Same successful effect and handoff |
| Mode key during a 30-second probe at brightness 48 | Profile indicator changed while the key animation continued; exit revealed the newly selected profile's lighting |

This supports allowing Wootility to remain running and treating our output as a
temporary RGB override, not a keyboard-configuration replacement. It is not a
shared compositor or an exclusive-access guarantee. These observations are for
one setup, not a versioned compatibility matrix; exact firmware/Wootility version
metadata was not recorded with every observation.

Background Service alone, App Linking, concurrent live lighting edits, physical
key alignment, disconnect/reconnect, sleep/wake, and Linux remain unverified.
The new shared-runtime path has hardware-free regression coverage, but has not
received another physical keyboard run. Repeatable checks are documented in the
[README](../README.md#repeatable-coexistence-checks).

## Runtime and coexistence model

```text
Menu-bar / tray GUI --+
                     +--> Local control API --> Background engine --> Keyboard
CLI -----------------+                                |
                                               Saved configuration
```

One engine owns device access for this app. CLI and GUI clients select modes and
edit settings through it; they do not launch competing SDK writers. Existing
foreground commands must eventually coordinate with the engine or refuse clearly
when it owns the device. A per-user single-instance guard prevents duplicate
engines, but cannot arbitrate ownership with third-party RGB applications.

The engine handles rendering, source lifecycles, effect switching, SDK loading,
disconnect/reconnect, and graceful shutdown. The GUI is an optional controller:
closing its settings window does not stop the engine. Users explicitly pause
lighting or quit the runtime to give up control.

### User-visible states

| State | Meaning |
| --- | --- |
| Keyboard lighting | Release our RGB override and device session; use the keyboard's configured lighting |
| Custom lighting | Render the selected mode; our engine owns its RGB output |
| Paused for configuration | Release control while the user works in Wootility; resume explicitly |
| Disconnected / error | Explain the condition, stop failed work, and offer bounded recovery |

Pause means **restore and release**, not merely stop frame transmission. Stop
analog sampling while paused as well. An eventual “Pause and open Wootility”
action should release control before launching Wootility, and should not silently
resume while the user is still configuring it.

- Never kill Wootility or its background service automatically.
- Start with explicit handoff; automatic Wootility detection is deferred until
  supported by reliable evidence and a clear resume policy.
- Persist the selected preset and whether custom lighting is enabled. Explicit
  pause must survive a service restart; startup is opt-in and must respect it.
- On normal stop, Ctrl-C, service termination, or a runtime failure, attempt
  lighting restoration and release resources. Report failures rather than
  claiming success. Force-kill, crashes, and physical disconnection cannot
  guarantee restoration; document recovery.
- Reconnect/sleep recovery must respect paused state, use backoff, and avoid
  repeatedly fighting another RGB writer.

## Configuration and visualization model

Use one versioned, validated TOML configuration model for CLI, engine, and GUI.
Extend the existing model rather than building a second unrelated configuration
system. Define migration behavior for existing profiles before changing semantics.

The first version should configure:

- Active mode/preset and whether custom lighting is enabled.
- Shared brightness, palette, frame-rate limits, and per-mode parameters.
- Optional integration settings, such as the repository used for CI status.
- Device/SDK overrides for troubleshooting, without requiring them normally.

The engine validates changes before applying them and saves atomically. Invalid
edits retain the last valid configuration and produce an actionable error. The
GUI uses the same validation path; there must not be competing UI-only settings.
Keep tokens out of ordinary config and logs through environment/credential-store
references. Treat command-running presets as trusted local code, not content to
execute automatically after importing a shared preset.

Begin with **one active mode**: ripples, an ambient animation, CI status, or a timer.
Later distinguish:

- **Base visualization:** the persistent effect, such as ripples or breathing.
- **Notification:** a temporary event, such as build failure or timer completion.
- **Preset:** a saved combination of visuals, parameters, and optional notifications.

Example future behavior: ripples normally, a failed build flashes the function
row, then ripples continue. Composition, priorities, durations, and return to the
base effect belong inside our engine. Reuse useful existing source/rule/scene
machinery without making its complexity mandatory for a simple toy.

## Platforms, installation, and GUI

Support macOS and Linux through a shared Rust engine and platform-specific
service/packaging adapters. macOS is the primary development target. Publish
support claims only for tested OS/architecture combinations; initial packaging
targets are Apple Silicon/Intel macOS and x86-64 Linux, subject to SDK availability
and validation. Additional architectures can follow.

| Concern | macOS | Linux |
| --- | --- | --- |
| Background runtime | Per-user LaunchAgent | `systemd --user` service |
| GUI | Menu-bar launcher and settings window | Tray/settings app where supported; CLI fallback |
| Packaging | Bundle native SDKs/dependencies; signing and notarization | Bundle or declare native dependencies; distribution-compatible builds |
| Permissions | Document required HID/security permissions | Provide reviewed udev guidance/rules for HID access |

Run in the user's session, **not as a root daemon**. Limited installation steps
such as installing udev rules may require elevated privileges, but the runtime
must not. Provide foreground operation where a user-service manager is absent.

Packages must work without a checkout, compiler, or temporary SDK path. Pin known
working SDK versions, review redistribution terms, and ship required notices.
Document install, start/stop, optional login startup, logs, upgrade, uninstall,
and manual return to keyboard lighting. Preserve user configuration on upgrade;
make data removal an explicit uninstall choice.

The first GUI contains only:

- Active visualization/preset picker and relevant settings.
- Brightness and palette controls.
- Pause/resume and return-to-keyboard-lighting action.
- Start-at-login preference.
- Device, runtime, and error status; access to diagnostics.

Choose a GUI toolkit after the engine control contract works. Prefer local,
user-restricted IPC (for example, a Unix-domain socket); do not expose an
unauthenticated network API merely to support a GUI. Tray availability varies
across Linux desktops, so the settings app and CLI must remain usable without it.

## Implementation sequence and completion criteria

### 1. Establish the coexistence contract

Test the fixed SDK with Wootility alone, Background Service alone, and both;
include App Linking/profile changes and live RGB editing. Verify normal exit,
Ctrl-C, visual restoration, disconnect/reconnect, and sleep/wake on macOS, then
Linux. Check typing-block mapping and record light-bar limitations.

**Done when:** documented supported combinations and a reproducible handoff test
exist. Any required pause of Wootility features is based on observed behavior,
not the old SDK timeout. Do not block ordinary static lighting on unnecessary
analog permissions or dependencies.

### 2. Unify modes and configuration

**Implemented foundation:** `examples/ripples.toml` selects the same ripple
implementation as `toy ripples`. Optional `schema_version = 1` preserves legacy
profiles; unsupported versions and invalid frame rates fail before hardware
startup. Constructors and previews are hardware-free. The shared session attempts
RGB restoration and mode shutdown after normal exit, interruption, and errors,
including partially completed startup. RGB-only modes do not require analog.

**Implemented controls:** the engine supports live select/pause/resume/status,
atomic snapshots, and rejection of invalid edits without changing the active
configuration. Shared brightness/palette/FPS edits do not restart command presets.
Original TOML inputs remain unchanged; see [engine behavior](engine.md).

Bring ripples into the shared runtime/configuration model alongside existing
animations and status utilities. Introduce clear select, pause, resume, and status
operations, preserving existing CLI behavior where possible. Share preview logic
and keep hardware-free tests.

**Done when:** one config can select/configure ripples or an existing mode, invalid
config leaves the last valid state intact, and transitions release mode-specific
resources. Mode selection no longer depends on unrelated execution paths.

### 3. Add the persistent engine and control API

**Implemented and mock-tested:** per-user engine/hardware leases, private Unix
IPC, persistent enabled/paused intent, live switching, bounded recovery, and
SIGINT/SIGTERM/SIGHUP cleanup. Commands never auto-retry or auto-resume on restart.
Physical reconnect/sleep and platform service acceptance remain unverified.

Add single-instance enforcement, local control, live switching, status reporting,
bounded recovery, and graceful service shutdown. Handle SIGTERM as well as Ctrl-C.
Preserve explicit pause across restarts and reconnects. Define how foreground
commands interact with a running engine.

**Done when:** CLI clients can switch/pause/resume a running engine without
restarting it; duplicate startup cannot create two writers; mocked failure tests
and physical lifecycle tests pass. Closing a client leaves runtime state intact.

### 4. Package for macOS and Linux

**Implemented tooling:** dependency-complete release builder, relocation,
staged installers, state-preserving upgrades/uninstall, opt-in LaunchAgent and
systemd-user helpers, and desktop launchers. Synthetic packaging/relocation tests
pass; reviewed SDK/license inputs, signed/notarized distribution, clean-machine
acceptance, and Linux hardware tests remain release gates. See [install](install.md).

Produce self-contained or dependency-complete release artifacts, user-service
definitions, installation instructions, and diagnostics. Replace checkout-based
SDK paths. Make login startup an explicit choice.

**Done when:** a clean supported machine can install, run, stop, restart, upgrade,
and uninstall without a development checkout or manually finding native SDKs.
Record hardware verification per supported platform, not just compilation results.

### 5. Add the thin GUI

**Implemented:** optional native settings controller using the same CLI/IPC,
preset/settings selection, explicit trusted-config import, pause/resume, engine
and login controls, and status/errors. Worker threads keep controller calls off
the UI thread; closing the client does not stop the engine. Build/headless tests
pass; graphical platform acceptance requires a display and manual validation.

Build the picker, settings, handoff, startup controls, and status surface on the
same control API used by the CLI. Keep headless operation supported.

**Done when:** a user can install once, select ripples, adjust brightness, and
return to normal lighting without opening a terminal. GUI/CLI changes agree and
persist; closing the settings window does not accidentally stop the engine.

### 6. Compose notifications over a base visualization

**Implemented and deterministic-test covered:** explicit command/CI/focus sources,
status filters, priority, selected zones, and 1–60 second expiration. Base input
continues advancing; stable events do not repeat. See [notifications](notifications.md).

Add opt-in build/CI/timer notifications with defined priority, duration, zone,
and resume behavior. Start with existing integrations rather than new providers.

**Done when:** a notification can interrupt or overlay our base effect and return
cleanly without a second hardware writer. A simple visualization-only setup stays
simple and makes no unnecessary network requests.

## Remaining acceptance and deferred work

The software pieces above are implemented and hardware-free checks pass. This
does not claim the physical/release completion criteria are satisfied:

- Exact Wootility coexistence restrictions: milestone 1 hardware evidence.
- Config and IPC currently use version 1; future versions need explicit migrations.
- The GUI uses optional eframe plus a macOS menu-bar launcher; Linux uses the
  desktop/settings-window fallback. Native interaction still needs manual review.
- Prepared archive/install tooling exists. Verified release SDK inputs, complete
  dependency notices, minimum-OS acceptance, signing/notarization, and distribution
  arrangements remain release gates. No publishing is performed by the tooling.
- Automatic Wootility handoff, additional layouts/models, light-bar behavior,
  real audio capture, screen ambilight, and multi-device support: later work after
  the core experience is reliable.

The older [ideas catalog](ideas.md) remains a source of effect concepts; its
historical implementation-status notes are not the current roadmap.
