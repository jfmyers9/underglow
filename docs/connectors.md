# Custom connectors

A connector reports facts. User rules select scenes; the engine alone writes to
the keyboard. A new source does **not** require a Rust provider or SDK access.
The [Pi connector](../connectors/pi/README.md) is a lifecycle integration; the
starter below demonstrates an independent, timed **focus** source.

## Generate and run a starter

From the repository root, using Python 3 on macOS or Linux:

```sh
python3 scripts/scaffold-connector.py --language python --name focus --capabilities --output /tmp/my-focus
python3 /tmp/my-focus/connector.py --stdout --duration-seconds 2 > /tmp/focus.ndjson
python3 scripts/check-connector.py /tmp/focus.ndjson
```

For TypeScript, use Node **22.19+** with native type stripping; no npm install,
TypeScript compiler, or packages are needed:

```sh
python3 scripts/scaffold-connector.py --language typescript --name focus --output /tmp/my-focus-ts
node /tmp/my-focus-ts/connector.ts --stdout --duration-seconds 2 > /tmp/focus-ts.ndjson
python3 scripts/check-connector.py /tmp/focus-ts.ndjson
```

Choose new output paths. The scaffold refuses **any existing destination**,
including empty directories and symlinks. A source name must be 1–128 ASCII
letters, digits, or `-_.:/`; it is a label, not a filesystem path.
Generated files are a single readable producer, a fixture, and a README. No
service is installed or enabled. Templates live in [`connectors/templates/`](../connectors/templates/).

### Use the source in the GUI without Rust changes

Add `--capabilities` to either scaffold command to generate `capabilities.toml`.
This optional strict v1 [manifest](source-capabilities.md) declares only the
starter's actual `focusing` state and `done` event, session identity, selectors,
TTL and renewal semantics. It declares no success, focus-window, idle or Pi
vocabulary. When adapting the producer, edit its manifest to match its facts.

1. Open **Connections**, refresh, and expand **Import custom capability manifest**.
   Select TOML and enter the generated file path (or paste its contents), then
   validate and **Import metadata into Connections**. A source-ID collision
   requires explicit metadata-only replacement acknowledgment.
2. The source now appears in generic layer behavior forms with its declared
   states/events. Choose the source, activation, target and appearance, preview
   locally, and explicitly save the composition. No Rust registration is needed.
3. To run the connector, separately review the generated executable. Configure
   its exact argv in Connections, save the reviewed disabled producer, then
   explicitly enable it (see [Connections](connections.md)). Or explicitly run
   it with the private socket path above. Importing a manifest never runs code.
4. Lighting stays paused until the separate **Resume** action. Producer enable,
   metadata import, binding repair and preview never resume it.

Manifest import is bounded, declarative data only. Never share a private full
connection-registry export as a manifest: it can contain secrets and executable
authorization. The metadata importer rejects all such fields rather than
silently stripping or trusting them.

Both producers expose the same arguments:

| Argument | Meaning |
| --- | --- |
| `--stdout` | Emit NDJSON messages, with no socket connection or ACK |
| `--socket PATH` | Publish directly to this explicit private `events.sock` |
| `--duration-seconds N` | Timed focus session, default 60; finite range 0–86400 |

Exactly one transport is required. Direct publishing to an **already running**
development engine looks like:

```sh
python3 /tmp/my-focus/connector.py --socket /actual/engine/state/events.sock --duration-seconds 60
```

Replace the path with the state directory printed by `make dev`, plus
`events.sock`. These starters deliberately do not guess the installed engine's
path or launch/resume an engine. Development simulation is sufficient.

Each invocation creates a fresh random UUID, publishes `focusing`, renews every
5 seconds with a 15-second lease, emits `done` at normal completion, and sends
`remove`. Ctrl-C/SIGTERM removes without completion. A crash expires by lease.
Delivery is serialized, replies are limited to 4 KiB, and socket attempts have
a 500 ms absolute deadline. Errors exit nonzero after one bounded removal
attempt; these intentionally small starters **do not reconnect or retry events**.
For a persistent integration, republish current state after reconnect/engine
restart rather than replaying old completions. See Pi for a bounded reconnect
implementation.

`--stdout` is useful for offline checks and supervised process readers. Keep it
exclusively NDJSON; diagnostics go to stderr. Pipe writes have a deadline, but
Node's synchronous terminal/file writes cannot be interrupted by JS timers; use
a pipe for managed streaming. Running the example by hand is explicit trust,
not sandboxing. The publishing protocol is the same regardless of transport.

## Version 1 publishing contract

Authoritative implementation: [`src/external.rs`](../src/external.rs) and
[`src/ingress.rs`](../src/ingress.rs). The envelope has **exactly** these fields:

```json
{"schema_version":1,"source":"focus","instance":"fresh-random-uuid","sequence":1,"payload":{"kind":"state","state":"focusing","ttl_ms":15000}}
```

`payload` is nested—not flattened—and accepts exactly one of:

```json
{"kind":"state","state":"focusing","ttl_ms":15000}
{"kind":"event","event":"done","ttl_ms":2000}
{"kind":"remove"}
```

- `schema_version` is integer `1`. Unknown envelope/payload fields are rejected.
- `source`, `instance`, and state/event labels are nonempty ASCII tokens using
  letters, digits, or `-_.:/`, each at most 128 bytes. No arbitrary metadata or
  free-text contents are accepted in v1.
- `sequence` is an unsigned 64-bit integer, strictly increasing across **all**
  kinds for each `(source, instance)`. Zero is valid for the first message.
  JavaScript producers should stay within `Number.MAX_SAFE_INTEGER` and stop
  before wrapping. Use a new random instance per process/session, never a fixed
  shared ID or an identity from an offline fixture.
