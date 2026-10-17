# Legacy setup migration

Engine startup and explicit profile selection adapt supported legacy snapshots
to canonical setups. Only the engine's private saved snapshot is written; the
user's original TOML is never rewritten. A canonical `setup` table (even an empty
stack) is authoritative and makes migration idempotent.

Static effects, Focus settings, reactive effects, ripple palettes/two-tone colors,
and external automation rules become a base layer plus conditional layers.
Automation groups, targets, selectors, transient durations, colors and priority
are retained. Equal-priority rules are reversed in the stack so the legacy first
declaration still wins. Legacy Pulse uses a 2000 ms cosine cycle; Sweep retains
its four-key trail at ten keys per second. The global brightness remains the
safety ceiling; base layer brightness is 255 to avoid applying it twice.
Sweep periods are calculated from the supported 80HE target mask. Cross-device
layout adaptation is not promised by this migration.

Connectors, credentials, executable commands and source definitions remain in
the outer application configuration, never in declarative layers. Migration
does not grant executable trust or activate hardware. A development engine
remains paused until explicit Resume.

Notifications, command/network/audio providers, multi-source status/scene
profiles, custom analog SDK paths, unsupported settings and scene brightness
caps that cannot be represented exactly stay **byte-for-byte in legacy
compatibility mode**, with a warning. No partial setup silently disables their
providers or commands. Original scene definitions are retained even when their
rules migrate.

Matrix + Pi is an explicit one-time development starter, not a runtime mode.
Existing custom Pi rules are migrated, not replaced. Canonical setups never
receive automatic Pi layers; deleting Pi remains deliberate across selections,
rebuilds and restarts. A private saved-state marker prevents repeated development
upgrades when a legacy profile needs compatibility mode. To start over, select
the bundled Matrix + Pi example explicitly.
