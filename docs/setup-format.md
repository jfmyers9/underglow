# Canonical lighting setup format (v1)

`underglow::setup::SetupDocument` is a hardware-independent, typed document.
It can be embedded as optional `[setup]` in the application config; when absent,
legacy signals/profiles are unchanged. Reading either format never activates
lighting. See [the document example](../examples/layer-setup.toml) and
[a runnable profile](../examples/layers-profile.toml).

## Identity and ordering

Required fields are `schema_version = 1`, `id`, `name`, and `layers`.
`revision` defaults to zero for optimistic editing; loading does not increment it.
IDs are explicit, stable tokens, independent of names, file paths, or position.
Layer IDs must be unique within a setup. Names need not be unique.

`layers` is ordered **bottom to top**. A background is an ordinary always-on,
whole-keyboard layer. Higher enabled, active layers replace lower ones only on
their target keys. Black is opaque, not transparency. Uncovered keys are black
while the engine owns lighting. Pause releases lighting back to the keyboard.
An empty layer stack is valid and does not create an implicit background.

## Layer fields

Each layer has `id`, `name`, `renderer`, `target`, and optional `enabled` (true),
`brightness` (255), and `activation` (always). Brightness uses 0..255 channels.

Renderer tables use `kind`:

| Kind | Settings |
| --- | --- |
| `effect` | Existing `effect` enum, optional `palette`, `speed` (100; 10..400) |
| `solid` | RGB `color` |
| `pulse`, `sweep` | RGB `color`, `period_ms` (1000; 1..60000) |
| `ripples` | Optional `palette`, or both `base_color` and `ripple_color` |
| `reactive` | `effect` (`constellation`, `heatmap`, `afterimage`), optional `palette` |
| `focus` | `focus_minutes` (25), `break_minutes` (5), `cycles` (1), `start_paused`, `meeting_safe`, `dim` (all false) |

Missing palettes use catalog defaults. Fixed-color effects (including Matrix)
reject explicit palettes. Row test rejects nondefault speed. Two-tone ripples
reject an explicit palette or an incomplete color pair. Focus durations are
bounded to 1..1440 focus minutes, 0..1440 break minutes, and 1..100 cycles.
RGB values require exactly three 0..255 integers. Unsupported fields are errors,
not silently ignored settings. `Renderer::catalog_id()` connects built-in
renderers to the existing shared visualization catalog.

Targets use `kind = "whole"` or `kind = "selection"`. Selections combine `zones`,
physical `keys`, and named `groups` by union. Empty selections are invalid, never
implicitly whole. Zones are `function`, `alpha`, `navigation`, `arrows`, `system`.
Top-level `groups` maps group IDs to nonempty, flat lists of physical key names.
Named keys reuse the 80HE ANSI mapping (`F1`, `ArrowUp`, `W`, etc.), not OS text
or remappings. Fn, light bars, arbitrary coordinates, nested groups, and other
keyboard layouts are not implied by this schema. Layout validation remains a
separate runtime responsibility before applying targets to a device.

Activation tables also use `kind`:

- `always`: no selectors.
- `while-state`: `source`, optional `instance`, optional `state`.
- `on-event`: `source`, optional `instance`, `event`, `duration_ms`.
- `on-transition`: `source`, optional `instance`, optional `from_state`, `state`,
  `duration_ms`.

Transient durations are 1..60000 ms. Missing instance matches any session;
missing while-state state means source presence. Source IDs are external fact
selectors, not executable definitions. Missing/offline sources are allowed and
hide conditional layers. Capability checking and variant behaviors can extend
this neutral activation/appearance split without connector-specific renderers.
Editors can use the separate [source capability registry](source-capabilities.md)
for declared choices and repairable activation warnings; canonical setup
validation remains source-neutral and retains unknown/offline selectors.

## Validation and safe replacement

`SetupDocument::from_toml()` validates before returning; `to_toml()` validates
before serialization; `replace_from_toml()` assigns only after parsing and
validation succeed, preserving the last valid setup after errors. Public serde
types also support JSON; callers using raw serde must call `validate()` before
accepting the document. Text input should be bounded before raw deserialization.

Limits: 256 KiB document text, 64 layers, 64 groups, 128 keys per selector/group,
5 zones and 64 group references per selection. IDs/selectors are at most 128
bytes of ASCII letters/digits or `-_.:/`; display names are nonempty, at most
128 bytes, and contain no control characters. Disabled layers and unused groups
are also validated. Unknown versions, fields, groups, and keys are rejected.

This data contains no SDK paths, executable definitions, connector trust,
network actions, or physical activation settings. Reading/importing a setup
cannot start a connector or take over keyboard lighting.

## Shared runtime and engine integration

`underglow::layer_runtime::CompiledSetup::compile(&document)` validates data and
resource combinations without acquiring hardware. At most one **enabled**
analog-input layer (ripples or reactive) is currently supported, including across
conditional layers; disjoint masks do not create separate SDK ownership. Disabled
layers still undergo document validation. Physical names and analog renderers
require an 80HE ANSI layout, checked again against the live device.

The runtime reuses the catalog effects, reactive/ripple simulations, focus timer
and renderer, and shared opaque compositor. `composition(ctx, elapsed)` exposes
ordered active contributions with masks and stack-index priorities;
`compositor::winning_layer` resolves per-key overlap. `render(ctx, elapsed)`
produces one fresh RGB frame, with uncovered keys and non-key cells black.
Global `ctx.brightness` is a safety multiplier: effective brightness is
`layer.brightness * ctx.brightness / 255`, applied once. Layer palette/speed
settings (or catalog palette defaults) are authoritative.

Pass **unscaled monotonic elapsed** time to `render`/`composition`, not the
legacy global speed-adjusted `ctx.animation_seconds`. Each decorative layer
applies its own speed. The binary adapter keeps a separate unscaled clock; focus
durations, conditions and input simulation are never multiplied by global speed.

`advance_input(dt, keys)` accepts in-memory HID pressure samples;
`preview_tick(tick)` supplies a deterministic 100 ms demo, while
`preview_tick_at(tick, dt)` accepts a caller's FPS/time step. Each preview owns a
separate runtime; demo samples never enter the live external registry.
`baseline(snapshot, now)` and `observe(snapshot, events, now)` reuse the existing
AutomationRuntime lease/event/transition semantics. Resume discards historical
events and transitions but retains currently leased persistent state. Missing
sources hide only their conditional layers. Registry failure clears conditions;
input failure hides only input-dependent contributions until input recovers
(the live adapter retries SDK initialization only on the next explicit resume).

`run --config examples/layers-profile.toml --preview --preview-format json`
uses the same pure compiler/renderer as live runs, without either SDK or provider
initialization. Ordinary config previews have no live facts. Engine `control
preview` uses an independent canonical runtime with read-only live facts and
synthetic input, including while paused; it does not resume lighting or start
producers. Legacy notifications and automation remain optional overlays.

Live setup programs initialize input only in the existing session lifecycle and
never write RGB themselves: the session remains the sole RGB writer. Engine
pause closes that session, restoring/releasing keyboard lighting and clearing
input/transient state. Hardware-free tests cover lifecycle state and composition;
real SDK restoration still requires a physical-device smoke test.
