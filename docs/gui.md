# Native controller

`wooting-gui` is an optional Rust/eframe desktop window, not a browser or an SDK
client. It invokes the **sibling** `wooting-signals` executable for engine/control
commands and the sibling `wooting-service` helper for login/service management.
Install matching versions together; it deliberately does not search `$PATH`.

## Build and launch

```sh
cargo build --features gui --bins
./target/debug/wooting-gui
```

Building alone does not start the engine or access keyboard hardware. Ordinary
`cargo build` and `cargo run -- …` remain headless CLI operations; eframe is only
compiled with the `gui` feature. Linux GUI builds require native window-system
build dependencies for X11/Wayland and a working OpenGL desktop. macOS uses the
native Cocoa window system. Platform service installation is separate; see the
packaging/service documentation.

On macOS, the **WS** menu-bar item offers **Open controller** and **Quit controller
(leave engine running)**. Closing the window hides it while the menu-bar item
remains; use the menu to reopen it or quit. The menu-bar launcher is built only
on macOS through an optional, GUI-only `tray-icon` dependency.

Linux has no tray dependency: reopen the controller using its installed desktop
launcher. The platform launcher plus **App settings** is also the
fallback if macOS menu-bar creation fails (reported on stderr and in the window).
Closing or quitting never pauses or kills the engine, and the controller never
registers login startup automatically.

## Controls

- Opening the GUI only polls status. If unavailable, **Start engine** explicitly
  starts a standalone engine. Fresh state starts paused; existing persisted
  enabled state remains the engine's responsibility. The GUI does not force resume.
- Click an effect card: Ripples, Comet, Spectrum (rainbow), Breathe (breath),
  Matrix, or Focus (focus-cockpit). Selection preserves the enabled/paused state;
  the highlighted card follows the engine's confirmed response.
- The **ripple simulation** uses the same renderer, color math, and 80HE matrix
  geometry as the engine. Use **Auto demo**, set synthetic **Pressure**, or
  click/hold mapped keys. It uses draft brightness, colors, and FPS; it is not
  live keyboard input or device feedback. Other effects retain illustrative
  previews. Simulation runs while lighting is paused and never opens an SDK.
- The main page groups **Brightness** (shown as a percentage), **Palette**, and
  **Frame rate** (1–120 FPS) under **Lighting controls**, before the preview and
  effect cards so frame rate stays visible in compact layouts. More FPS can mean
  smoother or faster motion, with higher CPU usage: Spectrum, Comet, Matrix, and
  Breathe advance per tick, while Ripples uses real time. Use **Apply changes** on
  the main page to save these adjustments. Background polling does not overwrite
  unapplied edits. **Discard changes**, also on the main page, restores the latest
  confirmed engine values locally without fetching them again. Brightness still
  uses the engine's 0–255 range internally.
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
  Advanced engine controls** contains the detailed connection message, recovery
  count, and last engine JSON response.
  Disconnected engine details are marked **Last seen**, not presented as live status.

### App settings

The Settings page is headed **App settings**. In its **Startup** section,
**Check startup status** checks the helper. **Enable login** registers startup
only; **Disable login** removes the registration and stops the managed service.

The collapsed **Advanced engine controls** section contains **Refresh status**,
managed-service start/stop controls, **Stop standalone engine**, and diagnostics.
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
./target/debug/wooting-gui --state-dir /absolute/path/to/test-state
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
cargo test --features gui --bin wooting-gui
cargo clippy --features gui --bin wooting-gui -- -D warnings
```

Tests cover response parsing, schema rejection, service diagnostics, exact
subprocess arguments, preserved unapplied settings, nonzero JSON errors, and
helper timeouts, and state-override precedence/service isolation. They do not
open a native window or menu-bar item, start the real engine/service,
or access a keyboard. Visual/native-window behavior and cross-platform packaging
still require manual verification on the target desktop.
