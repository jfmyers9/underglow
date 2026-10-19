# Native controller

`underglow-gui` is an optional Rust/eframe desktop window, not a browser or an SDK
client. It invokes the **sibling** `underglow` executable for engine/control
commands and the sibling `underglow-service` helper for login/service management.
Install matching versions together; it deliberately does not search `$PATH`.

## Build and launch

```sh
cargo build --features gui --bins
./target/debug/underglow-gui
```

Building alone does not start the engine or access keyboard hardware. Ordinary
`cargo build` and `cargo run -- …` remain headless CLI operations; eframe is only
compiled with the `gui` feature. Linux GUI builds require native window-system
build dependencies for X11/Wayland and a working OpenGL desktop. macOS uses the
native Cocoa window system. Platform service installation is separate; see the
packaging/service documentation.

On macOS, the **UG** menu-bar item offers **Open controller** and **Quit controller
(leave engine running)**. Closing the window hides it while the menu-bar item
remains; use the menu to reopen it or quit. The menu-bar launcher is built only
on macOS through an optional, GUI-only `tray-icon` dependency.

Linux has no tray dependency: reopen the controller using its installed desktop
launcher. The platform launcher plus **App settings** is also the
fallback if macOS menu-bar creation fails (reported on stderr and in the window).
Closing or quitting never pauses or kills the engine, and the controller never
registers login startup automatically.

## Edit a setup

The workflow is **choose a setup → edit layers → choose keys → try behavior →
Save → explicitly enable lighting**. For hardware-free development, use
`make dev`; its default simulation does not access the keyboard.

### Find your place

- **Editor** is the lighting workspace; **Connections** is a separate destination
  for applications and producers supplying information.
- The **setup-name switcher** identifies the combination you are editing and
  opens its browser. Browsing does not replace your draft; opening is explicit.
- The fixed header shows the setup, **Saved / Unsaved changes**, the one **Save**
  action and keyboard-lighting state. **Start engine / Resume lighting / Pause
  lighting** controls physical output separately.
- At 940×850, layers sit beside a larger keyboard with the inspector below it.
  At 1280×800, the inspector moves to the right; at 620×640, use the **Layer**
  picker and scrolling inspector below the pinned keyboard/testing strip.
  Resizing or visiting Connections preserves the draft and selected layer.

### Create and customize

1. Open the setup-name switcher and choose Blank or an editable starter such as
   **Matrix + Pi**. A starter is a local draft, not a recurring default.
2. **Add layer** offers Matrix, Aurora, a solid color, Pi status and other
   supported effects. Layers are topmost first. Row **...** menus and compact
   **Layer actions** provide rename, duplicate, reorder and delete.
3. One continuous inspector explains **Where** (keys), **Looks** (effect/color/
   percentage brightness) and **When** (trigger and connection data). The
   **Editing** picker selects an appearance; **Appearance actions** manages it.
   **Edit condition** opens its trigger directly. Sections scroll together,
   without an extra navigation index, nested cards or competing Done/Close buttons.
4. Use **Where → Edit keys** on the shared keyboard. Zones, direct keys
   and group references combine as a union. **Also includes** lists the extras;
   **Use selected keys only** removes that layer's zone/group references once
   direct keys exist. Shared groups and other layers are unchanged.

Pi is one logical layer with **Idle / fallback**, **Working**, and **Finished
(settled, not success)** appearances, all sharing the same target. Choosing an
appearance to edit does not emit a source event. Deleting Pi removes it until
you explicitly add another template. See [Editing layers](layer-editing.md).

### Try it without hardware

The keyboard previews the complete draft through the shared composition runtime.
It is not device feedback.

- The preview-input picker distinguishes **Live data** (real read-only
  observations) from **Private test**, advanced scenarios and isolated demos.
- **Test behavior…** offers contextual choices such as **Idle / Working /
  Finished**. Its selected label names the active test and playback state.
  Private facts do not edit the setup or publish to real connections.
  Finished is a transient settled event, never a success fact.
