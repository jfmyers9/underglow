# Saved setups

Open **Saved setups** in the Layers toolbar. Name and **Save current setup**
while lighting is paused. Duplicate a saved setup to give it a new document ID,
then switch to that copy to customize it. The original remains unchanged.
Switching is disabled until you Save or Discard any editor draft, and requires
Pause. Nothing in the library resumes lighting; Resume remains explicit.

**Blank**, **Aurora**, and **Matrix + Pi** create editable drafts with new IDs.
Save gives them a permanent library entry. Their layers, variants, references,
and groups are ordinary setup data: deleting every layer and saving is valid.
Opening or switching a setup never reinserts Pi or reapplies a template.
Saving the first canonical composition from legacy compatibility mode explicitly
replaces legacy notifications and automation rules in the private engine snapshot.
Source profiles and Connections are unchanged. Merely opening a starter does not
alter the retained legacy configuration. Hybrid canonical profiles with active
legacy notification/automation overlays are rejected, not rendered invisibly.
Rename affects only the selected saved setup. Delete requires confirmation and
cannot delete the current setup; switch to another entry first.

## CLI and local protocol

`underglow control library list` returns compact summaries in `library.setups`,
plus `schema_version: 1`, `revision`, and `current_id`. It does not return every
document. `underglow control library get ID` adds that selected `document`.
IDs (including slashes/dots) and names are JSON data, never filesystem paths.

Mutations use bounded JSON stdin and optimistic concurrency:

```sh
printf '%s' '{"op":"create","starter":"matrix-pi","id":"my-setup","name":"My setup"}' |
  underglow control library update --expected-revision 0
printf '%s' '{"op":"switch","id":"my-setup","expected_setup_revision":0}' |
  underglow control library update --expected-revision 1
```

Use the actual library revision from `list` and the separate current setup
revision from `control setup show`. Each accepted library mutation increments
the library revision; switching increments the global current-setup revision.
The current setup's content comparison ignores this runtime revision token.

JSON mutations (`op`):

- `store`: `document` (complete declarative SetupDocument, insert or replace).
- `create`: `starter` (`blank`, `aurora`, `matrix-pi`), unique `id`, `name`.
- `duplicate`: existing `id`, unique `new_id`, `name`; preserves layer identities
  and all connection references, resets the copy's document revision to zero.
- `rename`: `id`, `name` (current entry requires Pause).
- `delete`: `id` (cannot be the current setup).
- `switch`: `id`, `expected_setup_revision` (requires Pause).

Protocol actions are `library-list`, `library-get` (`id`), and `library-update`
(`expected_revision`, `mutation`) under protocol `schema_version: 1`.
Store never activates a setup or authorizes a connection. To change current
content use `control setup replace --expected-revision N`; that existing command
now atomically saves both current composition and its named library entry.
Store rejects changed content targeting the current ID rather than letting the
stored current entry silently diverge. Switch/rename return the current setup
envelope in addition to summaries; errors leave active and saved data unchanged.

## Persistence and boundaries

The private `state.json` contains `setup_library: {schema_version:1, revision,
documents:[...]}`. Old states copy the canonical current composition into the
library once, without replacing configuration or existing library entries.
An intentionally empty initialized library stays empty. Active configuration
and library updates share one private atomic rename, not two partial writes.
The independent `connections.json`, producer processes, trust, SDK paths, and
global lighting-enabled state are not library data and are never changed by
library operations.

Limits: 64 entries, 256 KiB per document/mutation input, 768 KiB serialized
library, and 1 MiB total serialized private state (including newline). Every
save preflights the total size so it remains readable on restart. List/get keep
GUI responses below its 512 KiB budget without streaming the entire library.
