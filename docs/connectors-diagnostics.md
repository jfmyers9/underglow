# Connector diagnostics

Inspect the source → rule → scene pipeline without resuming lighting:

```sh
cargo run -- control diagnostics
# Separate development instance:
cargo run -- control --state-dir /absolute/test-state diagnostics
```

The JSON response keeps the existing `status` payload and adds `diagnostics`:

- `instances`: source/instance identities, state, sequence, update age, and lease
  time remaining in milliseconds at capture. `available` means an unexpired
  reported state; it does not prove process liveness or terminal focus.
- `events`: currently retained, unexpired occurrences. Retention does not imply
  an event's scene is still active.
- `rules`: matched/active status, scene, priority, expiry, targeted and visible
  key counts. `obscured` means some targeted keys lose compositor arbitration.
  Black pixels still occupy keys: visibility describes layer ownership, not
  measured hardware brightness. Live reports include competition with legacy
  profile/notification overlays.
- `evaluation=live`: actual running program history, including transient
  activations. It is not reconstructed by replaying the event buffer.
- `evaluation=potential`: no active hardware session. Current while-state rules
  are evaluated on a synthetic 80HE layout. Event/transition history is not
  replayed. `lighting_applied=false` means these are potential layers, not a claim
  that LEDs changed. Engine `status` distinguishes paused, retrying, and error.

Diagnostics do not tick providers, run commands, acquire SDKs, alter enabled
state, or consume another reader's event cursor. Source state and the latest
rendered rule history can differ briefly between frames; this is a read-only
snapshot, not an instruction to render another frame.

## Isolated synthetic previews

```sh
cargo run -- diagnostics preview --config examples/matrix-pi.toml \
  --source pi --instance preview --state working
cargo run -- diagnostics preview --config examples/matrix-pi.toml \
  --state idle --event finished --elapsed-ms 500
```

Use a profile containing `[automation]` rules/scenes. The commands above use an
example path; replace it with your profile's actual location. Each invocation
creates a private in-memory registry, injects the requested state and optional
event, and samples the real evaluator/compositor at `--elapsed-ms` (0–60000).
Synthetic messages have 60-second leases. JSON includes diagnostics plus an
80HE key map with RGB values.

Only the profile's configured base **effect** is rendered. Signal providers,
commands, legacy notification polling, and real input are deliberately not run.
No socket is contacted and nothing is published into the live engine. Preview
does not need a running engine or SDK. A transition rule requires previous-state
history and therefore will not fire from this single-state fixture.

## GUI

Expand **Connector diagnostics** on the lighting page. **Refresh connector
diagnostics** uses the existing bounded background subprocess worker; it never
blocks rendering. Captured data is timestamped, and disconnected data is marked
**Last seen**. Lease ages are frozen at capture, not presented as live counters.
The exact JSON is available in a nested collapsed section.

Enter a profile path and synthetic source/state/event, then click **Preview
synthetic source**. This works offline and while lighting is paused, displays a
colored keyboard map, and never requires the trusted-profile execution checkbox
because it only deserializes configuration and renders pure effects. To apply a
profile to real lighting, use the separate explicit profile import controls.
