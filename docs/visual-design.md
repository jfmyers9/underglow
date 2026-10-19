# Editor visual design (M8)

## Approved direction

The user approved the whole-screen visual direction on 2026-10-18:
“Approve—implement this direction and use judgment for details.”
JIM-51 records the approval. This approves implementation, not final native
usability or physical-hardware acceptance.

Use a restrained desktop creative-tool presentation: neutral dark surfaces,
one UI accent, and color concentrated in the actual keyboard preview. Preserve
Where / Looks / When, but do not surround every section and control with
competing outlines. The keyboard is the visual centerpiece.

The SVGs in [design/visual-editor](design/visual-editor/) are static design
references, not application screenshots. Their RGB patterns are illustrative,
not a second evaluator or evidence about live connections. Implementation uses
the existing canonical renderer. Actual egui captures and the final user
walkthrough establish implementation fidelity and usability separately.

## Visual system

| Role | Direction |
| --- | --- |
| Canvas | Neutral charcoal, `#15181b` |
| Surface | Quiet panel, `#1b1f23` |
| Field / secondary control | `#272d33`; minimal idle stroke |
| Divider | `#343c44`, only between meaningful groups |
| Primary text | `#f0f3f6` |
| Secondary text | `#aeb8c2` |
| Accent | Restrained cyan, `#89cee6` |
| Warning / error | Distinct amber / rose, with text and non-color cues |

- Body 14px; controls 13–14px; secondary text 12px; section headings 14px
  strong; selected-layer heading 17–18px; setup identity 20–21px.
- Use a small 4/8/12/16/24px spacing scale. Align label/control columns.
- Standard controls approximately 28–30px high; modest 4–6px radii.
- Reserve filled accent for a meaningful primary action and quiet tinted
  surfaces for selection. Status is not a primary-action button.
- Focus, hover, selected, active, disabled, destructive and error states must
  remain distinct. Normal text targets 4.5:1 contrast; meaningful controls and
  focus indicators target 3:1 where applicable.
- The wordmark remains identifiable without a large illustrated icon/tagline
  consuming workspace height. Remove decorative spectrum seams from the editor.
- Do not replace every pixel value with an abstract framework. Reuse established
  egui controls and small shared styling helpers where useful.

## Responsive composition

Validate the actual available width and font/control fit. Approximate budgets:

| Window | Composition | Main dimensions |
| --- | --- | --- |
| 1280×800 | Layers / canvas / right inspector | 184px rail, 320px inspector, 16px gaps, keyboard up to 700px |
| 940×850 | Layers beside canvas; inspector below keyboard | 184px rail, 16px gap, main area about 716px |
| 620×640 | Layer picker / pinned canvas / scrolling inspector | Header ≤100px; picker about 34px; pinned canvas about 280–295px |

Use roughly 1200px available width for three columns and 760px for a persistent
layer rail; settle precise thresholds from actual captures. At minimum size,
keep a genuinely usable inspector viewport (at least the existing 175px pane
regression) rather than shrinking keys or placing controls beyond the window.
The keyboard retains the existing ≥18px key-face regression. Increase tiny
legends toward 8–11px as the measured key widths permit; keep full physical-name
tooltips and do not invent unsupported Fn/light-bar mappings.

### Toolbar and layers

- Quiet navigation row: identity, Editor / Connections, secondary app tools.
- Setup row: readable setup switcher, saved/dirty/pending state, Save, separate
  truthful output state and explicit Enable/Pause when applicable.
- Simulation clearly says hardware disabled; do not present that label as a
  primary action. Native output controls retain existing guards.
- Give the Layers rail the inspector's surface color, 6px corners and 12px
  inset. Keep Layers / Add layer and the stack-order hint pinned above a quiet
  divider; only the rows scroll. The compact layer picker stays unchanged.
- Use consistent 32px layer rows and full-row selection, hover and keyboard-focus
  cues; names must not have an extra button-shaped highlight. Preserve the native
  enable control and overflow menu.
- Keep setup identity, Save, layer context, keyboard and essential preview
  context pinned while long layer lists and inspector content scroll.

### Inspector

- One continuous inspector surface with clear Where / Looks / When hierarchy,
  light section dividers and aligned controls; no nested three-card chrome.
- Where: effective target summary and one Edit keys entry point. Selection
  options and reference-safe group operations remain available.
- Looks: explicitly labelled Editing selector, effect, color, percentage
  brightness and capability-appropriate parameters. Appearance actions manage
  appearance naming, enablement, addition, deletion and priority.
- When: readable actual condition and direct Edit condition control; concise
  truthful source status and contextual repair/details.
- Wide inspector stacks fields; normal-width lower inspector can use aligned
  field columns. Compact inspector scrolls rather than clipping controls.
- Advanced mixing/transitions, technical information and complete help remain
  reachable. Do not hide invalid/unsupported-condition warnings or trust risks.

### Preview and testing

Use one compact strip: **Follow real activity** is a direct return to real
observations; **Simulate activity…** starts a selected-layer simulation only
after an activity is chosen. Show its identity with **Simulating:** and Pause
simulation / Play simulation. Put advanced tools behind **More… → Advanced
preview…**, not a third equally prominent workflow.
Keep this distinct from Preview / Select keys / Inspect click modes and from
the appearance currently being edited. Common capability-derived tests must
remain discoverable, and long custom action labels need a measured fallback.
Details, manual clock, simultaneous sources, explanations and coverage remain
available without dominating the ordinary editing surface.

## State and safety contract

Restyle the existing behavior; do not replace it:

- One canonical setup/model, stable IDs, typed edits and shared renderer/runtime.
- Explicit CAS Save, safe late acknowledgements, dirty-draft protection and
  deliberate Pause-and-save. Save never resumes or enables lighting.
- Appearance selection is view state; private test facts never become real
  connection observations, producer facts or physical output.
- Pi stays one logical layer. Finished means settled, not success. Missing or
  expired facts never imply idle or success. Preserve process/data distinctions.
- Preserve custom and legacy setups without automatic merging, replacement or
  reinjection of deleted templates/layers.
- Machine-local executable trust, credentials, producer activation and service
  management remain independent explicit decisions.
- No schema/runtime redesign, new effects/connectors, autosave, SDK invocation,
  installed-service control or automated native windows.

## Delivery and acceptance

JIM-51 design approval → JIM-52 shared foundations → JIM-53 workspace and
JIM-54 shell/layers → JIM-56 testing / JIM-57 inspector / JIM-55 secondary
surfaces → JIM-58 integrated validation and user-owned native acceptance.

Compare actual production-style captures at all three sizes with these
references, including empty/dirty/pending/error/stale/legacy/expanded states,
long content and independent Editing Working / Testing Finished selections.
Use real clipped hit regions, genuine scrolling and keyboard focus checks.
Run focused GUI regressions, full Rust/GUI tests, strict Clippy, formatting and
diff checks. Run final suites sequentially with distinct logs.

Headless captures prove layout only. Do not mark JIM-58 Done until the user
accepts the native hierarchy, readability, clutter, controls and animation feel
in hardware-disabled simulation. Physical lighting acceptance is separate.
