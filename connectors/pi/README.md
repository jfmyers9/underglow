# Pi → Underglow connector

Reports **session present**, **working**, **idle**, and a brief **finished** event
to the local Underglow engine. Rules and scenes—not this extension—choose the
colors and target keys. It does not touch the RGB SDK or resume paused lighting.

Requires Pi with `agent_settled` support (validated against installed Pi 1.1.0),
macOS or Linux, and Node 22.19+ as required by Pi. No npm dependencies or build step.

## Load it

From this repository, start a new Pi process:

```sh
pi --extension "$PWD/connectors/pi/index.ts"
```

For automatic loading in future sessions, copy the **whole directory** (including
`connector.mjs`, not just `index.ts`) to `~/.pi/agent/extensions/wooting/`, or add a
directory symlink there. Only install extensions you trust: they run with your
user's permissions. Do not load this connector twice in one Pi process.

The engine must already be running with external ingress enabled. Pi remains
usable if it is stopped; the connector silently reconnects when it returns.

### Which engine?

Discovery, in order:

1. `WOOTING_PI_SOCKET`: connector-only override for the exact socket path.
2. `WOOTING_STATE_DIR`: engine state directory; append `events.sock`.
3. macOS: `~/Library/Application Support/wooting-signals/runtime/events.sock`.
4. Linux: `$XDG_STATE_HOME/wooting-signals/events.sock`, falling back to
   `~/.local/state/wooting-signals/events.sock`.

For `make dev` simulation, copy the **state directory printed at startup** and
launch Pi in a separate terminal with that directory:

```sh
WOOTING_STATE_DIR=/tmp/wsdev-UID-CHECKOUT/simulation \
  pi --extension "$PWD/connectors/pi/index.ts"
```

Replace the placeholder path; it is not a literal working example. The watcher
does not export its child environment to other terminals. Hardware development
also uses an explicit state directory; use the printed path rather than assuming
the installed engine's default. An exact socket override works for either mode:

```sh
WOOTING_PI_SOCKET=/actual/engine/state/events.sock \
  pi --extension "$PWD/connectors/pi/index.ts"
```

## Lifecycle and delivery

| Pi hook | Published fact |
| --- | --- |
| `session_start` | New random instance, `idle` state |
| `agent_start` | `working` state |
| `agent_settled` | `idle` state, then `finished` event |
| `session_shutdown` | Best-effort instance removal |

`finished` means Pi is no longer continuing automatically. It **does not mean
task success**. `agent_end` is intentionally ignored because retries, compaction,
or follow-ups can continue after it. Duplicate settle notifications while idle
do not create another event. Terminal/window focus is not detected.

- Each running session gets an unrelated UUID; two terminals do not overwrite
  each other. No session filenames or historical session IDs are sent.
- State leases last 15 seconds and renew every 5 seconds. A killed/crashed Pi
  disappears by lease expiry. Engine restart restores the current state on the
  next heartbeat/reconnect, never historical state transitions.
- There is one in-flight request, one coalesced pending state, and at most one
  pending completion. Socket requests have a 500 ms hard deadline, replies are
  capped at 4 KiB, and reconnect backs off from 250 ms to 5 seconds. Timers and
  sockets are unreferenced and cannot keep Pi alive by themselves.
- Completion expires two seconds after settlement, with its remaining lifetime
  sent as `ttl_ms`. Once attempted it is **never retried**, even if its ACK is
  lost. This intentionally favors a missed flash over duplicate/stale completion.
- Quit, reload, and session replacement stop renewal, cancel pending work, and
  attempt removal once. Shutdown waits at most 250 ms for that attempt, then
  destroys resources. Removal failure falls back to lease expiry.
- All message kinds share a strictly increasing safe-integer sequence. An ACK
  of `ignored_stale` is terminal, not a retry instruction. At the safe-integer
  limit the connector disables itself rather than wrapping; lease expiry cleans
  up that practically unreachable case.

The wire message contains only version, `source: "pi"`, random instance,
sequence, and a fixed lifecycle payload. **No prompts, responses, tool arguments,
commands, working directories, model names, or other session content are read or
published.** There is no network listener, subprocess, or telemetry.

## Diagnose and test

Inspect the matching engine's registry, supplying the same state directory if
needed:

```sh
./target/debug/underglow external --state-dir /actual/engine/state inspect
```

A live Pi instance should report `idle`, then `working` during a run, then `idle`
again after final settlement. Events are transient; inspecting after their TTL
has elapsed will not show the completion. Source presence alone does not make
lighting visible: enable a matching rule/scene and explicitly resume lighting.

Hardware-free tests (Node 26 used during implementation):

```sh
node --test connectors/pi/connector.test.mjs
```

Tests use a fake Pi event emitter and real temporary Unix-socket servers. They
cover lifecycle/privacy, multiple sessions, reload/replacement, renewal, missing
engines, restart recovery, event expiry, lost/stale ACKs, bounded delivery,
malformed replies, safe sequences, and idempotent resource cleanup. They do not
launch a real Pi model run or access a keyboard.
