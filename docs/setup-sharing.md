# Sharing declarative setups

Use **Share setups safely** in the setup library. Sharing is not a private
configuration backup: it accepts and exports only `SetupDocument` schema v1
([format](setup-format.md)), in TOML or JSON, at most 256 KiB. Unsupported
versions, renderers, fields and invalid groups/targets fail before any change.
Unknown renderers are rejected, never fetched or silently dropped.

## Export

1. Explicitly select **Applied setup** or **Validated current draft**. The latter
   includes unsaved edits, but neither saves the draft nor activates it.
2. Choose TOML/JSON and enter a **new local destination path**.
3. Export. Existing files (including symlinks) are never overwritten. A complete
   private file is published atomically; validation/write failures cannot truncate
   the old destination. Choose a different path to export again.

Only declarative appearance, targets, bottom-to-top layer order, groups, and all
base/variant conditions are shared, including disabled appearances. Runtime
editing revision resets to zero. Source identities become neutral `role-1`,
`role-2`, etc.; shared roles preserve which conditions used the same source.
No connection registry, producer argv/environment/cwd, executable trust,
credentials, machine SDK paths, or engine enabled state is serialized.

Names and fact/instance selectors are user-authored intent and remain literal.
Review them before sharing if you put sensitive information in those labels.
The exporter cannot discover secrets embedded in arbitrary display names.

## Import

1. Paste a document or enter a regular local file path, then explicitly validate.
   URLs are never fetched. Failed validation changes neither the editor, active
   setup, library nor local connections.
2. Inspect the isolated hardware-free preview. Scenario facts are synthetic and
   private to that preview; they never reach the engine. A conditional appearance
   without matching synthetic facts remains absent, not silently deleted. Use
   **Simulate original condition (isolated)** beside a binding to test its exact
   exported source-role/fact/instance selectors without any local connection.
3. For **every** source binding, choose a configured local connection and
   explicitly choose any instance or an exact local instance ID. This includes
   disabled layers/variants, and is required even when imported and local source
   names happen to match. Nothing silently binds to raw/unconfigured sources.
   All bindings must be resolved before storing.
4. Read capability warnings. State/event/transition tokens are preserved without
   inventing equivalence or translating facts. A configured connection can still
   be disconnected, disabled or untrusted; import does not start or authorize it.
   Add metadata or configure executable trust separately in **Connections**.
5. Store a new copy with a new local ID. Existing/active IDs cannot be replaced
   by this workflow. Revision starts at zero, not the sender's engine CAS value.
   Store changes only the saved library; the current dirty draft stays intact.
6. Explicitly Switch from the library when ready. Switching is paused; lighting
   requires a separate explicit Resume. Imported data never grants trust.

Cancel import discards only the import preview and mapping choices. Editing the
input or changing the connection-registry revision invalidates old approvals.

## Core API

`setup_sharing::{export_document, export_file, ImportPreview, BindingMapping}`
has no engine/producer transport. `ImportPreview::parse/read` validates and
compiles without I/O beyond the requested local read. `prepare` requires a fresh
noncolliding ID and complete explicit mapping through configured connections,
then returns a declarative `ImportCandidate` plus capability warnings. Callers
must store that candidate explicitly; a preview document is not an activation
request. The GUI returns only a library candidate, never Resume or connection
replacement.

File publication requires same-filesystem hard-link support; unsupported
filesystems return an error rather than falling back to unsafe truncation.
Native-window, physical keyboard and real-SDK behavior are not tested by the
sharing workflow's headless tests.
