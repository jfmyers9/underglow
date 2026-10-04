# Self-contained macOS app

The `.app` bundles the GUI, engine, native service helper, both SDKs, and their
linked non-system dependencies. Users need no terminal, Python, Homebrew,
environment variables, or separate SDK downloads.

## Installation and lifecycle

1. Open the `.dmg`, drag **Wooting Signals.app** into **Applications**, eject the
   image, and launch the installed app. Do not enable login from a disk image.
2. Click **Start engine**, choose an effect, and **Resume lighting**. Fresh state
   starts paused; existing saved enabled lighting may resume on startup.
3. Review any macOS keyboard/input-monitoring permission request in System
   Settings. Permissions are never granted automatically.

Closing the window leaves the engine running. Pause returns lighting control;
the **WS** menu quits the controller. Login startup is always opt-in.

- **Settings → Enable login** registers next-login startup without starting now.
  **Advanced engine controls → Start managed service** starts now;
  **Disable login** stops and unregisters it.
- Before replacing the app, click **Stop engines for update**, wait for success,
  and quit the controller. Replace in the same location, then relaunch. Settings
  and login preference survive. Restarting may restore enabled lighting; pause
  first if undesired. There is no automatic updater.
- SDK discovery survives moving the app. Login registration contains an absolute
  path: stop before moving, then explicitly re-enable from the new location.
  Stale registrations are reported, not silently repaired.
- Before trashing the app: **Settings → Remove this app → Disable login & stop
  engines**, wait for success, then quit. Trashing a running app alone cannot
  reliably stop its engine or remove its LaunchAgent. Custom-state engines must
  be stopped separately; these maintenance controls manage default state only.

Settings/logs remain in `~/Library/Application Support/wooting-signals/runtime`.
The registration is `~/Library/LaunchAgents/io.github.jfmyers9.wooting-signals.plist`.

**Local-test builds are ad-hoc signed, not notarized or for public distribution.**
Gatekeeper may block quarantined downloads. Do not disable Gatekeeper globally.
After license review, an ad-hoc prerelease is an option with explicit warnings;
Open Anyway may be required and is not guaranteed on managed Macs. Developer ID
and notarization provide the normal lower-friction public delivery experience.

## Build a local candidate

Build on macOS. Python 3.9+ and Apple command-line tools are build-time only.
Use trusted native inputs and a new output directory (overwrites are rejected):

```sh
cargo build --release --locked --features gui --bins
scripts/package-macos.sh \
  --binary target/release/wooting-signals \
  --gui target/release/wooting-gui \
  --service target/release/wooting-service \
  --rgb-sdk /trusted/libwooting-rgb-sdk.dylib \
  --analog-sdk /trusted/libwooting_analog_sdk_dist.dylib \
  --notices /local/candidate-notices \
  --version 0.1.0 --identifier io.github.jfmyers9.wooting-signals \
  --architecture arm64 --minimum-os 26.0 \
  --rgb-sdk-version 451f40dc056e0edefccdaba7807a72b694520e92 \
  --analog-sdk-version 0.9.1 \
  --local-test --output target/macos-local
```

Set the version to match the shipped code. Every native input/dependency must
support the declared architecture and minimum OS; `universal2` requires both
arm64 and x86_64 everywhere. Current local Homebrew RGB/hidapi/libusb builds
require **macOS 26.0**. Supporting older Macs requires rebuilding those inputs,
not lowering the plist value. The packager rejects incompatible payloads.

Notices require `NOTICES.md`, `MPL-2.0.txt`, and `native-licenses.json` covering
all three executables, SDKs, and native dependencies. See [notice requirements](install.md#maintainer-build-and-package-no-publishing).
`--local-test` permits pending review, not missing mapped files: record license
gaps honestly and do not distribute. It forbids Developer ID/notarization.

Output contains the `.app`, versioned `.dmg`, `SHA256SUMS`, and provenance.
Executables live under `Contents/MacOS`, SDKs under `Contents/Frameworks`, and
icon/notices/examples under `Contents/Resources`. Provenance records source
revision/dirty state, input hashes, supplied SDK versions, minimum OS,
architecture, review status, and signing mode. The image has an Applications
shortcut. Packaging never mounts, installs, launches effects, or enables services.

## Signing and distribution

After completing notice/provenance review, replace `--local-test` with
`--verified-inputs`. This now requires a hash-bound `notices/audit.json`, not just
a nonempty notice mapping. See the [audit result and reproduction steps](license-audit.md);
the upstream bindings caveat is documented as non-blocking, not legally resolved. For Developer ID
delivery, additionally supply:

```sh
--sign-identity 'Developer ID Application: YOUR NAME (TEAMID)' \
--notary-profile 'your-existing-keychain-profile'
```

Configure certificates and the notarytool keychain profile outside the repository;
never commit credentials. Signing uses hardened runtime and timestamps, nested
code first. Explicit notarization sends the app and disk image to Apple, waits
for acceptance, then staples/validates both. Credentials are never used implicitly.
Developer ID without notarization is recorded as unnotarized, not Gatekeeper-ready.

`.github/workflows/macos-release.yml` is **manual-only and never publishes**.
It requires reviewed-input acknowledgement and a checksum-pinned HTTPS tar.gz
containing root-level `rgb-sdk.dylib`, `analog-sdk.dylib`, and `notices/` with all
required files. Optional dependencies must have resolvable loader-relative
references. No enclosing folder, `./` entries, links, or special files are allowed.
Example: `tar -czf inputs.tar.gz rgb-sdk.dylib analog-sdk.dylib notices`.
Download/extraction limits and SHA-256 are enforced. Inputs must match the
runner's native architecture and deployment target. CI tests/builds and uploads
an **ad-hoc local-test** DMG/checksum artifact, with no signing credentials or
publishing token. Public GitHub Releases require separate explicit approval.

## Remaining acceptance gates

Fixtures test relocation, SDK discovery, state persistence, fake service
management, signing/notarization sequencing, and actual ad-hoc DMG creation.
They do not certify the real release:

- Review complete Rust/native/embedded SDK notices and source/relinking obligations.
- Test the actual quarantined download on a clean supported Mac without a checkout
  or development tools; verify Finder launch, permissions, RGB and analog effects.
- Test login/logout, upgrade, moved-app registration, removal, reconnect, sleep/wake.
- Obtain Developer ID credentials and perform real notarization/Gatekeeper checks.
  Until then, notarization is fixture-tested only; nothing is published.
