# Editor usability walkthrough

This checks whether the application makes sense to use, not just whether its
individual controls work. Try each task using the interface alone. If you need
developer instructions, cannot tell what will happen, or click the wrong thing,
record that as a finding rather than working around it silently.

## Safe session

Use the development simulation (`make dev`) with hardware disabled. No keyboard
is required. Do not enable hardware, install or change a service, enable login
startup, or overwrite a source profile for this walkthrough. The user opens and
uses the window; automated checks remain headless and do not invoke the actual
SDK. Physical lighting and restoration acceptance are separate.

Use a disposable editable setup so this exercise does not replace a composition
you want to keep. Start at the normal 940×850 window size, then repeat the core
editing tasks at 620×640 and resize to a wider window.

## Tasks

1. **Understand the starting screen.** Identify the current setup, selected
   layer, whether you have unsaved changes, and whether the keyboard is being
   controlled. Explain whether the on-screen keyboard is device feedback or a
   hardware-free preview.
2. **Build a composition.** Start with Matrix as the background and add one Pi
   status layer. Find where to change an appearance without replacing the rest
   of the setup.
3. **Choose keys.** Put Pi on a few keys of your choice. Explain what a key click
   will do before clicking. Use both an individual selection and a multiple-key
   selection. Check whether zones or groups add any extra targets, and make the
   footprint exactly the keys you want. Change modes without accidentally
   changing targets.
4. **Try Pi's appearances.** Customize and preview Idle, Working and Finished
   without connecting a producer or entering protocol details. Finished means
   settled, not successful. Return to live connection data and explain which
   context is currently shown.
5. **Save deliberately.** Find the one primary Save action, save the changes,
   and verify that saving did not enable lighting. Recognize that enabling
   physical lighting is a separate action; leave hardware disabled here.
6. **Manage another setup.** Save a copy with a different name, browse saved
   setups, and open another one deliberately. Make an unsaved edit and try
   switching again; try Cancel and Discard without losing unrelated work.
7. **Remove a layer permanently.** Delete Pi from the disposable setup, save,
   and reopen it. Restart the development session if needed to check persistence.
   Pi must not be silently reinserted.
8. **Navigate without losing context.** Make an unsaved edit, visit Connections,
   and return. The edit and selected layer must remain. Explain the difference
   between reported source activity and a managed producer's process health.
9. **Inspect and go deeper.** Explain a key's composition without changing its
   targets. Find advanced scenario testing, sharing and diagnostics, then return
   to ordinary editing without being stuck in an administrative flow.

## What to record

- Which task and window size were involved?
- What did you expect to click or happen?
- What did the interface actually do, or fail to explain?
- Was the needed control visible, reachable and clearly named?
- Did the layout preserve your keyboard/layer context while scrolling?
- Did animation/playback, preview-only status and saved/draft state make sense?
- Was there an unexpected lighting, producer or persistence side effect?

Screenshots of the editor can help explain a finding. Avoid including private
connection configuration, environment values, credentials or command arguments.

Record findings in the UX validation issue (JIM-47). Automated interaction,
geometry and paint-summary tests are useful evidence, but they are not native
screenshots or proof of visual hierarchy, readability, animation feel, trackpad
behavior, display scaling or desktop integration. Do not mark the usability
walkthrough complete from those automated checks alone.

## Optional headless images

The test-only software exporter can capture the real egui geometry, built-in
fonts, textures and clipping at the default, minimum and wide sizes without a
window, GPU, engine worker or SDK:

```sh
UNDERGLOW_UI_SNAPSHOTS=/tmp/underglow-ui \
  cargo test --features gui --bin underglow-gui \
  workspace_snapshots::export_workspace_snapshots --locked -- \
  --ignored --exact --test-threads=1
```

The exporter uses fixture setup/connection data, not a screenshot of your desktop
or private configuration. Images are lossless PNGs, deliberately written only
when an output directory is explicitly supplied. Software sampling and desktop
rendering can differ; use these images for layout inspection, not native visual
or hardware acceptance. The normal suite tests the small in-memory capture path
without exporting files.

Each size includes normal, empty, dirty, pending, disconnected, stale-data,
error-message and legacy-entry fixtures. Pending/error messages are illustrative
fixtures, not evidence that an actual save failed; workflow acknowledgement,
conflict and recovery tests exercise those paths separately.
