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
launcher. The platform launcher plus **Startup and service settings** is also the
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
- The animated keyboard is an **illustrative simulation**, not live input,
  exact engine frames, or proof that a keyboard is connected. Preview animation
  continues while lighting is paused and never opens an SDK.
- Adjust brightness (shown as a percentage) and palette, then **Apply changes**.
  Frame rate (1–120 FPS) lives in **Settings**. Background polling does not
  overwrite unapplied edits. **Reload settings** discards edits and fetches the
  engine's values. Brightness still uses the engine's 0–255 range internally.
- The fixed header has one primary action: **Start engine**, **Resume lighting**,
  or **Pause lighting**, depending on state. Resume takes lighting control; pause releases
  SDK lighting so the keyboard/Wootility can render again. It does not rewrite
  saved keyboard profiles.
- **Settings → Import a trusted profile** accepts a path only after an explicit trust checkbox.
  **Configurations can execute commands as your user.** Editing the path resets
  acknowledgement. Do not select untrusted downloaded files.
- The main screen shows engine state and lighting errors. Settings contains the
  detailed connection message, recovery count, and last engine JSON response.
  Disconnected engine details are marked **Last seen**, not presented as live status.

### Login / service settings

In Settings, **Check service** checks the helper. **Enable login** registers startup only;
**Start service** launches now. **Disable login** removes the registration and
stops the managed service. **Stop service** stops it without removing login opt-in.
Use **Stop standalone engine** only for an unmanaged engine; service managers may
restart a process stopped directly. These operations are never automatic.

The helper returns `{ok,supported,enabled,running,error?}` JSON. Unsupported
platforms, missing helpers, and failed requests are reported in the window; the
GUI does not silently install or repair a service. The legacy prefix helper
requires Python 3; the self-contained macOS app uses a bundled native helper.
See [macOS packaging](macos-release.md) for explicit update/removal controls.

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
No HTTP server, SDK loader, or lighting code lives in the GUI.

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
