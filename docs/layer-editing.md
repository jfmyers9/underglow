# Editing layers

Choose a layer in the top-first stack (or the compact **Layer** picker), then
use the **Appearance**, **Keys**, and **Behavior** inspector tabs. These are
views of the same draft, not independent copies. Editing and testing never
enable keyboard lighting; save explicitly and enable lighting separately.

## Common changes

- **Appearance** is the default. Choose the layer's effect or color, then adjust
  supported parameters and layer brightness. Changing Matrix to another effect
  does not replace the setup, its other layers, keys, or behavior.
- **Keys** edits the selected logical layer's target and reusable groups. Every
  appearance of that layer shares these keys. Group changes still protect all
  other references, including disabled layers.
- Existing zones/groups combine with direct keys. **Also includes** lists those
  extras; after selecting direct keys, **Use selected keys only** removes this
  layer's extra references without modifying shared groups or other layers.
- **Behavior** explains when the selected appearance is shown. Expand
  **Advanced condition** for the generic source/state/event/transition builder.
  Saved missing selectors remain intact with a repair warning. Observed choices
  come only from real read-only source samples, never private preview scenarios.

**Add layer** opens a non-mutating menu: choose **Aurora layer**, **Matrix layer**,
**Solid color layer**, **Pi status layer**, or another catalog effect. The new
editable layer is appended above the current stack. To begin a different setup,
choose **Browse setups**, then **Blank**. Unsaved drafts get Save / Discard /
Cancel protection before replacement.

Stack checkboxes enable/mute individual layers. Each row's **...** menu provides
rename, duplicate, reorder and delete; the compact layout offers **Layer
actions**. The selected inspector also exposes its name and Duplicate/Delete
buttons. **Toward top** moves a layer visually upward, giving it precedence on
overlapping keys; the saved document continues to store bottom-to-top stable IDs.

## Pi appearances

The explicit Pi template is one logical layer with **Idle / fallback**,
**Working**, and **Finished (settled, not success)** appearances. Pick an
appearance in either Appearance or Behavior; the same stable-ID selection is
shared across both tabs. Customize its renderer and optional brightness override
without changing the other appearances. **Inherit layer brightness** retains the
layer-wide value rather than storing a separate override.

Deleting Pi removes the layer, not just one appearance. Selection, saving and
reopening do not reinsert it. Add another template only through Add layer.

## Advanced editing

- **Appearance → Advanced layer settings:** opacity, blending, transition
  duration and easing, shared by the whole logical layer.
- **Behavior → Add behavior appearance:** another editable appearance with a
  stable ID. Its name, enable state and condition remain independently editable.
- **Behavior → Advanced appearance settings:** stable appearance ID and raw
  priority ordering. Transient matches outrank persistent matches, then later
  matches win within a class. These rules are unchanged.
- **Advanced layer details:** stable layer ID for diagnostics and repair.

Private Scenario facts and physical keyboard output remain separate. The Pi
connector declares Idle/Working and Finished, not approval or success facts.
