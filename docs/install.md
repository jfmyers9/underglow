# User installation and release packaging

For the self-contained Finder app and `.dmg`, use the
[macOS app workflow](macos-release.md). It needs no user-installed Python or
prefix. The older archive/prefix workflow below remains for Linux and CLI users.

## Contract and prerequisites

macOS and Linux per-user installs use a dedicated prefix, by default
`~/.local/opt/wooting-signals`. No sudo, service activation, hardware probe, or
profile change occurs during installation. Python **3.9+** is required by the
installer and `wooting-service`; it is a documented system prerequisite, not
bundled. Linux additionally needs a working desktop session for GUI use and
systemd's user manager for login integration. OS frameworks, glibc/ELF loader,
GPU drivers, display server, and dynamically discovered platform plugins remain
system prerequisites. Native linked non-system dependencies are bundled.

```
<prefix>/bin/wooting-signals
<prefix>/bin/wooting-gui                 # optional
<prefix>/bin/wooting-service             # JSON helper, Python3
<prefix>/lib/wooting-signals/            # both SDKs + native dependency closure
<prefix>/share/wooting-signals/          # notices, licenses, examples
```

Both SDKs are required in release packages, including Analog SDK 0.9.1 for
ripples. Libraries and services reference the installed prefix, never a checkout.
SDK names are `libwooting-rgb-sdk.dylib` and
`libwooting_analog_sdk_dist.dylib` on macOS, `.so` on Linux.

## Install a prepared archive

Verify its `.sha256` against a trusted release source (a checksum is not a
signature), extract, then:

```sh
cd wooting-signals
python3 install.py install                    # dry run
python3 install.py install --apply            # explicit filesystem changes
~/.local/opt/wooting-signals/bin/wooting-service status
```

Use `--prefix /absolute/dedicated/directory` to override. Add `<prefix>/bin` to
PATH yourself; the installer does not alter shell configuration or existing
binaries in `~/.local/bin`. From a source checkout the equivalent is
`scripts/install-macos.sh --package /path/to/extracted/wooting-signals --apply`
(or `scripts/install-linux.sh`). The old `--bootstrap` switch is deliberately
removed: login and immediate start are separate explicit decisions.

With a GUI binary, installation creates `~/Applications/Wooting Signals.app` on
macOS or an XDG user applications desktop entry on Linux. These launch the GUI,
not a hardware effect. The macOS app is a local launcher, not a self-contained,
Developer-ID-signed or notarized application. Do not move the installed prefix
without reinstalling its integration. The optional macOS GUI provides a WS
menu-bar launcher; Linux uses the installed desktop/settings-window fallback.

## Opt-in engine/login control

```sh
wooting-service enable     # next-login opt-in; does NOT start now
wooting-service start      # explicit start now; does NOT opt in at login
wooting-service status
wooting-signals control status
wooting-signals control select --preset ripples
wooting-signals control resume
wooting-signals control pause
wooting-service stop       # stops managed engine; keeps login preference
wooting-service disable    # stops managed engine and removes login preference
```

A fresh engine starts paused. On later starts its saved settings are retained;
do not assume restarting an existing state is equivalent to pausing it.
Services do not pass a hard-coded config on every startup (that would overwrite
saved selection). The GUI calls the exact sibling helper, not a PATH search.
Every helper response is one JSON object:
`{"ok":true,"supported":true,"enabled":false,"running":false}`.
Errors add `error`, set `ok:false`, and exit nonzero. `status` when stopped exits
zero. `running` describes the service-managed engine, not arbitrary foreground
engines. Foreground engine lifecycle remains `control stop` / terminal signals.

State is outside the install prefix:

- macOS: `~/Library/Application Support/wooting-signals/runtime`
- Linux: `$XDG_STATE_HOME/wooting-signals`, default `~/.local/state/wooting-signals`

The helper intentionally uses default state paths only. Custom `--state-dir`
engines are not managed by this helper. Mutating service operations reject a
nonempty `WOOTING_STATE_DIR`; unset it before managing the default login engine. Existing config files are never replaced;
select one explicitly with `control select --config /absolute/config.toml`.
Uninstall does not remove config, state, or logs. No udev rules, privileged
permissions, or macOS input-monitoring permissions are automatically changed.
Linux HID access may need administrator-installed vendor udev rules; the engine
must never be run as root merely to work around permissions.

## Upgrade and uninstall

