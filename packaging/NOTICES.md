# Distribution notices (maintainer template)

See `docs/license-audit.md` and `packaging/licenses/review-findings.json` for the
current collected evidence and unresolved upstream rights question. This
template is not a release attestation. The macOS verified-inputs gate requires
hash-bound `audit.json`, the actual application license, and no unresolved findings.

Wooting Signals is declared MIT in Cargo.toml. Retain the project's copyright
and license text in a release. This project is not an official Wooting product.

## Wooting RGB SDK

Copyright Wooting and contributors. Mozilla Public License 2.0; full text in
`MPL-2.0.txt`. Source: https://github.com/WootingKb/wooting-rgb-sdk
The exact submodule source revision is recorded in `manifest.json`.
Source for that revision is available at
`https://github.com/WootingKb/wooting-rgb-sdk/tree/<rgb_source_revision>`.
Retain notices in source files. Make any modifications to MPL-covered files
available as Source Code Form under MPL 2.0, and tell recipients how to get them.

## Wooting Analog SDK 0.9.1

Copyright Wooting and contributors. Mozilla Public License 2.0; full text in
`MPL-2.0.txt`. License verified against the upstream v0.9.1 LICENSE.
Source: https://github.com/WootingKb/wooting-analog-sdk/tree/v0.9.1
Source archive: https://github.com/WootingKb/wooting-analog-sdk/archive/refs/tags/v0.9.1.tar.gz
Official binaries: https://github.com/WootingKb/wooting-analog-sdk/releases/tag/v0.9.1
Retain notices and provide source for any changes to covered files.

## Required release-specific additions

This template alone is NOT a complete redistribution notice set. The release
maintainer must include copyright/license notices for the application's Rust
dependencies (including the optional GUI), Analog SDK's embedded dependencies,
and every copied native library (commonly hidapi and libusb). Honor selected
license terms, including LGPL source/relinking requirements where applicable.
Include exact source URLs/versions and corresponding source or source offers
where required. Do not assume the SDK's MPL covers its embedded dependencies.

The packaging command requires `native-licenses.json` mapping each copied native
file basename (including the two SDKs and application executables) to an existing
notice/license file in this directory. This is a mechanical completeness check,
not a substitute for legal review. Examples and binaries are not published by CI.
