# Testing appearances without a producer

The editor's keyboard always uses the existing setup evaluator and compositor.
Testing changes private preview facts, never the setup, live source registry,
producer configuration, lighting activation or keyboard output.

## Quick tests

The two everyday controls below the keyboard are:

- **Follow real activity** previews the draft using read-only activity reported
  by connected apps. It does not enable lighting or mirror the physical keyboard.
- **Simulate activity…** lets you try a supported activity for the selected
  layer, such as Pi Working or Finished, without running that activity in Pi.
  Opening the picker changes nothing; choosing an activity starts the simulation.

The picker then says **Simulating: Working** (or the chosen activity), with
**Pause simulation / Play simulation** beside it. Click **Follow real activity**
to end the simulation in one step. Editing an appearance remains independent
of simulating it.

Choices come from the selected layer's conditions and registered source
capabilities. Presence conditions offer that source's declared states; specific
state/event/transition conditions offer only matching declared facts. Disabled
appearances are not offered. Unknown configured facts are preserved, not
advertised as supported. Layers that are always shown need no source test.
Many choices use a picker rather than an expanding toolbar.

For the editable Pi status template:

- **Idle** and **Working** publish a private, finite state lease.
- **Finished** emits a private transient event. It does not create a state,
  imply success, renew presence or fabricate approval/focus facts.
- Every choice starts a fresh quick scene. Choosing Idle after Finished clears
  that quick scene's outgoing transient immediately.

No session IDs or TTLs need to be entered. Exact conditions use their configured
identity only inside the private test; any-instance conditions use a synthetic
identity. Lease lengths respect the declared source TTL range (up to 15 seconds
when that range permits it). A fresh event starts the appearance's configured
display duration, even after its event lease expires; this never creates
persistent source state. State-transition windows remain source-lease-capped.
Facts are never automatically renewed.

Tests start playing so Matrix and other animations keep advancing.
**Pause simulation** freezes animation, leases and transient time;
**Play simulation** resumes without
catching up time spent paused. An expired test needs a new explicit choice.

## Advanced tests and inspection

**More… → Advanced preview…** opens the advanced window, retaining the
independent multi-source **Scenario**, registered source/session controls,
finite leases, Play/Pause, deterministic
steps and the hardware-free **Isolated input demo**. The advanced scenario is
retained and paused when leaving its context, including while quick-testing.
Returning baselines its old occurrences instead of replaying them. **Reset
scenario** explicitly clears that advanced scene; **Reset quick test** clears
only the private quick scene.

Save, Discard and document replacement baseline the evaluator and clear quick occurrences;
they do not replay old Finished events. Switching back to Live also baselines
the live cursor, so historical real events cannot be replayed as new activity.

Under **Keyboard interaction**, choose **Inspect key**, then click a key to open
its explanation before the advanced forms. The report uses the same captured
frame, facts and evaluation clock as
the RGB canvas. Target/coverage/inspection annotations never replace RGB fills.

Headless interaction tests cover the production-styled 940×850, 620×640 and
1280×800 workspaces.
They are not native-window, physical-keyboard or whole-product usability
acceptance.
