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
Pulse is a smooth cosine from 64/255 to full intensity over one period. Sweep
sorts target keys by physical x/y and moves a cyclic four-key fading trail over
one period, retaining opaque black pixels elsewhere in the target. These reuse
legacy scene appearances (`period_ms = 2000` for legacy pulse; target key count
times 100 ms for legacy sweep).

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

The GUI's **Behavior appearances** form edits the selected logical layer's base
condition and its appearances without TOML or rule IDs. Choose Always, While a
source matches (presence or state), When an event occurs, or When a state changes.
Source metadata supplies state/event choices and supported any/exact instance
selectors. Optional live observations are marked **unknown/observed**, never
promoted to supported metadata. Missing or undeclared saved selectors remain
visible with repair warnings; simply opening the form does not rewrite them.
Exact IDs can be typed or chosen from observations and may name ephemeral
producer sessions, not windows or focus. Transient durations are 1..60000 ms.

Select **Base / fallback appearance** to choose whether the base remains visible
offline (Always) or hides with source absence/expiry (While source is present).
Appearances share the layer target and can inherit its brightness or explicitly
override it. Changing an appearance's renderer preserves brightness inheritance.
Add, rename, enable, delete and change appearance priority in the same form;
these are draft edits governed by the existing Save/Discard controls. No form
starts a connector, grants executable trust, or resumes physical lighting.

## Logical behavior variants

`variants` defaults to an empty list (omitted when saving an old setup). Each
strict variant has a stable `id`, editable `name`, `enabled` (default true),
`activation`, `renderer`, and optional `brightness`. IDs are unique **within the
logical layer**, not across the setup. Variants inherit the layer's target;
brightness overrides, rather than multiplies, the layer's value. Disabled
variants are validated too. There are at most 16 variants per layer, within the
same 256 KiB input/output document limit; transient durations remain 1..60000 ms.

Exactly one appearance contributes per logical layer per frame:

1. Active `on-event`/`on-transition` variants outrank persistent variants.
2. Within either class, the **last** active variant in declaration order wins.
3. If none matches, the original layer renderer is a fallback, gated by the
   original layer activation. Disabling a layer hides all its variants.

An absent instance selector means **any matching live instance**. Put idle
before working so any working session beats any number of idle sessions.
Source names and facts remain generic; no renderer/runtime knows about Pi.
An always-on fallback deliberately remains visible offline. For a status layer
that hides offline, use source-presence activation on its fallback as below.
`Always` variants likewise deliberately remain visible without a source.

Each transient variant has one bounded occurrence slot: a newer matching
occurrence (from any session) replaces its previous occurrence, not a queue.
Event IDs/cursors are deduplicated by the existing registry/evaluator. Deadlines
are computed from receipt/transition time, capped by event/transition lease;
delayed observations and state heartbeats never restart them. State-backed
variant events are additionally capped by the emitting session's lease at
observation and hide if that session disappears, even if another session is
still connected. Event-only producers use their event lease, without inventing
persistent state. Expiry is exclusive: an appearance hides at its deadline.
Transitions need two observed, contiguous states of the same session; first
appearance, expiry, removal, and same-state renewals are not transitions.
Resume baselines all variants, discards historical events/transitions and
restores only currently leased persistent state. Registry failure clears all
conditional appearances. There is no stale-appearance hold or disconnect latch.
Legacy standalone automation event lifetime semantics are unchanged.

For example, one editable Pi status item over an arbitrary background:

```toml
schema_version = 1
id = "agent-status"
name = "Agent status"

[[layers]]
id = "background"
name = "Background"
renderer = { kind = "solid", color = [8, 8, 16] }
target = { kind = "whole" }

[[layers]]
id = "sessions"
name = "Agent sessions"
brightness = 180
renderer = { kind = "solid", color = [0, 0, 80] }
target = { kind = "selection", keys = ["F1", "F2"] }
activation = { kind = "while-state", source = "pi" }

[[layers.variants]]
id = "idle"
name = "Idle"
activation = { kind = "while-state", source = "pi", state = "idle" }
renderer = { kind = "solid", color = [0, 40, 100] }

[[layers.variants]]
id = "working"
name = "Working"
activation = { kind = "while-state", source = "pi", state = "working" }
renderer = { kind = "sweep", color = [0, 100, 255], period_ms = 1000 }

[[layers.variants]]
id = "finished"
name = "Finished"
activation = { kind = "on-event", source = "pi", event = "finished", duration_ms = 2000 }
renderer = { kind = "pulse", color = [0, 255, 80], period_ms = 600 }
```

