# Editor workflow

The M7 restructure uses the existing setup, layer, preview, library and
connection models. Its purpose is to make a complete editing task understandable,
not introduce another renderer or a second copy of editor state.

## Primary journey

1. Choose a setup, or create an editable starter or blank setup.
2. Select or add a layer.
3. Change its appearance and choose its keys.
4. Try its behavior in the hardware-free preview.
5. Save the setup. If lighting is active, explicitly confirm pausing to save.
6. Explicitly enable keyboard lighting when ready.

Editing and testing never enable lighting. Saving leaves lighting paused.
The application distinguishes the draft being previewed from the saved version
being displayed by the keyboard.

## Workspace

Normal window, 940 by 850 logical pixels:

```text
Underglow         Editor | Connections                    Settings
[Matrix + Pi v]   Unsaved changes   [Save]   Lighting off [Enable]
-----------------------------------------------------------------
Layers             Keyboard preview            Pi status
----------------   -------------------------   ------------------
Pi status          [Preview][Select keys]       Appearance | Keys
Matrix             [Inspect]                   Behavior
[+ Add layer]      +-----------------------+   ------------------
                   | one composed keyboard |   Selected appearance
                   | with target overlays  |   Color / effect
                   +-----------------------+   Brightness
                   Preview only                Advanced settings
                   [Live][Try behavior]        (scrolls separately)
                   [Idle][Working][Finished]
-----------------------------------------------------------------
Saved setup / unsaved draft / keyboard-output state, when relevant
```

The topmost layer is displayed first. The document continues to store layers
bottom-to-top; presentation does not change composition semantics or stable IDs.
Long layer lists and inspectors scroll independently. The keyboard, setup
identity, save state and lighting controls must not scroll away with them.

At 620 by 640 logical pixels, collapse the layer list into an explicit picker
and keep the inspector below the keyboard:

```text
Editor | Connections                    Settings
[Matrix + Pi v]   Unsaved changes   [Save]
Lighting off                            [Enable]
------------------------------------------------
[Layer: Pi status v]               [+ Add layer]
[Preview][Select keys][Inspect]
+----------------------------------------------+
| one composed keyboard with target overlays   |
+----------------------------------------------+
Preview only   [Live][Try behavior]
Pi status     [Appearance][Keys][Behavior]
+----------------------------------------------+
| inspector content scrolls here                |
+----------------------------------------------+
```

Pane widths and breakpoint are implementation details to validate using the
actual application font, padding and spacing. Both documented sizes must keep
the primary controls reachable; a taller virtual window is not a substitute.

## Navigation and setup browser

- Editor and Connections are destinations, not section toggles.
- The setup selector opens a clearly titled browser/dialog. Merely selecting a
  saved item in that browser does not replace the draft or change lighting.
- Open a saved setup through an explicit action, with dirty-draft protection.
- Naming, creating, duplicating and deleting setups belong in that browser.
  Save a copy is distinct from saving changes to the current setup.
- Sharing, diagnostics, capability manifests and detailed producer settings are
  secondary flows. Keep consequential trust and replacement confirmations at
  the action that needs them, not hidden in an optional help panel.
- Navigating to Connections and back preserves the draft, selected layer,
  inspector context and preview. It does not install or enable a producer.

## One keyboard, explicit modes

| Mode | Click behavior | Changes the setup? |
| --- | --- | --- |
| Preview | Read-only composed output; no implicit key retargeting | No |
| Select keys | Edits the selected layer's target using named physical keys | Yes |
| Inspect | Explains the actual captured composition for that key | No |

The Keys inspector and Select keys mode communicate the same selection context.
Target, coverage and inspection overlays do not replace or modify rendered RGB
colors. Named groups retain reference-safe operations. Unsupported positions
are identified honestly; no new keyboard layout is implied by the restructure.

## Selected-layer inspector

- **Appearance:** color/effect, supported parameters and appearance selection.
  Pi remains one logical layer with Idle, Working and Finished appearances.
- **Keys:** whole-keyboard/direct-key selection, shortcuts, zones and groups,
  using the one shared canvas rather than a second keyboard.
- **Behavior:** when the selected appearance is shown, with readable summaries
  and the existing generic condition builder available for advanced editing.
- Stable IDs, raw precedence, blend/transition details and protocol-level
  diagnostics remain available without crowding the default editing path.

The layer list supports enable, rename, duplicate, delete and reorder. Removing
a layer never causes a starter or template to be silently reinserted later.

## Preview and behavior testing

Live connection facts and private test facts are visibly separate contexts.
Quick test actions derive from the layer's conditions and source capabilities.
For Pi, Idle and Working simulate persistent states; Finished emits a transient
event and means settled, never success. Do not invent focus or approval facts.

Quick testing needs no live producer, session-ID entry or TTL configuration.
Animation/playback state must be obvious. Arbitrary sources, simultaneous
channels, expiry and deterministic clock stepping remain in an advanced test
panel. Test facts never enter the live registry or alter real producer intent.

## Save and lighting interaction states

| State / action | Required behavior |
| --- | --- |
| Saved and paused | Show Saved; Enable lighting is a separate explicit action |
| Dirty draft and paused | Save writes through the existing revision-checked path |
| Dirty draft and lighting active | Show that the keyboard uses the saved version; Save offers an explicit Pause and save confirmation |
| Pause-and-save cancelled | Keep the draft and current lighting unchanged |
| Pause succeeded, save failed | Keep the draft, show the error and actual paused state; do not resume |
| Pending save | Show progress and retain the exact submitted revision/document |
| Later edit before an acknowledgement | Do not mark a newer draft saved or replace it with the earlier result |
| Revision conflict | Retain the draft and explain recovery; never force overwrite |
| Switch or replace a dirty draft | Offer Save / Discard / Cancel, preserving the draft on failure or Cancel |
| Open/close browser or navigate pages | Preserve the current draft; no save/switch/activation side effect |
| Engine offline | Allow local editing/testing; explain why saving and activation are unavailable |
| Missing or stale connection | Explain the affected behavior and offer repair/testing; do not fabricate live health |
| Legacy setup | Explicitly offer keeping existing behavior or beginning a new setup, with replacement consequences before adoption |

The setup/library revision checks and atomic persistence remain authoritative.
Do not replace the applied baseline just because an asynchronous action failed.

## Safety and acceptance boundaries

There is still one RGB writer. Physical lighting activation is explicit, and
Pause releases ownership. Machine-local producers, credentials and executable
trust stay separate from shared declarative setups. Source profiles and installed
services/login startup remain untouched by editing, testing and migration.

Validate interaction and layout headlessly at the documented window sizes using
the actual brand style. Automated checks are necessary but do not establish
whole-product usability. Finish with a user walkthrough in hardware-disabled
simulation. Do not automate native-window/actual-SDK acceptance or treat this
work as completion of the separate physical keyboard acceptance ticket.
