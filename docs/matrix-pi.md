# Matrix + Pi: a function-row status strip

[The starter profile](../examples/matrix-pi.toml) keeps Matrix as the background
and reserves the **80HE ANSI function zone** for Pi:

| Reported activity | Function zone |
| --- | --- |
| No leased Pi session | Matrix, like the rest of the keyboard |
| At least one session open | Dim cyan |
| Any session working | Blue traveling sweep |
| A session finishes | Green sweep for up to two seconds, then current state |

With several sessions, working wins over idle. A completion briefly wins over
both, even if another session is still working; afterward blue returns if needed.
Sweeps paint dark between their moving lights **inside their zone**. The typing,
navigation, arrows, and system zones retain Matrix. Global brightness scales both
the background and overlays; this profile starts at 180/255.

```text
Pi lifecycle hooks → connector → private events.sock → leased source registry
    → profile rules → zone scenes → compositor over Matrix → RGB SDK → keyboard
```

Only the engine writes RGB. The connector reports facts, not colors. It never
resumes lighting, and it doesn't inspect terminal text or detect window focus.
`finished` means Pi reached `agent_settled` and will not continue automatically,
**not that the task succeeded**. No prompts, responses, tool arguments, commands,
or working directories are sent.

## 1. Build and preview without hardware

**Recommended development flow:** run `make dev`, then run `pi` in another
terminal in this checkout. Development state is seeded/upgraded automatically,
Pi discovers the running engine, and the main GUI preview shows live source
overlays while hardware remains disabled. Existing animations/settings and
explicit Pi rules are preserved. Restart old watcher/Pi processes once after
updating; Pi's normal project-extension trust prompt may appear. For keyboard
output use `make dev DEV_HARDWARE=1`, then explicitly Resume lighting.

The remaining steps describe standalone engines and isolated diagnostics.

Run these from the repository root. Rust/Cargo is required. For the connector,
use Pi with `agent_settled` support (validated against Pi 1.1.0) and its supported
Node version (22.19+). There is no connector npm install or build step.

```sh
cargo build --bin underglow
./target/debug/underglow diagnostics preview \
  --config examples/matrix-pi.toml --source pi --state idle
./target/debug/underglow diagnostics preview \
  --config examples/matrix-pi.toml --source pi --state working --elapsed-ms 400
./target/debug/underglow diagnostics preview \
  --config examples/matrix-pi.toml --source pi --state idle --event finished
./target/debug/underglow diagnostics preview \
  --config examples/matrix-pi.toml --source pi --state idle --event finished --elapsed-ms 2000
```

These emit JSON containing key colors and rule visibility. They do not contact
an engine, load SDKs, execute providers, or publish synthetic sessions. The last
command shows the expired completion with the cyan indicator restored. For plain
Matrix with no matching source, change `--source pi` to `--source unconfigured`.

For a visual offline view, build the GUI (`cargo build --features gui --bins`),
open **Connector diagnostics**, enter the absolute path to this TOML, source
`pi`, and state `idle` or `working`. Optionally enter event `finished`, then click
**Preview synthetic source**. This preview is separate from the live registry;
it doesn't make the keyboard follow a real Pi session.

## 2. Select the profile on one paused engine

Use one current engine, not a second writer alongside an installed engine or
another development watcher. If a current Underglow engine is already running,
use it and skip the start command below. Otherwise, in terminal A:

```sh
# Foreground; stays running. No hardware access while paused.
./target/debug/underglow engine --paused
```

In terminal B, from the same repository root:

```sh
./target/debug/underglow control pause
./target/debug/underglow control select --config "$PWD/examples/matrix-pi.toml"
./target/debug/underglow control diagnostics
```

The explicit selection matters: `engine --config` only seeds **new** state;
saved settings win on subsequent starts. After editing the TOML, select it again.
Selection and pause are persistent. No login startup is enabled by these steps.

All clients must address the same engine state directory. By default that is
`~/Library/Application Support/wooting-signals/runtime` on macOS, or
`$XDG_STATE_HOME/wooting-signals` (fallback `~/.local/state/wooting-signals`) on Linux.
If you set `WOOTING_STATE_DIR`, use the same value in every terminal. Keep socket
paths short, private to your user, and on a local filesystem.

## 3. Connect an actual Pi session

In terminal C, from the repository root:

```sh
# For a standalone engine, explicitly select its actual private socket.
WOOTING_PI_SOCKET=/actual/engine/state/events.sock \
  pi --extension "$PWD/connectors/pi/index.ts"
```

In terminal B, inspect without resuming lighting:

```sh
./target/debug/underglow external inspect
./target/debug/underglow control diagnostics
```

Look for a `pi` instance initially `idle`, then `working` during a prompt, then
`idle` after final settlement. Paused diagnostics report **potential** rule
matches, not applied lighting or a replay of completions. Each Pi process has an
independent random identity. Heartbeats renew 15-second leases every 5 seconds;
normal shutdown attempts removal, and a killed terminal expires within a lease.
After engine restart, current state returns on renewal/reconnect; old completion
events are not replayed.

For future sessions, copy or symlink the **whole** `connectors/pi` directory into
`~/.pi/agent/extensions/wooting/`. Don't copy only `index.ts`, and don't also load
the command-line extension if it is already auto-loaded. Extensions execute with
your user permissions. Install only trusted code. The local socket is a
same-user trust boundary, not authentication of whether a real Pi process is
running. See [connector lifecycle and discovery details](../connectors/pi/README.md).

### Using `make dev` instead

The default `make dev` is hardware-disabled simulation with live source overlays
in the main keyboard preview. No profile selection, copied socket path, or Pi
extension flag is needed: run `pi` from this checkout. The project-local extension
reads private engine discovery that the watcher updates after successful starts.
It waits if development is stopped, rather than falling back to installed
lighting. Existing Pi sessions reconnect; completion events from a previous
engine are not replayed.

The watcher still prints its state directory for optional CLI diagnostics:

```sh
# Replace /actual/engine/state with the printed path, not a guessed default.
export WOOTING_STATE_DIR=/actual/engine/state
./target/debug/underglow external inspect
```

`WOOTING_PI_SOCKET=/actual/engine/state/events.sock` overrides only Pi's
destination. It takes precedence over `WOOTING_STATE_DIR`, which also overrides
automatic development discovery. Don't leave stale overrides pointing at
another engine. Hardware development is
explicit (`make dev DEV_HARDWARE=1`); see [development ownership and SDK setup](dev.md).

## 4. Explicit physical-keyboard opt-in