The same status item can target any selection or the whole keyboard, and any
catalog renderer can serve as its background or appearance. Mutually exclusive
reactive/ripple variants share one logical input-resource budget; two enabled
logical input-consuming layers are still rejected, even on disjoint targets.
The engine remains the only SDK owner/RGB writer. Compilation, selection, and
preview add no acquisition, producer, trust, or automatic-resume path.

`CompiledSetup::selected_appearance(layer_id)` exposes the selected variant ID
(`None` for fallback), activation, renderer, effective local brightness, and
occurrence start/deadline. An outer `None` means hidden. This is pre-occlusion
data for diagnostics/editors; it does not claim a higher stack layer left keys
visible. `elapsed` is the conditional appearance's evaluator time minus its
activation start; always-on appearances instead use setup elapsed time. Call
`observe` on monotonic registry time to update selection, animation and expiry.

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
applies its own speed. Conditional appearances use unscaled time since activation,
not setup age. The binary adapter keeps a separate unscaled clock; focus
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

## GUI drafts and composition preview

The layer editor keeps an applied baseline separate from its local draft.
**Discard draft** restores that baseline. **Save composition** submits the complete
draft with the baseline engine revision; a rejected/stale save retains the draft,
baseline, and error. Status refreshes never overwrite dirty work. Saved documents
use the same canonical persistence format and survive restart.

Save is available only while the connected engine reports paused lighting. It
never resumes lighting: physical activation remains the separate explicit Resume
action. Editing, Discard, and composition preview send no lighting commands.

The full keyboard preview owns an independent `CompiledSetup` and uses its shared
renderer/compositor, including opaque black, stack order, layer targets, palette,
speed, and brightness capped by global brightness. **Live facts** inspects only
the private source endpoint in a bounded background worker; it does not start
providers or read physical input. **Isolated input demo** supplies private demo
keystrokes to the same Focus/reactive/ripple simulations and never mixes in live
facts or publishes synthetic sources. These modes never acquire an SDK lease.
Conditional state expires on the extrapolated engine clock. Load, Save, Discard,
mode changes, stale recovery, and endpoint generation changes baseline event
cursors so old completions cannot replay. Ordinary scoped edits preserve unchanged
runtime state through `CompiledSetup::reconfigure`.

## Atomic engine edits

`control setup show` returns `setup: {revision, document}` without migration or
activation. Legacy profiles return revision zero and a null document. Use
`control setup replace --expected-revision 0 < setup.json` to install a complete
declarative draft. Imported revisions are ignored: successful edits/replacements
increment the current local revision even when the setup ID changes.

`control setup edit --expected-revision 0` reads a JSON operation array from stdin:

```json
[{"op":"update","id":"matrix","patch":{"renderer":{"kind":"effect","effect":"aurora","speed":100}}}]
```

Operations are `create {layer,index}`, `update {id,patch}`, `remove {id}`,
`duplicate {id,new_id,index}`, `enable {id,enabled}`, and `reorder {ids}`.
Indices and reorder IDs are **bottom to top**; reorder must include every ID
exactly once. Duplicate requires a caller-supplied fresh ID. Patches accept
`name`, `renderer`, `brightness`, `target`, and `activation`, never identity.
Replacing a renderer supplies its complete typed settings, not a partial
renderer object. One transaction supports 1..128 operations; stdin is bounded
to 256 KiB. The outer application config remains bounded to 256 KiB too.

Socket protocol v1 actions are `setup-show`, `setup-edit` with `expected_revision`
and `edits`, and `setup-replace` with `expected_revision` and `document`. Successful
responses include the resulting `setup` object. Stale revisions, unknown IDs,
invalid data/resource combinations, live-device incompatibility, and persistence
errors reject the entire candidate before changing saved or running state.

Only `[setup]` changes: connector executables/trust, sources, notifications, and
global settings remain unchanged. Validation and compilation finish before an
atomic synced-file rename; the engine then adopts the precompiled candidate
without SDK reacquisition or provider initialization. Same-ID unchanged renderers
retain input simulations; unchanged activation conditions retain original
deadlines. New/changed conditions baseline current state without replaying old
events. A post-rename directory-sync failure is reported as a durability warning,
not a rejected edit after the file has already committed.

Live analog-input requirement changes and adopting a setup into a legacy live
program require explicit Pause first. Edits never Resume, and paused edits and
previews never acquire hardware ownership. Runtime adapters forward the canonical
edit hook through notifications/automation without restarting unrelated commands.
