# Canonical lighting setup format (v1)

`underglow::setup::SetupDocument` is a hardware-independent, typed document.
This defines the format; it does not yet replace the legacy application config
or activate lighting. See [the example](../examples/layer-setup.toml).

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