- **Play test / Pause test** controls private time. Facts have finite lifetimes;
  pausing freezes leases and animations.
- **Preview tools** retains simultaneous-source scenarios, deterministic
  stepping, isolated input demo, coverage and expiry. Quick testing preserves
  those advanced channels.
- **Inspect** makes a key click open its actual **Why this key?** report before
  advanced forms. It never retargets a layer or enables lighting.

Keyboard modes are **Preview** (read-only), **Select keys** (target editing), and
**Inspect** (explanation). Outlines do not replace rendered RGB colors. Missing
source data is not treated as idle or success. Physical targets remain 80HE ANSI;
unsupported keys, remaps and light-bar positions are not guessed. See
[Preview testing](preview-testing.md) for advanced contexts and clocks.

### Save, copy and switch deliberately

**Save** persists the draft with revision checks. If lighting is active, confirm
**Pause and save** explicitly; saving never resumes it. Failures retain your
draft, and late acknowledgements cannot clear newer edits.

The browser owns setup naming, starters, **Save a copy**, opening, renaming and
deleting setups. Copies are stored inactive. Replacing a dirty draft offers
**Save changes / Discard changes / Cancel**. Closing a browser or following a
Connections link is harmless navigation, not replacement.

Offline drafts and local starters remain usable. Saving and remote operations
explain their connectivity requirements. For conflicts, preserve a copy or
explicitly reload the saved version; the GUI never forces an overwrite. See
[Setup saving](setup-saving.md).

### Connections, sharing and existing profiles

A source-driven layer shows a concise note and **Connections: …** link. Recent
data, registration and process health are separate: no recent facts do not prove
an application stopped. Private tests never become live health observations.

Connections starts with reporting apps, session/state counts and **Used by**
layer links. **App details** shows current sessions and setup help; **Advanced
details** contains technical diagnostics. Local connector configuration stays
under **Advanced → Connection administration**, with executable review, trust
and enablement as separate explicit actions. Browsing never starts a producer. **Diagnostics**
and **Settings** are secondary windows.

**Setup-name switcher → Share setups** opens import/export. A new imported copy
requires explicit connection rebinding, including disabled appearances, and is
stored without switching, credentials or executable trust.

Existing legacy profiles offer **Keep current behavior** or **Begin new setup**.
A new local draft does not replace old lighting. Before adoption, confirmation
explains replacement of legacy visuals/notifications/automation in the private
snapshot. Source files and machine-local Connections stay unchanged.

See [Connections and exceptional flows](connection-workflow.md), the legacy
reference below, and the [usability walkthrough](gui-walkthrough.md).

### Legacy compatibility controls

The controller shares the app icon's visual identity: graphite surfaces,
keycap-like cards, a keycap wordmark, cyan controls, and subtle static RGB seams.
Settings uses the same theme. RGB decoration is not an activity indicator and
does not change your lighting palette or settings. Engine state always has a
text label: green for active, amber for reconnecting, pink-red for errors, and
muted for paused/offline. Preview chassis stay neutral; key illumination still
follows the selected effect and draft colors. Keyboard legends stay readable as
the light changes. Native macOS menus retain the platform's styling.

- Opening the GUI reads status and the canonical setup. If unavailable, **Start engine** explicitly
  starts a standalone engine. Fresh state starts paused; existing persisted
  enabled state remains the engine's responsibility. The GUI does not force resume.
- Click an effect card: Ripples, Comet, Spectrum (rainbow), Breathe (breath),
  Matrix, Focus (focus-cockpit), or any of the nine choices below.
  Selection preserves the enabled/paused state;
  the highlighted card follows the engine's confirmed response.
- The **ripple simulation** uses the same renderer, color math, and 80HE matrix
  geometry as the engine. Use **Auto demo**, set synthetic **Pressure**, or
  click/hold mapped keys. It uses draft brightness, colors, and FPS; it is not
  live keyboard input or device feedback. Decorative previews now call the same
  frame renderer as the engine and CLI, sampled on the canonical 80HE LED matrix.
  This replaces the old illustrative keyboard/animation implementation.
  Simulation runs while lighting is paused and never opens an SDK.
