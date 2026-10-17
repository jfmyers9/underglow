# Managed connector processes

The engine can supervise explicitly trusted local executables that publish
[version 1 external messages](connectors.md#version-1-publishing-contract) as
newline-delimited JSON on stdout. This is **not a sandbox**: an executable runs
with your user privileges and inherited environment. Review its code, command,
arguments, and updates before granting trust. No shell is inserted.

## Configure and enable

Generate a [starter connector](connectors.md), review it, then add this to your
profile (replace the absolute paths):

```toml
effect = "matrix"

[[connectors]]
id = "focus-timer"
source = "focus"
command = ["/usr/bin/python3", "-u", "/absolute/path/my-focus/connector.py", "--stdout", "--duration-seconds", "60"]
enabled = false
trusted = false
restart_limit = 0 # one timed session, not a continuously restarted timer
```

Both flags default to false. After reviewing the producer, set `trusted = true`
in the profile, then explicitly select it and enable the connector:

```sh
underglow control select --config /absolute/path/profile.toml
underglow control connectors enable focus-timer
underglow control connectors status
underglow external inspect
underglow control connectors disable focus-timer
```

Enable cannot grant trust. `enabled=true` with `trusted=false` is a validation
error. Enable/disable persists to the engine's selected configuration snapshot,
not your original file. Re-selecting a file applies that file's flags again.
The CLI acknowledges the saved intent; poll connector status for completion of
asynchronous start/stop/reaping. Unknown IDs and extra control fields reject.
To rerun a completed one-shot connector, disable it, wait for `disabled` status,
then enable it. Rapid edits coalesce to the latest desired configuration.

Use absolute paths and an unbuffered/flushing producer. Arguments are literal
argv entries: no quoting expansion, `$HOME`, `~`, redirects, or pipes are
interpreted. Relative paths use the engine's working directory, which may differ
between development and an installed service. Executables inherit the engine's
environment; avoid embedding secrets in argv or configuration. Stdin is closed;
stderr goes to the null device, never an unbounded log or blocking reader.

## Lifecycle and lighting are independent

- Only `underglow engine` constructs the supervisor. Parsing, validation,
  `run --dry-run`, `run --preview`, and offline diagnostics never launch it.
  Foreground `run` does not supervise connectors; use the persistent engine.
- Explicitly enabled/trusted connectors start on engine startup and profile
  selection, **including while lighting is paused**. They observe facts, never
  resume lighting or acquire an SDK. Pausing lighting does not stop connectors.
- Disable, removal/change in a selected profile, engine stop, and SIGINT/SIGTERM
  shutdown terminate the owned process group, close its stdout transport, and
  reap its direct child. Unchanged connector configurations retain their PID
  across visual settings edits and unrelated connector changes.
- Cleanup uses immediate SIGKILL, not graceful completion hooks. A producer's
  final notification must never be inferred from termination. Remaining states
  expire by their finite leases (at most 300 seconds); Underglow does not delete
  a whole source and accidentally remove another publisher's instances.
- Each invocation must create fresh instance IDs and increasing sequences.
  Source must exactly match the configured `source`. At most one managed
  connector per source is allowed, but independent socket publishers can still
  use that source; this is not an exclusive/authenticated namespace.

After natural leader exit, remaining group descendants are killed, queued final
stdout facts are drained under the usual limits, and the direct child is
reaped. Replacement never overlaps the retiring producer. Process groups cover
normal descendants, **not a hostile executable deliberately detaching into a
new session/group**. Trust is mandatory; this is process lifecycle management,
not containment. Engine SIGKILL/crash cannot guarantee descendant cleanup.

## Supervision and resource limits

Only the explicitly opted-in connector argv is retried. This does not add retry
behavior to existing command-pulse/build/deploy sources.

| Setting | Default | Valid range |
| --- | --- | --- |
| `restart_initial_ms` | 1000 | 250–60000 |
| `restart_max_ms` | 30000 | initial delay–300000 |
| `restart_limit` | 8 | 0–32 retries after the initial attempt |

Exit (including exit 0), malformed/oversize/unauthorized output, and stream
failure stop the producer. Retries use capped exponential backoff. After 30
seconds of running with recent accepted traffic the consecutive failure budget
resets; lifetime `restarts` remains visible. Zero retries suits finite tasks.

Bounds are fixed: 16 connectors, 64 argv entries/16 KiB total per connector,
4096-byte JSON frames excluding newline, 32 frames per producer per worker tick,
100 frames and 64 KiB per second, and a one-second deadline for an incomplete
line. Stale/duplicate messages also consume the rate budget. Over-limit output
fails the producer rather than silently skipping facts. One 4 KiB read per
producer/tick and bounded buffers prevent floods from starving other producers.
No-newline output cannot grow memory without bound. Producers should normally
publish only transitions and low-frequency lease renewals.

A single worker owns process creation, nonblocking stdout, and child reaping;
the render/control loop never waits on producer I/O. Configuration replacement
uses one coalescing slot, not an accumulating command queue. Shutdown waits at
most two seconds per attempt and reports a timeout if the OS cannot reap an
already-killed child; no portable userspace code can force kernel-stuck children
to become waitable.

## Diagnostics

Control responses include a separate `connectors` array:

```json
{"id":"focus-timer","source":"focus","enabled":true,"trusted":true,"state":"running","running":true,"healthy":true,"pid":1234,"restarts":0,"retry_in_ms":null,"last_message_age_ms":120,"last_error":null}
```

`state` is `disabled`, `starting`, `running`, `stopping`, `backoff`, or `failed`.
`healthy` means the process is running and accepted a fresh protocol message in
the last 30 seconds—not that its reported task succeeded, its application lease
is still current, a rule matched, or lighting is enabled. Stale sequences do not
renew health. Use `external inspect` for application state/lease freshness and
`control diagnostics` for rule/scene visibility. A healthy process can have no
active application state; a failed process can leave a state awaiting expiry.

## Platform verification

Supported targets are macOS and Linux. Group creation uses the standard Rust
Unix `CommandExt::process_group(0)`. Before signaling an exited leader's group,
`waitid(WNOWAIT)` preserves its PID until the signal is sent, then standard
`Child::try_wait` reaps it; this avoids signaling an unrelated reused PGID.
The small FFI shim uses the documented macOS/Linux `P_PID`, `WEXITED`, `WNOHANG`
and platform-specific `WNOWAIT` constants. Its aligned, zero-initialized 256-byte
storage exceeds both platforms' `siginfo_t` sizes; it reads only the leading
`si_signo` integer, not a platform-specific union layout. Darwin constants were
checked against the local SDK `sys/wait.h`.

`cargo test --test managed_connectors` exercises fake Python producers in
isolated, hardware-disabled engines: trust/opt-in, inert dry-run/preview,
paused observation, bad/spoofed/flooded output, bounded retries, independent
enable/disable, profile replacement, final facts, descendants, and reaping.
These tests were executed on macOS; Linux constants follow its waitid ABI but
Linux execution still needs CI verification.
