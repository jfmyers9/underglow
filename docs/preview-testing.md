# Testing appearances without a producer

The editor's keyboard always uses the existing setup evaluator and compositor.
Testing changes private preview facts, never the setup, live source registry,
producer configuration, lighting activation or keyboard output.

## Quick tests

Select a layer, open **Test behavior…** below the keyboard and choose a
capability-derived test. The selected label names the active test and playback
state. The separate preview-input picker distinguishes **Live data**, **Private
test**, isolated input demos and advanced scenarios. To end a private test,
open **Private test** and choose **Live data**. This is not device feedback.

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
when that range permits it). Events and transitions retain their actual runtime
duration and source-deadline caps. Facts are never automatically renewed.

Tests start playing so Matrix and other animations keep advancing. **Pause test**
freezes animation, leases and transient time; **Play test** resumes without
catching up time spent paused. An expired test needs a new explicit choice.

## Advanced tests and inspection

**Preview tools** retains the independent multi-source **Scenario**, arbitrary
registered source/session controls, finite leases, Play/Pause, deterministic
steps and the hardware-free **Isolated input demo**. The advanced scenario is
retained and paused when leaving its context, including while quick-testing.
Returning baselines its old occurrences instead of replaying them. **Reset
scenario** explicitly clears that advanced scene; **Reset quick test** clears
only the private quick scene.

Save, Discard and document replacement baseline the evaluator and clear quick occurrences;
they do not replay old Finished events. Switching back to Live also baselines
the live cursor, so historical real events cannot be replayed as new activity.

In **Inspect** mode, click a key to open its explanation before the advanced
forms. The report uses the same captured frame, facts and evaluation clock as
the RGB canvas. Target/coverage/inspection annotations never replace RGB fills.

Headless interaction tests cover the production-styled 940×850, 620×640 and
1280×800 workspaces.
They are not native-window, physical-keyboard or whole-product usability
acceptance.
