# Save a setup without activating lighting

The header shows the current draft's name and whether it has **Unsaved changes**.
**Save** always saves that draft. It does not enable keyboard lighting.

1. Edit or test the draft in the Editor.
2. Choose **Save** in the header.
3. If lighting is active, review **Pause lighting to save?** and choose
   **Pause and save**, or **Cancel** to leave lighting and the draft unchanged.
4. After saving, lighting stays paused. **Resume lighting** is a separate,
   explicit action. In development simulation, physical lighting is disabled.

The output control's details identify the saved version the keyboard uses,
separately from the draft. The preview is not physical-device feedback.
Offline, editing and preview testing remain available; saving requires an engine
connection. Pending requests do not silently discard or acknowledge later edits.

## Name, copy, create or open

Choose the **setup-name switcher** to open its browser. Selecting an item only
browses it; **Open saved setup** is the explicit opening action.

- **Name draft** uses the entered name for the local draft. Persist that change
  with the header's **Save** action.
- **Save a copy** stores the current draft under a new identity, using the entered
  name. It neither opens the copy nor marks the original draft saved.
- **Blank**, **Aurora** and **Matrix + Pi** create local drafts. Their layers are
  editable copies, not recurring defaults. They are not persisted until Save.
- Opening or creating another setup while dirty offers **Save changes**,
  **Discard changes** and **Cancel**. A failed Save prevents the opening action.
  A failed opening request retains the original draft even after Discard was
  selected. Opening while active requires its own **Pause and open** confirmation.

For a legacy profile, the browser explains that adopting a new saved setup
replaces legacy notifications and automation rules in the engine's private
snapshot. Source profiles and machine-local Connections are not modified.

## Recover without overwriting someone else's changes

A revision conflict retains the draft; it does not force an overwrite. You can:

- Use **Save a copy** to preserve your current work separately.
- Choose **Reload saved version**, then **Discard changes**, to read the current
  saved version. **Cancel** keeps your draft. Reloading does not change lighting.

If Pause succeeds but Save fails, the error and actual paused state remain
visible. The draft is retained and lighting is never automatically resumed.
Malformed or missing acknowledgements stop pending opening actions. Refresh the
engine status before retrying an uncertain result; the operation may already
have persisted. A valid acknowledgement of an earlier draft updates the known
saved version but never clears newer edits.

Producer executables, credentials, trust, installed services and login startup
are separate from setup persistence. Save, copies, imports and preview tests
never publish live source facts, start producers or activate lighting.