First close the GUI, stop any custom-state engine, then `wooting-service stop`
and `wooting-signals control stop` for any independently started default engine.
Install the new extracted archive into the same prefix. Upgrades refuse a
reachable default engine, stage the new files, then swap the dedicated prefix;
config/state and login preference remain untouched. Installation never restarts
an engine. A later explicit start may restore the saved active state.

To uninstall, `wooting-service disable`, stop any foreground engine, then:

```sh
python3 install.py uninstall --apply          # same --prefix if customized
```

An unmanaged prefix is rejected. Installation/uninstallation also rejects a
nonempty `WOOTING_STATE_DIR`, so its engine checks cannot target a different
instance. Uninstall requires removing even an inactive service registration
with `wooting-service disable`. Config/state/logs remain for reinstall. Old
pre-engine `~/.local/bin/wooting-{signals,extension,hack}` aliases and legacy
LaunchAgents are not migrated or removed automatically. Stop/remove those
manually before using the new engine to avoid two RGB owners.

## Maintainer: build and package (no publishing)

Build on the target OS/architecture. Rust toolchain and SDK build dependencies
are build-time prerequisites. Use `cargo build --release --locked` for CLI or
`cargo build --release --locked --features gui --bins` for CLI + GUI. Build the
RGB SDK at the pinned submodule revision; obtain the matching official Analog
SDK **0.9.1 distributable**, verify its provenance, and retain its licenses.
Do not substitute the Analog SDK plugin or an old wrapper library.

Prepare a release-specific notices directory using `packaging/NOTICES.md` and
`packaging/licenses/MPL-2.0.txt` as a starting point. Include actual copyright and
license texts for all Rust dependencies, embedded SDK dependencies, and copied
native dependencies, plus applicable source/relinking information. Both SDKs'
actual licenses are MPL-2.0, not MIT. Record the exact revision/source of local
SDK changes and distribute their covered source. A `native-licenses.json` map
must cover every native basename, e.g.:

```json
{
  "wooting-signals": "application-and-rust-notices.txt",
  "wooting-gui": "application-and-rust-notices.txt",
  "libwooting-rgb-sdk.dylib": "rgb-notices.txt",
  "libwooting_analog_sdk_dist.dylib": "analog-and-embedded-notices.txt",
  "libhidapi.0.dylib": "hidapi-notices.txt",
  "libusb-1.0.0.dylib": "libusb-notices.txt"
}
```

```sh
scripts/package-release.sh \
  --rgb-sdk external/wooting-rgb-sdk/mac/libwooting-rgb-sdk.dylib \
  --analog-sdk /verified/analog-0.9.1/release/libwooting_analog_sdk_dist.dylib \
  --gui target/release/wooting-gui \
  --notices /reviewed/release-notices \
  --output /tmp/wooting-signals-macos-arm64.tar.gz
```

On Linux use the built `.so` inputs and install build-time `patchelf`. Packaging
recursively copies dependencies discovered with `otool -L` / `ldd`, fails on
unresolved dependencies/name collisions, and rewrites loader paths relative to
the installed files. **Use trusted build inputs only** (`ldd` can execute code).
macOS uses `install_name_tool` and re-applies local ad-hoc signatures solely for
Mach-O integrity; these are not Developer ID signatures or notarization.
Linux retains system glibc/loader, so build on the oldest supported distribution
and test there. Dynamically loaded GUI platform/driver dependencies require a
normal graphical OS installation; bundling is not an OS image.

Archives include a file-hash/dependency manifest and an external SHA-256 file.
The tool never installs, starts services, initializes SDKs, signs with an
identity, notarizes, or publishes. `--gui` is optional; omitting it produces CLI
packaging with the same complete SDK payload.

## Verification scope

`python3 -m unittest discover -s packaging/tests -v` uses temporary HOME/XDG
paths, mock binaries/service tools, and synthetic SDK dependency trees. CI runs
these checks plus shell syntax checks on macOS/Linux; it does not publish
artifacts or touch keyboards. Physical Linux operation, signed/notarized Mac
artifacts, and a real clean-machine release installation remain release gates,
not claims made by the automated fixture tests.

To include the real packaged-CLI integration checks locally:

```sh
cargo build --locked --bin wooting-signals
WOOTING_TEST_BINARY="$PWD/target/debug/wooting-signals" \
  python3 -m unittest discover -s packaging/tests -v
```

These tests copy the real CLI into a temporary install prefix, compile synthetic
SDK ABI fixtures, and use authoritative SDK overrides plus fake service tools.
They verify paused startup, saved settings after upgrade, live-engine upgrade
refusal, preserved state after uninstall, generated service arguments, and
SIGTERM cleanup of mock resources. No host service or device is accessed.
