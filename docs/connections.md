# Machine-local connections (v1)

Connections are independent of lighting setups. One stable protocol source ID
identifies one connection. Setup `Activation.source` references that ID; no
producer command, credentials, trust flag, or process identity belongs in a
`SetupDocument`.

The engine owns `connections.json` beside `state.json`, in its private state
directory. Writes use a new 0600 file, sync, atomic rename, and directory sync.
An existing registry always wins on restart, including an explicitly empty one.
Malformed, oversized, nonprivate, and symlink registry files fail closed.

## Read and explicitly edit

```sh
underglow control connections show
underglow control connections show | jq '.connections.registry' > /private/path/connections.json
# Review/edit the private file; keep its revision unchanged.
underglow control connections replace --expected-revision 0 < /private/path/connections.json
```

Use a private location (0600) for exports: the full registry can contain local
argv secrets. Do not distribute it as a setup or capability manifest. The
replacement is an explicit **machine configuration** action and may authorize
and start executable code with your user privileges, even while lighting is
paused. Review executables before setting `producer.trusted=true`. Trust defaults
false and enabling an untrusted producer is rejected. Metadata-only imports
must supply no producer, and cannot install or authorize an executable.

The submitted full JSON has this shape:

```text
{
  "schema_version": 1,
  "revision": 0,
  "connections": [
    { "descriptor": <SourceDescriptor>, "producer": <optional ConnectorConfig> }
  ]
}
```

A [SourceDescriptor](source-capabilities.md) is required. Omitting `producer`
means the source is externally published, not that an executable should be
discovered or installed. The initial Pi descriptor is external-only. For a
reviewed managed connector, add its descriptor and the machine-only producer:

```json
{
  "id": "focus-timer",
  "source": "focus",
  "command": ["/usr/bin/python3", "-u", "/absolute/path/connector.py", "--stdout"],
  "enabled": false,
  "trusted": false,
  "restart_initial_ms": 1000,
  "restart_max_ms": 30000,
  "restart_limit": 0
}
```

The containing descriptor's `source` must also be `focus`. Producer IDs and
managed sources must be unique. After explicit review and a registry replacement
setting trust true, `control connectors enable focus-timer` starts it;
`disable` stops it. Both legacy commands write the same registry and increment
its revision, never a separate profile copy. Refresh the registry after any edit.

Each replacement requires matching expected and submitted revisions, increments
the stored revision once, validates everything before persistence, and only then
updates supervisor intent. Stale or malformed edits leave registry, setups,
enabled lighting intent, and producers unchanged. Input is strict JSON bounded
to 256 KiB; at most 32 connections and 16 managed producers are allowed.

## Legacy migration and visual selection

On first engine startup without a registry, existing local `AppConfig.connectors`
migrate once, preserving commands and trust/enable flags explicitly set by the
user. Trusted enabled producers may continue as before. Unknown legacy sources
get descriptors with no invented fact vocabulary. The saved visual profile is
then stripped of its connector table; the registry becomes the source of truth.
Crash recovery prefers an already committed registry over an old profile.

**Breaking change:** `control select --config profile.toml` now changes visuals
only. Connector definitions in that file are ignored with a JSON `warning`,
not installed, trusted, enabled, disabled, or restarted. To change connections
after initial migration, use the explicit registry workflow above. Original
input files are not rewritten. Setup replace/edit, visual profile selection,
preview, and reads do not update supervisor intent. Unchanged producer configs,
including descriptor-only edits, preserve the current PID.

Missing referenced connections are reported in `connections.missing_sources`;
layers and their original selectors remain intact and repairable. An offline
source is not a reason to delete a layer. Replacements never resume lighting.

## Editor hooks and runtime health

The public `underglow::connection_registry` module exposes `ConnectionRegistry`,
`Connection`, `ConnectorConfig`, bounded parsing/validation, CAS replacement,
`managed_configs()`, `capability_registry()`, and `missing_sources(setup)`.
Capability exports contain descriptors only, never commands or trust.

Protocol actions are `connections-show` and `connections-replace` with
`expected_revision` and `registry`. Responses include `connections.registry`,
`connections.runtime_index` keyed by stable source, and `missing_sources`.
Each index entry has `kind` (`managed` or `external`) and `producer` (the
existing supervisor health record or null). Health is asynchronous; null for
external sources is not evidence of offline status, and accepted traffic is
not task success. Use `external inspect` for live facts and lease freshness.