**Do this only when ready to take over RGB.** Have a single mapped 80HE ANSI and
the RGB SDK available to the engine process; Matrix needs no Analog SDK. See the
[RGB SDK setup](../README.md#quick-start). If using an explicit
`WOOTING_RGB_SDK_PATH`, set it in the terminal **before starting the engine**.
An already-running engine doesn't inherit later shell environment changes.

Stop other Underglow engines and competing RGB writers yourself. State-directory
overrides are not permission to run multiple hardware writers. The engine cannot
arbitrate ownership with third-party RGB applications.

```sh
./target/debug/underglow control resume
./target/debug/underglow control diagnostics
# When finished: restore/release lighting, even while Pi remains open.
./target/debug/underglow control pause
# To shut down the foreground engine as well:
./target/debug/underglow control stop
```

Resume should show Matrix outside the function zone and the current Pi indicator
inside it. Opening Pi or publishing while paused never resumes lighting. Events
received while paused are not replayed on resume. Closing a GUI window does not
stop a normal persistent engine; explicitly pause or stop it.

## Customize the display

### Editable layer template (no TOML)

In **Layer composition**, select **Add Pi template**. This appends **one logical
layer** above your existing layers, initially targeting the function zone; it
does not replace Matrix, Aurora, custom targets, or other layers. Use the target
canvas, zones, keys or groups to move all Pi appearances together. Stack order is
bottom to top, so place it above whichever supported background you prefer.

The base/fallback appearance is dim cyan while a Pi session is present. In
**Behavior appearances**, select Working to change its blue sweep or Finished
to change its green sweep and two-second event duration. Colors, renderers,
brightness, sweep periods and conditions are ordinary editable data. The
finished event means **settled, not success**; there are no approval/focus facts.
By default each condition selects any session. To bind one exact session, choose
its instance in the base condition and both appearance conditions; exact IDs can
be ephemeral. Any working session beats idle; a completion briefly wins over
working. Presence and variants disappear on source lease expiry/removal.

Delete Working or Finished independently, or delete the whole Pi layer. Saving
and restarting never restores these defaults. A new blank composition has no Pi.
Fresh `make dev` starter state uses the same single-layer template once; existing
canonical setups (even empty ones) are authoritative. Only the exact, untouched
bundled Pi trio is folded during one-time development migration. Other legacy Pi
rules retain independent migrated layers rather than being guessed into a
template. No source TOML is rewritten.

Adding/editing/previewing is hardware-free and never starts a connector or
changes executable trust. Save while paused, then explicitly Resume only when
ready for physical output through the single engine writer.

### Legacy standalone profile

Edit `[automation.scenes.pi-*]` in a copy of the starter, then select that file.
`zones = ["function"]` targets a subset. Other named zones are `alpha`,
`navigation`, `arrows`, and `system`; multiple zones form a union. `zones = []`
(with no other targets) covers the whole keyboard, so a Pi scene could replace
Matrix globally instead. Priorities are per key: 30 completion > 20 working >
10 presence. Unrelated navigation overlays can coexist; higher-priority scenes
cover only their overlapping keys. Equal-priority automation rules resolve in
declaration order. This starter doesn't require individual-key/custom-group
configuration. The light bar has no dedicated mapping.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| No Pi instance | Extension loaded once? Correct `events.sock`? Engine current and running? Check `WOOTING_PI_SOCKET` before `WOOTING_STATE_DIR`. |
| Pi remains usable but no indicator | Expected if engine unavailable: connector retries silently. Inspect the registry to separate delivery from rendering. |
| Instance present, no matching rule | Select this profile explicitly; saved state may contain another profile. Inspect source/state and rule IDs in diagnostics. |
| Rule matches but keyboard unchanged | Is engine paused, in simulation, or reporting an SDK error? `lighting_applied=false` is not a hardware result. Explicit resume is required. |
| Active rule not visible | Check `obscured`, priority, and visible/targeted key counts. A higher-priority overlapping scene can cover it. |
| No completion flash | Did Pi reach final settlement? It is a brief at-most-once event, not success detection. Paused/disconnected engines may deliberately miss it. Heartbeats don't retrigger it. |
| Stuck working after closing terminal | Allow up to 15 seconds from the last renewal after a crash. A second Pi session may still be working. |
| TOML edits do nothing | Reselect with `control select --config`; engine startup config doesn't overwrite saved selection. |
| Wrong areas light up | This mapping is for 80HE ANSI. Preview is not proof of physical key alignment; complete the checklist below. |

## Verification and pending physical acceptance

Hardware-free checks:

```sh
cargo test --test matrix_pi
node --test connectors/pi/connector.test.mjs
```

The Rust integration test needs `cc` and Node. It drives the production Pi
connector with fake lifecycle hooks through the real engine/socket/rules and
captures frames in a fake RGB SDK with **no HID/vendor SDK linkage**. It checks
multiple sessions, completion expiry, Matrix preservation/advancement, competing
zones, pause/resume, restart/reconnect, normal removal, and a killed connector's
real 15-second lease. Offline preview checks compare Matrix outside the function
zone exactly. These tests don't run an actual Pi model conversation or establish
physical appearance/restoration.

**Physical 80HE visual acceptance is pending.** No hardware or installed services
were accessed for this feature's automated verification. Before marking it fully
accepted, a person should record date, OS, firmware, SDK, and observations:

- [ ] With no Pi session: Matrix covers the mapped keyboard.
- [ ] Open Pi: cyan appears only on the intended function keys; confirm Escape,
      function-row boundaries, other zones, and lack of unintended light-bar changes.
- [ ] Submit a prompt: blue travels visibly; Matrix keeps moving outside the zone.
- [ ] Final settlement: a readable green sweep returns to cyan, not a success claim.
- [ ] Two sessions: idle doesn't override working; finishing one briefly flashes,
      then another working session returns to blue.
- [ ] Close normally, then kill a separate session: removal/lease expiry restores
      the expected current state, with no permanently stuck indicator.
- [ ] Pause while Pi works: Wootility lighting is visibly restored; subsequent Pi
      activity doesn't retake RGB. Resume shows current state without old flashes.
- [ ] Restart the engine paused: no takeover; Pi reconnects; explicit resume works.
- [ ] Exercise a second zone notification and an overlapping higher-priority alert;
      confirm independent display and expected conflict/restoration behavior.
- [ ] Pause/stop restores lighting visibly and Wootility remains usable. A fake
      SDK's successful close acknowledgement is not evidence of visual restoration.
