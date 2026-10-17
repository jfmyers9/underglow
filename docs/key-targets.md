# Named keys and custom groups

Automation scenes can target the whole keyboard, legacy zones, individual
physical keys, or reusable groups. Matrix (or the selected background) keeps
advancing underneath; an overlay only replaces its selected keys.

## Configuration

```toml
[automation.groups]
WASD = ["W", "A", "S", "D"]
agent-strip = ["F1", "F2", "F3", "F4"]

[automation.scenes.agent-a]
keys = ["F1"]
color = [0, 80, 255]

[automation.scenes.focus]
groups = ["WASD"]
kind = "pulse"
color = [255, 160, 0]

[automation.scenes.combined]
zones = ["arrows"]
keys = ["Escape"]
groups = ["agent-strip"]
color = [0, 255, 64]
```

- `zones`, `keys`, and `groups` form a **union**, not an intersection.
- All three absent/empty means whole keyboard, preserving existing profiles.
  `keys = ["F1"]` with no zones selects only F1, never the whole keyboard.
- Duplicate/overlapping selections light each coordinate once. Usual per-key
  priority applies; the first declared rule wins equal-priority conflicts.
- Group definitions are flat key lists, not nested group references. Group names
  are case-sensitive tokens (letters, numbers, `-_.:/`), up to 128 bytes.
- Key names are ASCII case-insensitive (`F1` and `f1` match). Ranges and shortcuts
  such as `F1-F4`, `WASD`, and `Ctrl+C` are not key names; enumerate a group.
- At most 64 groups, 128 entries per group, 128 direct keys per scene, and
  64 group references per scene. Empty groups, unknown keys, and missing group
  references are configuration errors, even when no rule currently matches.

See [the complete Matrix example](../examples/automation-key-groups.toml).

## One key per session

Use a rule's existing explicit `instance` selector, rather than automatically
allocating keys based on session arrival order:

```toml
[[automation.rules]]
id = "pi-session-a"
source = "pi"
instance = "sessionA"
mode = "while-state"
state = "working"
scene = "agent-a"

[[automation.rules]]
id = "pi-session-b"
source = "pi"
instance = "sessionB"
mode = "while-state"
state = "working"
scene = "agent-b"

[automation.scenes.agent-b]
keys = ["F2"]
color = [255, 0, 120]
```

This matches the **exact identity published by the connector**. It does not
rename a Pi session or discover which terminal has focus. Use actual connector
instance IDs in your rules, or a producer configured to publish stable IDs.
An expired/removed instance releases its key back to the current background.
Omitting `instance` aggregates all instances of the selected source as before.

Test F1 without hardware or a running engine:

```sh
cargo run -- diagnostics preview --config examples/automation-key-groups.toml \
  --source pi --instance sessionA --state working
```

## Supported physical map

Named selectors currently require **80HE ANSI, 6 rows × 17 columns**. ISO, JIS,
split/unknown layouts, different dimensions, and other keyboard models are
rejected by live device validation, even for currently inactive named scenes.
Whole-keyboard/zone-only profiles retain their existing behavior.

Supported names:

- `A`–`Z`, `0`–`9`, `F1`–`F12` (enumerated individually in config).
- `Escape`, `PrintScreen`, `Pause`.
- `Grave`, `Minus`, `Equal`, `Backspace`, `Tab`, `BracketLeft`,
  `BracketRight`, `Backslash`, `CapsLock`, `Semicolon`, `Quote`, `Enter`,
  `ShiftLeft`, `ShiftRight`, `Comma`, `Period`, `Slash`, `Space`.
- `Insert`, `Home`, `PageUp`, `Delete`, `End`, `PageDown`.
- `ArrowUp`, `ArrowLeft`, `ArrowDown`, `ArrowRight`.
- `ControlLeft`, `MetaLeft`, `AltLeft`, `AltRight`, `ControlRight`.

These identify physical ANSI positions, not characters produced by your OS
layout or a Wootility remap. Fn/custom namespaces, right Meta, numpad keys, and
the light bar are deliberately unsupported. The SDK's column-16 `SCR MODE`
cell has a model-dependent legend (Mode on 80HE), so neither `ScrollLock` nor
`Mode` is exposed as a named selector. Positions share the typing map
with reactive input and follow the upstream RGB matrix diagram (F1 is at
row 0, column 2; matrix gaps are not compressed). No hardware verification is
implied by offline rendering tests.

For library callers, validate `AutomationConfig`, call `validate_layout` before
rendering to a device, and use `scene_mask` to resolve groups. Standalone
`Scene::mask()` supports direct keys/zones only and fails closed to an empty
mask for unresolved groups or invalid keys. `AutomationRuntime` caches fully
resolved masks for both rendering and diagnostics.