- The main page groups **Brightness** (shown as a percentage), **Palette**, **Speed**, and
  **Frame rate** (1–120 FPS) under **Lighting controls**, before the preview and
  effect cards so frame rate stays visible in compact layouts. Spectrum, Comet,
  Matrix, Breathe, and the six new decorative effects use elapsed time: FPS controls sampling smoothness and CPU
  usage, not animation pace. **Speed** (10–400%) adjusts their pace independently;
  100% is the calm default. It is hidden for all key-reactive effects and Focus, and for older
  engines that do not report speed support. Ripple physics and Focus timers are
  unchanged. Use **Apply changes** on
  the main page to save these adjustments. Background polling does not overwrite
  unapplied edits. **Discard changes**, also on the main page, restores the latest
  confirmed engine values locally without fetching them again. Brightness still
  uses the engine's 0–255 range internally.
- At 100% Speed, Matrix advances 7.5 rows/second, Comet 12 keys/second,
  Spectrum completes a hue cycle in 30 seconds, and Breathe cycles in 6 seconds.
  Higher render FPS interpolates between animation steps. Speed edits preserve
  the current phase rather than restarting the effect; previews use draft speed.
- **Palette** appears for Comet, Breathe, all nine new effects, and palette-based Ripples.
  Spectrum uses a fixed rainbow; Matrix uses Terminal green; Focus uses fixed
  phase colors. Their previews ignore the selected palette too. Focus previews
  use the shared Focus renderer with a synthetic blue focus phase at 55%
  progress, not the live timer. Unknown/custom modes explicitly show that a
  preview is unavailable rather than substituting another effect.
- For **Ripples**, enable **Two-tone ripple** and choose **Base color** and
  **Ripple color**, then **Apply changes**. Idle keys keep the base color and
  pressure-driven waves blend toward the ripple color before fading back.
  Brightness scales both colors. Black base keeps the old dark idle look;
  disabling Two-tone ripple restores black idle lighting with palette-based waves.
  The preview renders your draft colors with synthetic input.
  Existing profiles keep their original colors until explicitly changed.
- The fixed header has one primary action: **Start engine**, **Resume lighting**,
  or **Pause lighting**, depending on state. Resume takes lighting control; pause releases
  SDK lighting so the keyboard/Wootility can render again. It does not rewrite
  saved keyboard profiles.
- **Settings → Import a trusted profile** accepts a path only after an explicit trust checkbox.
  **Configurations can execute commands as your user.** Editing the path resets
  acknowledgement. Do not select untrusted downloaded files.
- The main screen shows engine state and lighting errors. **App settings →
  Advanced engine controls** contains recovery controls and counts;
  **Diagnostics → Engine protocol response** contains the last engine JSON.
  Disconnected engine details are marked **Last seen**, not presented as live status.

### App settings

The Settings page is headed **App settings**. In its **Startup** section,
**Check startup status** checks the helper. **Enable login** registers startup
only; **Disable login** removes the registration and stops the managed service.

The collapsed **Advanced engine controls** section contains **Refresh status**,
managed-service start/stop controls and **Stop standalone engine**. Connector
diagnostics and the engine protocol response are in the **Diagnostics** window.
Starting the managed service launches it now; stopping it does not remove login opt-in.
Use **Stop standalone engine** only for an unmanaged engine; service managers may
restart a process stopped directly. These operations are never automatic.

The helper returns `{ok,supported,enabled,running,error?}` JSON. Unsupported
platforms, missing helpers, and failed requests are reported in the window; the
GUI does not silently install or repair a service. The legacy prefix helper
requires Python 3; the self-contained macOS app uses a bundled native helper.
**Updates & removal** remains a separate section in App settings. See
[macOS packaging](macos-release.md) for its explicit update/removal controls.

