# Layer appearance inspector

`src/bin/gui/setup_inspector.rs` provides `show(ui, &mut Layer) -> bool` for
the setup editor (JIM-20 mounting is separate). It reports only real appearance
or per-layer brightness changes; the caller owns document revision, validation,
persistence and preview refresh. All widget state is scoped by stable layer ID,
not editable names or stack indices. Target selection remains in `setup_targets`.

Effect choices, labels, palettes, speed and two-tone capabilities use the shared
renderer catalog. Diagnostic effects already present in a document are preserved
but omitted from the everyday chooser. Solid, Pulse and Sweep are explicit scene
primitives outside the visualization catalog: RGB color, plus period for Pulse
and Sweep. Focus timings come from canonical setup model defaults. Keyboard
input requirements are informational, not SDK configuration or permission grants.

Switching effects preserves layer identity, name, enabled state, target,
activation, order and brightness. Explicit palettes and speeds survive only when
both renderers support them. Switching to fixed colors clears the palette;
switching back uses the new renderer's default, not a hidden stale setting.
Primitive colors and periods are retained only between compatible primitives.
Ripple two-tone mode clears the palette and exposes exactly two color controls;
returning to palette mode clears both colors. Unsupported controls are absent
with an explanation, and invalid imported settings are reported, not silently
rewritten on an idle frame.

Layer brightness changes only that layer's intensity. Global brightness and
safety limits remain separate and still cap physical output. This component has
no engine, source or SDK access: edits cannot acquire hardware, start producers,
emit real input or Resume physical lighting. Preview remains hardware-free;
actual lighting requires explicit Resume through the existing single writer.

Headless egui tests drive real pointer events through effect/palette switching,
brightness, ripple two-tone and Focus controls, including fixed Matrix controls,
compatible setting retention and independent layer preservation. No native GUI
or physical SDK is invoked.
