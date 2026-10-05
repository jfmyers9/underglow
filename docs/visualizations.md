# Adding visualizations

Decorative effects use one hardware-independent implementation in the engine,
CLI preview, and GUI preview. The UI paints returned RGB values by LED address;
it does not implement a second animation or choose a lighting palette itself.

## Boundaries

| Location | Responsibility |
| --- | --- |
| `src/effects/<effect>.rs` | One effect's rendering, pace/period, and local tests |
| `src/effects/mod.rs` | Stable `EffectKind` IDs, dispatch, shared interpolation and contract tests |
| `src/catalog.rs` | Preset IDs/order, names, descriptions, renderer kind, defaults, controls, preview labels |
| `src/{device,layout,render,scenes}.rs` | Pure metadata, geometry, frame/palette types, drawing helpers |
| `src/{ripple,focus}.rs` | Stateful ripple simulation and pure Focus state rendering |
| `src/reactive/<effect>.rs` | Constellation, Heatmap, Afterimage rendering and decay |
| `src/reactive/mod.rs` | Fixed-size key state, input normalization, press-edge detection |
| `src/signals/`, `src/runner.rs`, `src/sdk/` | Live inputs, lifecycle, single hardware owner, device writes |
| `src/bin/gui/preview.rs` | Synthetic inputs/time and presentation of shared renderer output |

The library exports the pure types and renderers. `DeviceInfo` is data, not a
device handle; `DeviceInfo::synthetic_80he()` supplies common offline metadata.
No library renderer loads SDKs, spawns commands, polls providers, or owns a
wall clock. The application retains its explicit activation and shutdown paths.

## A new decorative effect

1. Agree on a stable kebab-case ID, defaults, palette support, and speed support.
   Display names can differ (for example, `rainbow` is shown as Spectrum).
2. Implement `pub(super) fn render(ctx: &RenderContext<'_>) -> Frame` in its own
   `src/effects/<effect>.rs`. Keep timing parameters and tests in that file.
   `comet.rs` is a small example; `interpolate_steps` is available for discrete
   cyclic effects. Continuous effects can render directly from
   `ctx.animation_seconds` without that helper.
3. Register the module, `EffectKind` variant, and dispatch arm in
   `src/effects/mod.rs`. Keep existing serde/clap IDs and the default variant.
4. Add one `Visualization` entry in `src/catalog.rs`, with
   `RendererKind::Static(EffectKind::YourEffect)`. The engine's preset allowlist
   and generated configuration, GUI cards/descriptions/controls, speed support,
   and preview renderer selection consume this catalog automatically.
5. Add semantic tests for the effect and run the shared checks below. Do not add
   GUI animation math, an SDK dependency, or new engine ownership logic.

Decorative effects receive **speed-adjusted elapsed seconds**, not an FPS-driven
frame count. `ctx.tick` remains for legacy/state-driven pulses and the internal
discrete interpolation sample. Respect brightness (including zero), layout
coordinates, and the device bounds; return a complete `Frame` each time.

Catalog defaults apply only to a newly selected preset or fresh engine. Existing
saved configurations keep their explicit values. Diagnostics such as `row-test`
are registered but excluded from everyday preset selection. Standalone CLI
subcommands retain their command-specific defaults for compatibility.

## Working in parallel

Assign one implementation file and its local tests to each contributor. Reserve
IDs and agree on the existing rendering contract first. Coordinate the small
registration edits in `effects/mod.rs` and `catalog.rs` through one integrator;
contributors should not each edit the GUI, engine, or shared capabilities lists.
Once a module is registered, the common tests and GUI preview discover it.
Timing or algorithm changes to an existing effect stay in its own file.

This is a compiled catalog, not a dynamic plugin system. New live-input/provider
types still need a `SignalProgram` adapter and their configuration plumbing.
Put their simulation/rendering in the pure library (Ripples and Focus are
examples), then deliberately add a synthetic preview path. Never initialize the
live adapter to obtain a preview. Existing provider scheduling, cancellation,
command trust checks, and hardware ownership remain separate and unchanged.

The three analog reactive modes share `src/signals/reactive.rs` and
`ReactiveSimulation::{advance,render,clear}`. Their decay uses elapsed input time,
never decorative speed. GUI input advances independently of the displayed FPS,
which samples a bounded copy of simulation state so short clicks are not lost.
Live SDK reads remain sampled at engine FPS. Only one analog source, including
Ripples, is permitted in a profile. Mode-specific configuration lives in
`[signal.reactive]` or `[sources.reactive]`; no input activity is serialized.

## Verification

```sh
cargo test --locked --lib
cargo test --locked --features gui
cargo clippy --locked --features gui --all-targets -- -D warnings
make test-dev
```

Shared contracts cover complete frame size, device bounds, brightness,
determinism, elapsed-time behavior, catalog coverage/identity, and GUI/frame
parity. Existing effects also have pre-refactor frame fingerprints, so moving
code cannot silently alter their output. GUI tests verify sampled RGB values,
canonical geometry, stable white legends, unsupported-mode behavior, and
synthetic Focus state without a window or device.

GUI previews are simulations, not hardware feedback: they use the shared 80HE
LED matrix rather than a different illustrative physical-key drawing. Focus
shows a fixed synthetic phase/progress; ripple input is synthetic. Unknown
profiles report that no preview is registered rather than guessing an effect.
