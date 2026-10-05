# Underglow Ideas

Underglow is a data-driven RGB automation app for Wooting keyboards. A signal reads external state, turns it into a small status snapshot, and renders a lighting strategy on the keyboard while Wootility remains responsible for keyboard configuration and baseline profiles.

## Core vocabulary

- **Sources**: command output, GitHub, stock/market APIs, sports/racing APIs, calendars, local app context, audio, future analog input.
- **Signal state**: normalized values such as running/success/failure, price up/down, score changed, PR needs review, meeting active.
- **Rules**: logic mapping signal state to visual behavior.
- **Scenes**: RGB outputs such as pulse, sweep, comet, bloom, alert, heatmap.
- **Profiles**: named collections of sources, rules, and scenes.
- **Preview**: deterministic ANSI, JSON, or SVG output generated without opening the keyboard SDK.
- **Fixture/replay**: scripted source states for demos, tests, and visual reviews without network calls or child processes.

## Command Pulse

- Source: local command such as `make check`, `cargo test`, or `npm test`.
- State: pending, running, success, failure, timeout, interrupted.
- Scene: running sweep, green success hold, red failure hold, orange timeout, purple interrupt.
- Status: implemented first signal.
- Risks: child process cancellation, output policy, timeout defaults.

## GitHub / CI Beacon

- Source: GitHub Actions, PR reviews, issues, Graphite stack state.
- State: CI passing/failing/running, review requested, merge conflict, PR approved.
- Scene: function-row CI status, navigation-zone review alerts, red conflict pulse.
- Status: high-value next data integration after local command support.
- Risks: auth, API rate limits, polling frequency, private repo handling.

## Market Pulse

- Source: stock/crypto/watchlist APIs.
- State: market open/closed, ticker up/down, threshold crossed, volatility spike.
- Scene: green/red directional wave, dim market-closed idle, yellow threshold flash.
- Status: good proof of external API source abstraction.
- Risks: API keys, delayed data, rate limits, avoiding distracting constant updates.

## Sports / Racing Alerts

- Source: sports score APIs, F1/racing schedule/results/telemetry-like feeds where available.
- State: game started, team scored, lead changed, race session live, favorite driver event.
- Scene: team-color burst, checkered-flag sweep, sector-color pulses.
- Status: fun showcase integration.
- Risks: API availability/cost, event deduplication, team/driver color config.

## Focus Cockpit

- Source: local timer state: focus, break, overtime, meeting-safe dim mode.
- State: phase, remaining time, overtime.
- Scene: progress bar across function row or arrows; red overtime pulse; gentle break sparkle.
- Status: strong productivity signal candidate.
- Risks: continuous lifecycle, reset behavior, avoiding distraction.

## App Aura

- Source: manual profile switch now; future frontmost macOS application detection later.
- State: manual, IDE, terminal, meeting, game, recording, late-night.
- Scene: app-specific ambient palette or scene.
- Status: prototype implemented as a portable manual profile signal.
- Risks: macOS frontmost-app automation requires Accessibility permission and may need Automation consent for some app integrations. Linux/Windows need separate platform backends. Portable fallback is manual profile selection and requires no permissions.

## Soundwave Desk Toy

- Source: manual level/bass values now; future microphone or system audio levels later.
- State: disabled, volume, spectrum bands, bass pulse.
- Scene: spectrum bars, bass pulses, ambient color wash.
- Status: prototype implemented as an opt-in disabled-by-default signal.
- Risks: microphone/system-audio capture requires explicit OS permission, can increase CPU usage, and is platform-specific. Current prototype starts no capture by default and exposes a CPU-limit config placeholder.

## Analog Lava Lab

- Source: Wooting analog key pressure via future `wooting-analog-sdk_dist` backend.
- State: pressed keys and pressure values.
- Scene: pressure visualizer, typing heatmap, rapid-trigger trainer, analog game overlays.
- Status: roadmap stub added in `src/sdk/analog.rs`; runtime signal deferred until SDK behavior and matrix mapping are validated.
- Risks: SDK distribution, device permissions, concurrent RGB/analog access, and HID-keycode-to-RGB-matrix mapping.

## Visualization platform iteration

- Shared scene primitives now cover zone washes, progress bars, pulses, sweep trails, split fills, heatlines, and status colors.
- Offline previews can render effects and profiles as ANSI blocks, JSON summaries, or SVG contact sheets.
- Fixture/replay profiles make showcase states deterministic: build lane, CI stack, focus sprint, market heatline, sports burst, meeting/recording safe mode, and ambient app aura.
- Profile v2 runtime executes `[[sources]]`, `[[rules]]`, and `[scenes]` when no legacy `[signal]` override is present.

## Deferred sources

- Calendar/meeting detection should reuse profile scenes and stay permission-aware.
- System health and Graphite/Git stack sources should start as fixture-backed previews before live polling.
- Frontmost app automation, real audio capture, and analog pressure visualizers remain explicit opt-in work because they involve OS permissions or SDK contention.

## Engine direction

Rendering should stay deterministic where possible: `state + tick + layout + palette + brightness -> Frame`. The runner owns polling, timing, cancellation, logging, keyboard lifecycle, and restore policy. Profile runtime owns source snapshots, rule matching, scene priority, and zone-scoped composition.