- A state update replaces and renews that instance's state. `ttl_ms` must be an
  integer from 1 through 300000 under current defaults. TTL starts at receipt
  using the engine's monotonic clock; no producer timestamp is needed.
- An event is an occurrence, not automatically success. It does not create or
  renew state. Its TTL bounds pending relevance; rules also configure visual
  duration. Do not emit events on every heartbeat. Do not retry an event after
  an ambiguous delivery: the ACK may have been lost after it was applied.
- Removal clears state, not already accepted events. It still advances sequence.
  Stale/equal sequences do nothing, including no lease renewal. Bounded sequence
  tombstones are retained for at least 300 seconds beyond accepted message
  deadlines, so deduplication is not permanent.
- Facts are ephemeral. Engine restart clears all states/events. Default registry
  bounds are 32 sources, 128 active instances, 256 tracked identities, and 256
  queued events; full identity/state capacity rejects, full event queue drops
  oldest. Producers must handle rejection rather than grow unbounded queues.

### Socket framing and acknowledgments

Use a local Unix stream socket. Open **one connection per message**, send one
UTF-8 JSON object (at most 4096 bytes) followed by `\n`, read one newline-delimited
reply, and close. Do not send several frames on one connection. Bound your
connection/write/read lifetime and reply bytes; the engine also enforces bounds.
Success replies are:

```json
{"schema_version":1,"ok":true,"outcome":"applied"}
{"schema_version":1,"ok":true,"outcome":"ignored_stale"}
```

`ignored_stale` is not an instruction to retry. It usually signals identity or
sequence reuse; these starters fail rather than continue misleadingly. Rejection:

```json
{"schema_version":1,"ok":false,"error":{"code":"invalid_ttl","message":"..."}}
```

For scripts, the existing CLI is another publishing option (same contract):

```sh
./target/debug/underglow external --state-dir /actual/engine/state state \
  --source focus --instance fresh-random-id --sequence 1 --state focusing --ttl-ms 15000
./target/debug/underglow external --state-dir /actual/engine/state inspect
```

The example ID above is a placeholder; generate a new one in real automation.
`external publish` accepts **one** JSON message on stdin, not an NDJSON stream.

## Map facts to keyboard scenes

Merge this into the engine TOML configuration, preserving existing automation
rules/scenes. It works with Matrix or any selected base effect:

```toml
[[automation.rules]]
id = "focus-active"
mode = "while-state"
source = "focus"
state = "focusing"
scene = "focus-strip"
priority = 10

[[automation.rules]]
id = "focus-done"
mode = "on-event"
source = "focus"
event = "done"
scene = "focus-finished"
priority = 20
duration_ms = 1500

[automation.scenes.focus-strip]
kind = "pulse"
color = [0, 120, 255]
zones = ["function"]

[automation.scenes.focus-finished]
kind = "sweep"
color = [0, 255, 100]
zones = ["function"]
```

Omitting `instance` matches any live instance of the source. Multiple focusing
sessions keep the strip active until the last matching lease ends. A heartbeat
does not restart a persistent animation. `while-state` has no duration; event
and transition rules require `duration_ms` from 1 to 60000.

Named zones are `function`, `alpha`, `navigation`, `arrows`, and `system` on
mapped layouts. Empty/omitted `zones` targets the **whole keyboard**. Higher
priority wins on overlapping keys; equal priorities use first declaration order.
Disjoint indicators coexist. Matrix keeps advancing beneath them and becomes
visible when they end. Global brightness and pause still apply. Publishing
cannot resume paused lighting; explicitly resume through normal engine controls.

## Fixture checks and verification

```sh
python3 scripts/check-connector.py connectors/examples/fixtures/valid.ndjson
python3 scripts/check-connector.py /tmp/my-focus/fixture.ndjson
python3 -m unittest scripts/tests/test_connectors.py -v
make test-connectors
```

`make test-connectors` runs the Pi lifecycle/discovery and starter-kit tests. It is an
optional target requiring Node 22.19+ and Python 3; `make check` is unchanged and
does not require Node.

The checker uses only Python's standard library. It bounds input lines and checks
v1 exact fields, tokens, integer types/ranges, default TTL, duplicate fields, and
strict per-identity sequencing. It rejects blank/missing-newline records and
limits an offline fixture to 10000 messages and 256 identities. Invalid examples
in [`connectors/examples/fixtures/`](../connectors/examples/fixtures/) exercise
flattened payloads, extra fields, TTL overflow, and sequence reuse.

**This is a fixture linter, not a replacement for Rust validation.** It does not
model clock/lease expiry, tombstone collection, live capacity, rule evaluation,
or transport admission. It rejects stale sequences rather than acknowledging
them, and rejects duplicate JSON keys conservatively. Actual acceptance is
determined by the engine. Tests generate both starters, run their output through
the checker, and publish to real temporary fake-server sockets. No installed
engine, Pi session, SDK, or physical keyboard is touched.

## Trust and privacy

- Read connector code before executing it. Python/Node producers have your full
  user permissions; the local socket is not an executable sandbox.
- The socket is user-private, not source-authenticated. Programs under the same
  user can claim another source label. Do not treat lighting as a security signal.
- Do not expose the socket over TCP or loosen its permissions. Keep publication
  separate from engine control: it cannot select profiles, execute commands, or
  resume paused hardware.
- Publish lifecycle/status labels only. Never send prompts, command arguments,
  secrets, terminal contents, or paths. Keep queues and retries bounded.
- Generation and fixture validation do not imply trust or enable automatic
  execution. A process supervisor must require explicit opt-in independently.