For a separate engine instance:

```sh
./target/debug/underglow-gui --state-dir /absolute/path/to/test-state
```

This directory is passed unchanged to engine/control subprocesses. The
`WOOTING_STATE_DIR` environment override is also supported; explicit `--state-dir`
takes precedence. Either override disables service controls because the installed
service uses the default platform state directory. With neither override, path
selection is delegated to the CLI. A custom state directory does not bypass the
engine's per-user single-instance guard.

## Implementation and verification

A background worker serializes subprocess operations; UI rendering never waits
for the CLI. Control/service helpers have a 15-second deadline and bounded output.
Status polling is visually silent: it does not dim controls or show a busy
indicator. An explicit action during a poll is queued once behind it, rather
than dropped. Apply/Discard controls retain their layout space while editing.
Nonzero exits with JSON retain their structured error. An explicitly started
engine has detached console streams and is not bound to the window's lifetime.
Its stderr goes to private `engine.log` in the selected runtime directory; early
startup failures include their diagnostic text and log path. Service-managed
engines use the service's documented log destination. A timed-out control may
already have applied its action; refresh status before retrying it. Helpers run
in isolated process groups so timeout cleanup also stops their descendants.
No HTTP server or SDK loader lives in the GUI. The GUI and engine share a pure,
hardware-independent ripple renderer. `make dev` runs the controller with a
supervised, isolated engine; hardware is disabled unless explicitly requested.
See [development workflow](dev.md).

```sh
cargo test --features gui --bin underglow-gui
cargo clippy --features gui --bin underglow-gui -- -D warnings
```

Tests cover response parsing, schema rejection, service diagnostics, exact
subprocess arguments, preserved unapplied settings, nonzero JSON errors, and
helper timeouts, and state-override precedence/service isolation. They do not
open a native window or menu-bar item, start the real engine/service,
or access a keyboard. Visual/native-window behavior and cross-platform packaging
still require manual verification on the target desktop.

## New effect choices

All start at full brightness. Palette names below are fresh-preset defaults;
you can change the palette for every new effect.

| Effect | Appearance | Default palette | Timing at 100% Speed |
| --- | --- | --- | --- |
| Aurora | Broad, slowly folding curtains | Ocean | 24-second drift |
| Embers | Scattered points glow and fade | Ember (`heat`) | Independent 6–12-second cycles |
| Tide | Soft band washes across and retreats | Ocean | 12-second roundtrip |
| Orbit | Hollow luminous arc circles the center | Neon (`cyberpunk`) | 8-second revolution |
| Prism | Two intersecting diagonal band patterns | Neon (`cyberpunk`) | 12-second combined cycle |
| Radar | Radial sweep with fading angular trail | Terminal | 6-second revolution |
| Constellation | Pressed keys become stars with fading links | Ocean | Pressure-driven; 2.4-second fade half-life |
| Heatmap | Repeated presses warm keys; idle activity cools | Ember (`heat`) | 20-second cooling half-life |
| Afterimage | Stationary soft glow lingers around each touch | Neon (`cyberpunk`) | Pressure-driven; 1.4-second fade half-life |

The last three use real elapsed time, not the Speed control. A held Heatmap key
counts once, not once per frame; release and press again to add warmth. Links in
Constellation connect current fading stars, not a stored sequence of keystrokes.
All activity is bounded, in memory only, and cleared on session close (including
pause, mode replacement, and recovery). Settings persist; activity does not.

Their live input currently supports one **80HE ANSI** keyboard, using the same
typing-block and Space mapping as Ripples. Function/navigation/custom keys are
not mapped; other layouts are rejected rather than guessed. The GUI uses only
synthetic input: toggle **Auto demo**, click/hold mapped typing keys, adjust
**Pressure** (key travel), or use **Clear activity**. Clearing also stops the
auto demo. Switching modes discards the old simulation. Preview activity is
never sent to the engine and never reads physical keystrokes.
