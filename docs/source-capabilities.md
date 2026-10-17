# Source capability descriptors (v1)

`underglow::source_capabilities` exposes declarative metadata for generic editors.
It does not modify the engine, executable connector trust, connection bindings,
or setup activation. Reading/importing metadata never starts producers, loads an
SDK, emits real sources, writes lighting, or resumes a paused engine.

## Schema

See [the built-in Pi fixture](../examples/pi-source-capabilities.toml). Required
fields are `schema_version = 1`, stable protocol `source`, `display_name`,
`instances`, `lifecycle`, `states`, and `events`. Empty fact lists are valid.
Each fact has an ASCII token `id` and human-readable `label`. States and events
are separate namespaces; a declared event is never automatically a state.

`instances.semantics` is `session` (unrelated ephemeral producer sessions) or
`producer-defined` (meaning not known to the editor). `selectors` is a nonempty,
unique subset of `any` and `exact`. In setup activation, missing `instance`
selects any; a supplied instance selects an exact protocol identity. Metadata
does not enumerate sessions or claim terminal/window focus.

The closed lifecycle vocabulary describes the existing external-v1 protocol:

| Field | Value and meaning |
| --- | --- |
| `presence` | `live-state-lease`: only currently leased state implies presence |
| `absence` | `no-live-state`: missing, expired, or removed; not idle or success |
| `expiry` | `at-deadline-no-synthetic-fact`: expires at the deadline, producing no state/event |
| `renewal` | `higher-sequence-state`: resend state with an increased shared sequence |
| `events` | `occurrence-only`: events neither create nor renew state/presence; no success implication |
| `removal` | `drops-state-keeps-events`: removal clears state, not queued events |

Optional `state_ttl` / `event_ttl` each contain `min_ms`, `max_ms`. Omission
declares no support for that fact kind; nonempty fact lists require a range.
A range with no named states can describe presence without promising vocabulary.
TTL is producer output metadata, not permission to change engine limits.
Optional `renewal_interval_ms` describes expected state heartbeat cadence, not
delivery guarantees, and must be positive and below minimum state TTL.
Events are transient and best-effort; metadata never promises an event is
currently available. Runtime consumers still expire their live registry and
handle event gaps according to the external protocol.

## Verified Pi facts

`pi_descriptor()` loads the fixture. Verified against
`connectors/pi/connector.mjs`, its lifecycle registration, and `src/external.rs`:

- `session_start`: new random instance, `idle` state.
- `agent_start`: `working` state.
- `agent_settled`: `idle` state, then `finished` **event only**, if previously working.
- Shutdown attempts removal; otherwise state lease expiry removes presence.
- State TTL is 15000 ms, renewed every 5000 ms. Event TTL is the remaining
  settlement window, 1..2000 ms, and is not retried after an attempted delivery.
- Settled means no further automatic continuation, **not task success**.
  There are no focus, success, or approval signals. No renderer encodes facts.

## Editor API and repair

`CapabilityRegistry::new()` / `default()` are empty. `with_builtins()` explicitly
registers Pi. `insert(SourceDescriptor)` validates before mutation and rejects
duplicate IDs/capacity overflow. `descriptors()` iterates descriptors in source
ID order; `descriptor(source)` looks up one. Connection registries own bindings
separately; a descriptor is not an installed/online connection or trusted code.

`choices(source, FactKind::{State,Event}, observed: &[String])` lists declared
choices first, in descriptor order, then de-duplicated optional observations.
Each choice has `id`, `label`, and `origin`: `Declared` or `UnknownObserved`.
Callers must supply observations scoped to that source and fact kind. Unknown
sources have no declared choices. Observations never mutate descriptors, become
guaranteed choices, or bypass activation diagnostics.

`validate_activation(&Activation) -> Result<Vec<CapabilityWarning>, String>`
first applies setup structural validation. Missing descriptors, undeclared
state/event names, unsupported instance selectors, and unsupported presence
produce **repairable warnings**, not destructive filtering. Both transition
states are checked. The original activation is never changed. A descriptor
cannot diagnose live source/session availability; an offline source remains a
valid selector and conditional layers remain hidden by runtime rules.
These checks are opt-in; the canonical setup validator remains source-neutral.

## Import bounds and trust

`SourceDescriptor::{from_toml,from_json}` bound input before parsing, reject
unknown fields at every nesting level, and validate before returning.
`to_toml()` validates and bounds output. Raw serde/direct construction requires
`validate()` before acceptance; registry insertion always validates too.

Limits: 64 KiB per descriptor, 32 registry sources, 128 states and 128 events,
128 observations per query, 128 bytes per ID/label, TTL range 1..300000 ms.
Tokens allow only ASCII letters/digits and `-_.:/`; labels must be nonblank and
contain no control characters. IDs are unique within each fact kind. Versions,
unknown fields/enum values, duplicate selectors, malformed ranges, and impossible
heartbeat cadence are errors. No arbitrary nested metadata or executable,
credential, trust, renderer, SDK, or physical activation fields are accepted.
Declarative metadata is not evidence that imported executable code is trusted.
