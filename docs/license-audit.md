# macOS dependency notice audit

## Underglow rename

Historical evidence below describes the pre-rename collection, not approval of
new Underglow artifacts. Recollect application/runtime notices and assemble a new
hash-bound audit for `underglow`, `underglow-gui`, and `underglow-service`.
Old receipts do not authorize renamed/rebuilt binaries: packaging requires exact
native input names and hashes and fails closed on a mismatch. Original license
wording, upstream SDK names, and historical evidence remain unchanged.

## Result and scope

The current arm64/macOS inputs have complete **technical notice collection**:

| Component | Evidence/materials collected |
| --- | --- |
| Application Rust graph | 225 registry crates, exact archive/lockfile checksums, pinned license policies |
| Analog SDK Rust graph | 44 reachable registry crates, `ffi,dist` release features; no `virtual-input` |
| Rust runtimes | 1.94.1 application / 1.91.1 SDK, standard-library copyright and exception notices |
| Wooting RGB SDK | MPL-2.0, revision `451f40dc056e0edefccdaba7807a72b694520e92`, complete source |
| Wooting Analog SDK | MPL-2.0, v0.9.1 source/lock/workflow and exact official binary-archive/member identity |
| hidapi 0.15.0 | BSD-3-Clause alternative selected; complete source, attribution and build evidence |
| libusb 1.0.30 | LGPL-2.1-or-later, complete source, build/replacement instructions and acceptance evidence |

Cargo collection is a conservative graph including optional/build/development
dependencies, **not** a claim that every recorded crate is linked into the app.
Native correspondence is normal upstream-release/build evidence, not a claim
of byte-reproducible builds. This is an engineering audit, not a legal opinion.

### Documented, non-blocking upstream caveat

