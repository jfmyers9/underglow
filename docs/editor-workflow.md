# Editor workflow

The M8 visual refinement preserves the existing setup, layer, preview, library
and connection models. Its purpose is a calm, coherent editing surface, not a
second renderer, configuration model or copy of editor state. See the
[approved visual design](visual-design.md) for the visual-system reference.

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

Use real production fonts and a stable keyboard-over-inspector composition:

| Window | Layout |
| --- | --- |
| 1280×800 and wider | Centered 902px workspace; short Layers panel beside aligned keyboard and content-sized settings |
| 940×850 | Same composition; panels grow only with their content |
| 620×640 | Layer picker, pinned keyboard/testing strip, scrolling inspector |

The quiet header owns setup identity/switching, save state and the one Save
action. Output status and explicit activation are separate from Save and private
preview. Add layer belongs in the layer rail or compact picker, not another
global toolbar.

The topmost layer is displayed first; canonical document order remains
bottom-to-top. Layer lists and inspector content scroll independently. Setup
identity, Save, layer context, keyboard and essential preview input/playback
context stay pinned.

Where / Looks / When are sections of one continuous inspector surface with
aligned fields, restrained dividers and contextual actions. The exact pane
budgets and breakpoints follow actual control/font fit, not a fixed three-column
layout forced into the normal window. Expanded controls and long content must
use genuine scrolling. A taller virtual viewport is not a substitute.

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

Where and Select keys mode communicate the same selection context.
Target, coverage and inspection overlays do not replace or modify rendered RGB
colors. Named groups retain reference-safe operations. Unsupported positions
are identified honestly; no new keyboard layout is implied by the restructure.

## Selected-layer inspector

The original M7 tabs and later navigation-index/three-card refinements are
superseded by one continuous, quiet inspector:

- **Where:** effective target summary, one Edit keys toggle, Selection options.
- **Looks:** explicit Editing choice, effect/color/percentage brightness,
  Appearance actions and capability-appropriate parameters.
- **When:** readable actual condition, direct Edit condition and concise truthful
  source status with contextual repair/details.

All sections remain mounted; key and condition editors can be open independently.
Their content scrolls without moving the keyboard, selected-layer context or
Save. Advanced mixing/transitions, technical details and complete effect/help
information remain available; blocking warnings are not hidden for appearance.

Appearance selection is stable-ID view state and never changes test input.
The separate testing strip names its active test and playback state. Pi remains
one logical layer with Idle/fallback, Working and Finished appearances. Missing
data never means idle or success. Percentage display does not quantize saved
brightness. Original setups, executable trust, save acknowledgements and lighting
ownership semantics are unchanged.

Layer actions own layer management; Appearance actions own appearance management.
The setup-name switcher owns browser navigation. There is no extra section index,
tab strip, duplicate heading row or competing Done/Close control.

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
