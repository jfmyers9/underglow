# Editing layers

Select a layer. The inspector is three cards in one scrolling form:

- **Where:** the current physical target and **Edit keys**.
- **Looks:** effect, color and percentage brightness for the selected appearance.
- **When:** its readable condition, **Edit condition**, and relevant connections.

There is no separate navigation index, duplicate heading strip or tab switch.
The keyboard, layer identity and Save remain fixed while the cards scroll.
At compact sizes, use the always-visible inspector scrollbar to reach later
cards rather than squeezing all controls into a small grid.

## Where: choose keys

Click **Edit keys**, then select keys on the shared keyboard. Click the same
button again to finish; there is no separate Done control. **Select keys** above
the keyboard is another entry point to the same editing mode.

The card summary lists direct keys, saved zones and group references. They
combine as a union. If direct keys coexist with extra references, **Use selected
keys only** removes this layer's zone/group references explicitly. That action
appears only when it can do something; shared groups and other layers stay intact.

**Selection options** contains whole-keyboard/arrow presets, **Saved zones**,
reusable groups and physical-map help. Saved zones preserve the legacy engine
map, including unnamed cells; their meaning is not silently converted. The
physical arrow preset selects the four named arrows. All appearances of a
logical layer share its target.

## Looks: change an appearance

For a layer with multiple appearances, **Editing** chooses which one to change.
Pi has **Idle / fallback**, **Working** and **Finished (settled, not success)**.
Choose an effect and its supported parameters, then adjust brightness.

Brightness is displayed/entered as a percentage; existing saved 0–255 values
are preserved exactly until edited. **Use layer brightness** keeps an appearance
on the layer-wide setting rather than storing an override. Smaller **Cycle
duration (ms)** values make Pulse/Sweep animations faster.

**Appearance actions** owns appearance creation, renaming, enablement, deletion
and advanced priority. These controls no longer interrupt condition editing.
Effect descriptions are available on the effect picker's tooltip rather than
as repeated general-purpose paragraphs.

Selecting Working to edit does not simulate it. Use the separate test buttons
beneath the keyboard; the preview caption names the tested behavior. You can
edit Working while testing Finished. **Live** reads actual source facts.
Private tests never publish source messages, save or enable physical lighting.

## When: change the trigger

The card shows the selected appearance's condition directly. **Edit condition**
opens the source/state/event/transition controls right there, without another
Advanced condition disclosure. Click it again to return to the readable summary.

Pi's base uses current state data when no other appearance overrides it; absent
or expired data is not inferred as idle. Finished is settled, not success.
Exact session selectors and disabled appearances are included in summaries.
Suggestions use real observations only, never private test facts.

Connection status belongs in this card. **Connections: …** opens the relevant
source for repair while preserving the draft. Short status reports describe
recent data, not inferred process health; hover for the complete diagnostic.

## Layer and setup actions

**Layer actions** (or a stack row's **...**) owns rename, enable, duplicate,
reorder and delete. **Toward top** raises precedence on overlapping keys.
**Layer mixing & transitions** owns opacity, blending and crossfades for the
whole layer. **Technical details** retains stable IDs.

Deleting Pi remains deliberate across saving, reopening and rebuilds. Older
migrated profiles may retain three independent Pi rules. They are explained,
not automatically merged. To adopt one Pi layer, save a copy, remove the old
rules, then add Pi status and configure its target and appearances.

**Current setup** identifies this combination. **Change setup...** opens setup
creation, saved setups and Save a copy. Save/Discard/Cancel protects dirty drafts;
Save and physical lighting activation remain separate explicit actions.
