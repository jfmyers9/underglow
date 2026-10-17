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
whole-keyboard layer. Higher enabled, active layers blend over lower ones only on
their target keys. Default normal/255 replaces exactly as before. Black is a color,
not transparency; opacity zero is transparent. Uncovered keys are black
while the engine owns lighting. Pause releases lighting back to the keyboard.
An empty layer stack is valid and does not create an implicit background.

## Layer fields

Each layer has `id`, `name`, `renderer`, `target`, and optional `enabled` (true),
`brightness` (255), `opacity` (255), `blend` (`"normal"`), and `activation` (always).
Brightness and opacity independently use integers 0..255. Unknown blend modes,
out-of-range opacity and unsupported fields are rejected, including on disabled
layers. Older documents without these fields render unchanged.

### Blend contract

`blend` is one of `"normal"`, `"add"`, `"multiply"` (case-sensitive strings).
Composition is bottom-to-top, channel-linear integer RGB, **not gamma-corrected**.
For destination `d`, source `s`, opacity `a`, define `R(n) = floor((n+127)/255)`:

* Normal: `R(d*(255-a) + s*a)` (alpha replacement).
* Add: `min(255, d + R(s*a))` (saturating addition).
* Multiply: `R(d*(255-a) + R(d*s)*a)` (rounded multiply, then alpha interpolation).

Every mode at opacity 0 leaves the destination unchanged. Normal at 255 is exact
replacement, including opaque black; at 128 black dims the advancing underlying
effect, rather than freezing it. Add black is neutral; multiply black at 255
produces black. Multiply over an uncovered black destination remains black.
Targets and activation eligibility apply before blending; lower animations and
facts continue advancing even when obscured. The low-level compositor preserves
signed priority ordering and first-declaration-on-top for equal priorities.
Canonical setups give higher stack indices higher priority.

Logical-layer opacity/blend are shared by every behavior variant; they are not
renderer brightness or per-appearance brightness overrides. Save/Discard,
library persistence and import/export retain these declarative fields without
granting executable connector trust or activating hardware.

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
and renderer, and shared blending compositor. `composition(ctx, elapsed)` exposes
ordered active contributions with masks and stack-index priorities;
`compositor::winning_layer` identifies the top nonzero-opacity eligible layer
(not necessarily the only contributor). `contributing_layers` reports structural
contributors after opaque normal replacement. RGB coincidences, saturation and
rounding do not claim structural obscuration. `render(ctx, elapsed)`
produces one fresh RGB frame, with uncovered keys and non-key cells black.
Global `ctx.brightness` is a safety multiplier: effective brightness is
`layer.brightness * ctx.brightness / 255`, applied once. Layer palette/speed
settings (or catalog palette defaults) are authoritative. After blending, final
RGB channels are clamped to `ctx.brightness` by the same `compose_capped` helper
for live output and coverage/preview capture. This is a ceiling, not a second
multiplier: additive layers cannot exceed global safety brightness and default
normal frames are not double-dimmed.

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

### Isolated source scenarios

**Scenario** supplies private external-v1 facts to that same draft evaluator and
compositor, with no keyboard, Pi session, provider, or live engine required. The
source picker and state/event buttons come from declared source capabilities,
not renderer names. Pi offers Idle, Working, and Finished (an occurrence, not
success). Custom descriptors offer their own vocabulary. Disconnected means
remove/expire presence, never an invented `disconnected` state.

Choose a session identity (default `scenario-session`), TTL, and a state or event.
Change source or identity and publish another state to compose independent
channels; the picker never clears other channels. **Edit session** selects a
listed state/event identity for further edits or removal. Any-instance conditions
aggregate sessions, exact conditions select one. The bounded simulated-facts list
shows state leases and retained event occurrences, not invented process health.
State does not renew automatically. Events neither renew state
nor imply successful completion. Removal drops state but preserves leased events
according to the existing evaluator semantics. Descriptor TTL ranges describe
real producer output; test TTLs use the registry's bounded 1..300000 ms range.
Invalid identifiers/TTLs are rejected without changing synthetic facts, leaving
the input editable for repair.

The clock starts **paused**. **Play** advances local monotonic time; **Pause**
freezes at the last evaluated instant, including source expiry, transient
deadlines, and every animation. **+100 ms**, **+1 s**, or **Advance ms** step all
channels synchronously without sleeping. A successful step auto-pauses and drops
unevaluated wall-clock drift. Resuming never catches up paused time. Steps cannot
move time backwards; only reset returns to zero.

The Scenario badge remains visible. **Reset scenario** pauses and clears synthetic
facts, sequences, event cursors and deadlines. **Return to Live** does the same and
baselines the latest real snapshot without replaying historical completions.
Save/Discard/reload also baseline event history. Real `LiveSample` snapshots and
event batches are retained unchanged and never mixed into Scenario. Isolated
input demo remains a separate mode; scenario facts do not imply fake keypresses.
No scenario action publishes to IPC, writes the engine, changes saved drafts or
hardware-enabled intent, starts a producer, acquires SDK ownership, or resumes
lighting. Physical activation still requires explicit Resume.

`underglow::scenarios::Scenario` is a pure private `external::Registry` wrapper.
It exposes `publish_state`, `publish_event`, `remove`, typed `apply(Message)`,
`advance_to`/`advance_by(Duration)`, `now`, `limits`, `snapshot`, non-draining
`events_since(cursor)`, `generation` (reset identity), and `reset`. Its own
sequence generator is shared across all message kinds and source/session
identities, independent of live producers.
Manual typed messages use schema 1 and the same protocol bounds. Time is injected
and cannot go backwards; errors do not consume sequences or mutate facts. Each
preview owns its own registry and compiled runtime. This library supports multiple
independent sources/sessions for deterministic tests and the scenario controls.

Preview's `evaluation_now`, `evaluation_elapsed`, `evaluation_brightness`, and
`evaluation_facts` expose the last rendered evaluation inputs; `sample()` always
remains the unchanged real-only transport, even while Scenario is selected.

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
`name`, `renderer`, `brightness`, `opacity`, `blend`, `target`, `activation`, and
`variants`, never identity.
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
