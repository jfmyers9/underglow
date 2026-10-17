# Pi → Underglow connector

Reports **session present**, **working**, **idle**, and a brief **finished** event
to the local Underglow engine. Rules and scenes—not this extension—choose the
colors and target keys. It does not touch the RGB SDK or resume paused lighting.

Requires Pi with `agent_settled` support (validated against installed Pi 1.1.0),
macOS or Linux, and Node 22.19+ as required by Pi. No npm dependencies or build step.

## Load it

From this repository, start the development session and launch Pi in another
terminal in the same checkout:

```sh
make dev
# In another terminal, from this checkout:
pi
```

Pi auto-loads `.pi/extensions/underglow/index.ts` after the normal project-trust
approval. There is no global install, path copying, environment export, or
`--extension` argument. Already running Pi sessions need `/reload` once to load
the new project extension. Pi may start before `make dev`; it reconnects when
the development engine is ready. Use `make dev DEV_HARDWARE=1` for the real
keyboard and explicitly resume lighting. Simulation uses the GUI preview only.

### Outside this checkout

To load the generic connector explicitly:

```sh
pi --extension /absolute/path/to/connectors/pi/index.ts
```

For automatic loading outside this project, copy the **whole directory**
(including `connector.mjs`, not just `index.ts`) to
`~/.pi/agent/extensions/wooting/`, or add a directory symlink there. Only install
extensions you trust: they run with your user's permissions. No installer or
development command changes that directory. The generic adapter uses the
installed engine's normal discovery paths and silently retries if unavailable.

### Which engine?

Discovery, in order:

1. `WOOTING_PI_SOCKET`: connector-only override for the exact socket path.
2. `WOOTING_STATE_DIR`: engine state directory; append `events.sock`.
3. **Project-local adapter:** private checkout-specific `make dev` metadata,
   dynamically following simulation/hardware mode and engine restarts. If dev
   is inactive or the descriptor is invalid, **do not connect to any engine**.
   This adapter never falls back to the installed engine.
4. **Generic adapter only, macOS:**
   `~/Library/Application Support/wooting-signals/runtime/events.sock`.
5. **Generic adapter only, Linux:** `$XDG_STATE_HOME/wooting-signals/events.sock`, falling back to
   `~/.local/state/wooting-signals/events.sock`.

The development watcher publishes a private `current.json` under
`/tmp/wsdev-UID-CHECKOUT_HASH/` only after its engine is ready and removes it
before replacement/shutdown. Discovery checks the real checkout, expected mode
directory/socket, live supervisor/engine PIDs, ownership, permissions, file type,
and bounded descriptor size. A new engine nonce discards stale completion events
and republishes only the current state, preserving the Pi instance identity.

Explicit overrides intentionally bypass automatic dev discovery. For example:

```sh
WOOTING_PI_SOCKET=/actual/engine/state/events.sock \
  pi
```

Clear old `WOOTING_PI_SOCKET`/`WOOTING_STATE_DIR` exports to restore automatic
development discovery. Loading both current generic and project adapters still
creates one publisher: they arbitrate through Pi's runtime-scoped event bus,
with project discovery winning regardless of load order. Previously copied
older connector versions should be updated or disabled to avoid duplicates.

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
node --test connectors/pi/*.test.mjs
```

Tests use a fake Pi event emitter and real temporary Unix-socket servers. They
cover lifecycle/privacy, multiple sessions, reload/replacement, renewal, missing
engines, restart recovery, event expiry, lost/stale ACKs, bounded delivery,
malformed replies, safe sequences, and idempotent resource cleanup. They do not
launch a real Pi model run or access a keyboard.

Discovery tests additionally cover Pi-before-dev, descriptor removal, mode and
nonce changes, stale PIDs, malformed/oversized descriptors, unsafe permissions,
symlinks, explicit overrides, and duplicate adapter registration using real
temporary Unix sockets.