The `objc2` family’s upstream [LICENSE.md at the pinned revision](https://github.com/madsmtm/objc2/blob/7b1abfd750a2cacaea71d6a56ecfb83cb7de560b/LICENSE.md)
declares the normal open-source licenses, but also says:

> These crates are derived from Apple SDKs shipped with Xcode.
> From reading the license, it is unclear whether distributing derived works
> such as these crates are allowed?

The current upstream license retains that caveat. [Issue #23](https://github.com/madsmtm/objc2/issues/23)
concerns contributor relicensing, not Apple’s permission. The public
[Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf), EA2002 dated
06/08/2026, permits compliant macOS app/library distribution in §2.4 and reserves
Apple Software rights in §§2.6–2.7. Those clauses do not expressly resolve the
generated-bindings question. The current public document is not proof of which
agreement was accepted for the local Apple command-line tools.

**This is not evidence that distributing this application is prohibited.** The
project owner has accepted keeping this as a documented, non-blocking caveat for
the current ad-hoc prerelease. It is not a claim that the underlying legal
question has been resolved or that Apple has granted additional rights. Revisit
the decision if concrete contrary evidence or changed upstream terms appear.

`packaging/licenses/review-findings.json` retains the evidence and disposition in
review notes. With no blocking findings, the regenerated `audit.json` is
**reviewed**, not legally certified. Exact input, notice, source, architecture,
and license checks remain enforced by `package-macos.sh --verified-inputs`.
Future genuinely unresolved findings still fail that gate. Nothing is published
by these tools.

## How missing notice text was handled

The initial filename-only pass missed real upstream license files, flattened
license symlinks, font terms, embedded C notices, and runtime notices. The new
collector verifies actual `.crate` archives against Cargo.lock before reading
them, and checks explicitly reviewed file hashes for each package/version.
Missing texts recovered from upstream are pinned to full commits and SHA-256.

Seventeen application records explicitly use `metadata-declaration-with-source`:
the original package declares SPDX terms but lacks complete packaged text. The
complete checksum-verified source package and its actual author/license metadata
are retained, alongside separately labeled canonical terms. **No copyright
holder or year was invented.** This is distinct from a verbatim upstream notice;
the package manifests document that choice. It does not waive upstream caveats.
The retained source packages are optional evidence for these permissive crates,
not a blanket MIT requirement to redistribute source. They are separate from
the corresponding native source supplied for MPL/LGPL obligations.

Nested font, ring/BoringSSL/fiat, and embedded hidapi notices are retained. Native
source archives travel with the notice bundle rather than relying solely on
mutable source links. libusb replacement instructions preserve modification and
debugging rights; the project MIT license imposes no contrary EULA restriction.

A modified libusb was rebuilt, substituted into an isolated copy of the app,
ad-hoc signed, and verified with deep/strict signature checks. Its modified
version marker was read through `libusb_get_version` without initializing USB.
The original app was unchanged. This checks replacement/loading, **not** actual
keyboard operation with the modified library or a future hardened-runtime build.

## Reproduce collection

Audit tools require **Python 3.11+**, Cargo and trusted source inputs. They are
maintainer tools only; the installed app still requires no Python. Outputs must
be new directories. Package versions, hashes and source pins are explicit in
`packaging/licenses/{rust-overrides,native-inputs}.json`; a changed input fails
closed and needs a refreshed review, not merely another `--verified-inputs` flag.

Generate Cargo metadata after fetching the appropriate locked dependencies:

```sh
cargo metadata --locked --all-features --filter-platform aarch64-apple-darwin \
  --format-version 1 > /tmp/wooting-app-metadata.json
cargo metadata --locked --manifest-path /path/to/analog-source/Cargo.toml \
  --filter-platform aarch64-apple-darwin \
  --features wooting-analog-sdk/ffi,wooting-analog-sdk/dist \
  --format-version 1 > /tmp/wooting-analog-metadata.json

python3 packaging/collect_rust_notices.py --metadata /tmp/wooting-app-metadata.json \
  --output target/license-audit/application-rust
python3 packaging/collect_rust_notices.py --metadata /tmp/wooting-analog-metadata.json \
  --root-package wooting-analog-sdk --output target/license-audit/analog-rust

python3 packaging/collect_native_notices.py \
  --inputs /path/to/actual-inputs.json --provenance /path/to/original/provenance.json \
  --homebrew-prefix /opt/homebrew --cache target/license-audit/native/sources \
  --output target/license-audit/native/collected

python3 packaging/collect_runtime_notices.py \
  --binary target/release/underglow --binary target/release/underglow-gui \
  --binary target/release/underglow-service --analog-sdk /path/to/analog-sdk.dylib \
  --cache target/license-audit/toolchain-sources --output target/license-audit/runtime
```

The native inputs JSON maps `rgb`, `analog`, `hidapi`, and `libusb` to original
dylib paths. Original packaging provenance means pre-relocation input hashes,
not hashes of rewritten/re-signed libraries inside the app. The native collector
pins the current Homebrew formula/receipt/SBOM and validates the actual Cellar
binaries. For other builds, update the reviewed pins and evidence deliberately.
Runtime collection downloads checksum-pinned Rust archives but extracts only
notices; downloaded compilers are never installed or executed.

Assemble the component collections (the existing local native collection with
replacement-test evidence is `target/license-audit/native/collected-final`):

```sh
python3 packaging/assemble_notices.py \
  --application-rust target/license-audit/application-rust \
  --analog-rust target/license-audit/analog-rust \
  --runtime target/license-audit/runtime \
  --native target/license-audit/native/collected-final \
  --binary target/release/underglow --gui target/release/underglow-gui \
  --service target/release/underglow-service --output target/license-audit/notices
```

This validates collection hashes, lockfiles, policy and runtime/binary identity,
copies original notices/source, includes James Myers’s MIT license, and emits
hash-bound `audit.json`/`native-licenses.json`. It preserves unresolved findings;
it never turns “collector complete” into automatic release clearance.

Record resolutions or explicitly accepted non-blocking caveats with supporting
evidence and a review disposition, then regenerate the bundle. Acceptance is
not a claim that additional legal permission was obtained. A verified package requires exact
matches for the full notice tree, target architecture, application license, and
every original native input. Editing a notice or rebuilding a binary invalidates
that attestation. Reviewed ad-hoc candidates are labeled **ad-hoc-unnotarized**;
local tests remain **not for distribution**. Neither mode publishes anything.

CI still builds **local-test-only** candidates: recompiling creates new binaries,
so CI cannot inherit a previous build’s hash-bound release attestation. Publishing
or notarization requires separate authorization and appropriate acceptance tests.
