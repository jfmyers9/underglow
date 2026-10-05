#!/usr/bin/env python3
"""Collect pinned native sources/notices without loading libraries or approving release.

The pin manifest is a reviewed input, not inferred from arbitrary binary filenames.
Source archives are retained intact; only explicitly selected regular files are read.
"""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile
import urllib.request

MAX_ARCHIVE_BYTES = 128 * 1024 * 1024
MAX_MEMBER_BYTES = 64 * 1024 * 1024
REQUEST_TIMEOUT_SECONDS = 90


class HTTPSRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not newurl.startswith('https://'):
            raise ValueError('source redirects must use HTTPS')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def bounded_read(stream, limit, label):
    data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f'size limit exceeded: {label}')
    return data


def digest(data):
    return hashlib.sha256(data).hexdigest()


def checked(data, expected, label):
    if digest(data) != expected:
        raise ValueError(f'SHA256 mismatch: {label}')
    return data


def relative(name):
    p = PurePosixPath(name)
    if not name or p.is_absolute() or '..' in p.parts or '\\' in name:
        raise ValueError(f'unsafe path: {name}')
    return p


def fetch(pin, cache):
    name = relative(pin['file'])
    if len(name.parts) != 1:
        raise ValueError('archive cache names must be basenames')
    path = cache / str(name)
    if path.exists():
        with path.open('rb') as stream:
            checked(bounded_read(stream, MAX_ARCHIVE_BYTES, str(path)), pin['sha256'], str(path))
    else:
        if not pin['url'].startswith('https://'):
            raise ValueError('source URLs must use HTTPS')
        opener = urllib.request.build_opener(HTTPSRedirectHandler())
        with opener.open(pin['url'], timeout=REQUEST_TIMEOUT_SECONDS) as response:
            if not response.geturl().startswith('https://'):
                raise ValueError('source response must use HTTPS')
            data = bounded_read(response, MAX_ARCHIVE_BYTES, pin['url'])
        checked(data, pin['sha256'], pin['url'])
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return path


def member_bytes(archive, name):
    relative(name)
    with tarfile.open(archive) as tar:
        matches = [m for m in tar.getmembers() if m.name == name]
        if len(matches) != 1 or not matches[0].isfile():
            raise ValueError(f'expected exactly one regular archive member: {name}')
        if matches[0].size > MAX_MEMBER_BYTES:
            raise ValueError(f'size limit exceeded: {name}')
        with tar.extractfile(matches[0]) as stream:
            return bounded_read(stream, MAX_MEMBER_BYTES, name)


REBUILDING = '''# Native libraries: source, notices, and replacement

This directory contains original source archives, selected original notices and
build files, and checksum/provenance records. No source modifications were made
to these SDKs/libraries. The app packager changes Mach-O install names/rpaths and
code signatures only. Full source-file copyright notices remain in the archives.
This engineering inventory is not a legal opinion or whole-release approval.

## Wooting SDKs (MPL 2.0)

RGB: Copyright 2018 Wooting Technologies B.V.; retain the original headers and
MPL text. Source is the pinned RGB archive. Extract it; on macOS with clang,
pkg-config, hidapi and libusb development files installed, run `make -C mac`.
`notices/rgb/mac/Makefile` gives the exact upstream build recipe. The observed
local binary is associated with a clean submodule at that revision; no original
compiler build log or byte-reproducible-build claim is provided.

Analog: source and upstream binary archives are pinned separately. The input
library must equal the release archive member byte-for-byte. The source tag,
Cargo.lock, release workflow and cargo alias associate that release with the
source graph. Upstream uses Rust 1.91.1, release profile, features `ffi,dist`;
`virtual-input` is NOT enabled for the release dist library. Extract source and
run `cargo rustc --locked -p wooting-analog-sdk --features ffi,dist --release --
-C link-arg=-Wl,-install_name,./target/release/libwooting_analog_sdk_dist.dylib
-C link-arg=-Wl,-o,./target/release/libwooting_analog_sdk_dist.dylib` (one command)
from its root with Rust 1.91.1 and Apple command-line tools on Apple Silicon.
See original `docs/BUILD.md` and
`.cargo/config.toml` for upstream commands. No SDK is initialized by this audit.
All observed embedded crate-version paths agree with Cargo.lock, but strings
are partial corroboration, not a complete binary dependency inventory or a
reproducible-build proof. Collect the locked SDK Rust graph's own licenses and
the Rust standard-library notices separately. Its hidapi crate embeds a native
macOS HID backend; MPL does not replace those dependencies' license terms.

MPL-covered Source Code Form is supplied in the adjacent pinned source archives.
Retain notices; distribute modifications to covered files under MPL 2.0 and
provide recipients the corresponding modified source when distributing binaries.

## hidapi 0.15.0

Selected alternative: BSD-3-Clause (not GPL). `LICENSE.txt` explains the choice;
`LICENSE-bsd.txt`, AUTHORS and mac/hid.c preserve original terms and attribution.
Extract source, then `cmake -S . -B build -DHIDAPI_BUILD_HIDTEST=ON`,
`cmake --build build`, `cmake --install build --prefix "$HOME/hidapi-local"`.
The captured Homebrew formula records original distribution build flags.

## libusb 1.0.30 (LGPL 2.1 or later)

libusb is a separate, dynamically linked dylib; no libusb static objects are
linked into Underglow. The complete unmodified corresponding upstream
source archive, COPYING, AUTHORS, INSTALL and Homebrew build recipe accompany
this inventory. Extract source, then run
`./configure --disable-dependency-tracking --prefix="$HOME/libusb-local"`
followed by `make` and `make install` with Apple command-line tools installed.
Homebrew formula/receipt/SBOM identify the distributed input and source checksum;
the captured formula has no source patches. Original full sources retain all
per-file notices. Keep source available with the binary download, not only a
mutable upstream link. Do not impose terms forbidding reverse engineering for
debugging user modifications of LGPL-covered components.

Recipients may modify/rebuild libusb and replace it in their own copy of the app.
Quit the GUI and stop the engine first. Work on a copy of the app, preserve the
same architecture and compatible libusb ABI, and replace
`Contents/Frameworks/libusb-1.0.0.dylib`. If needed, set its install name with
`install_name_tool -id @rpath/libusb-1.0.0.dylib <replacement>` before signing.
Changing a dylib invalidates the original signature; ad-hoc-sign the replacement
with `codesign --force --sign - <replacement>`, then re-sign the copied app with
`codesign --force --sign - <app-copy>`. These commands modify only your copy;
the original developer identity/notarization no longer applies. The application
must permit replacement (no enforced library validation for this ad-hoc release).
Do not claim this replacement workflow tested until the release acceptance check
actually replaces a compatible rebuilt library and verifies loading; synthetic
library loading is not equivalent to a rebuilt libusb acceptance check.
'''


def collect(manifest, inputs, provenance, output, cache, homebrew=None):
    created = []
    try:
        return _collect(manifest, inputs, provenance, output, cache, homebrew, created)
    except Exception:
        # Only remove an output directory this invocation successfully created.
        # Pre-existing results fail before ownership is recorded and stay intact.
        if created:
            shutil.rmtree(output)
        raise


def _collect(manifest, inputs, provenance, output, cache, homebrew, created):
    if manifest.get('schema_version') != 1:
        raise ValueError('unsupported native pin schema')
    if output.exists():
        raise ValueError('output must not exist (refuse stale inventory)')
    components = manifest['components']
    if set(inputs) != {c['id'] for c in components}:
        raise ValueError('input set must exactly match pinned components')
    # Check actual inputs and original packaging provenance before creating output.
    for c in components:
        relative(c['id'])
        relative(c['version'])
        checked(Path(inputs[c['id']]).read_bytes(), c['input_sha256'], c['id'])
        recorded = provenance['native_inputs']['Contents/Frameworks/' + c['bundle_name']]
        if recorded['sha256'] != c['input_sha256']:
            raise ValueError('packaging provenance disagrees: ' + c['id'])
    output.mkdir(parents=True)
    created.append(output)
    report = {'schema_version': 1, 'status': 'collected-not-release-approval',
              'architecture': manifest['architecture'], 'components': [],
              'remaining_checks': ['SDK embedded Rust and standard-library notices',
                                   'Modified-compatible libusb replacement acceptance test']}
    for c in components:
        source = fetch(c['source'], cache)
        target = output / 'sources' / source.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        record = dict(c)
        record['files'] = {}
        for name in c['notice_members']:
            relative(name)
            data = member_bytes(source, c['source']['root'] + '/' + name)
            dest = output / 'notices' / c['id'] / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
            record['files'][dest.relative_to(output).as_posix()] = digest(data)
        if 'upstream_binary' in c:
            binary_archive = fetch(c['upstream_binary'], cache)
            data = member_bytes(binary_archive, c['upstream_binary']['member'])
            checked(data, c['input_sha256'], 'upstream release member')
            if c.get('rustc_revision') and ('/rustc/' + c['rustc_revision'] + '/').encode() not in data:
                raise ValueError('upstream binary toolchain revision not corroborated')
            record['upstream_binary_match'] = True
            observed = sorted(set(x.decode() for x in re.findall(
                rb'registry/src/[^/\x00]+/([a-zA-Z0-9_-]+-[0-9][^/\x00 ]*)/', data)))
            import tomllib
            lock = tomllib.loads(member_bytes(source, c['source']['root'] + '/Cargo.lock').decode())
            pins = {p['name'] + '-' + p['version'] for p in lock['package']}
            if set(observed) - pins:
                raise ValueError('binary crate paths disagree with source Cargo.lock')
            record['observed_crate_paths'] = observed
            record['binary_graph_evidence'] = 'partial corroboration of upstream release association'
        if c['id'] in ('hidapi', 'libusb') and not homebrew:
            raise ValueError('Homebrew prefix required for pinned Homebrew inputs')
        if homebrew and c['id'] in ('hidapi', 'libusb'):
            cellar = homebrew / 'Cellar' / c['id'] / c['version']
            library = cellar / 'lib' / c.get('cellar_name', c['bundle_name'])
            checked(library.read_bytes(), c['input_sha256'], 'Homebrew library')
            for name in ('INSTALL_RECEIPT.json', 'sbom.spdx.json', '.brew/' + c['id'] + '.rb'):
                data = (cellar / name).read_bytes()
                checked(data, c['homebrew_evidence'][name], 'Homebrew evidence ' + name)
                if name.endswith('.rb'):
                    if ('sha256 "' + c['source']['sha256'] + '"').encode() not in data:
                        raise ValueError('Homebrew formula/source checksum mismatch')
                    if ('url "' + c['source']['url'] + '"').encode() not in data:
                        raise ValueError('Homebrew formula/source URL mismatch')
                dest = output / 'homebrew' / c['id'] / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(data)
                record['files'][dest.relative_to(output).as_posix()] = digest(data)
        report['components'].append(record)
    (output / 'REBUILDING.md').write_text(REBUILDING)
    (output / 'native-inputs.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (output / 'native-report.json').write_text(json.dumps(report, indent=2) + '\n')
    summary = {'schema_version': 1, 'status': 'complete', 'unresolved': [],
               'scope': 'native source and license collection only; SDK Rust graph/runtime and replacement acceptance are separate',
               'architecture': manifest['architecture'], 'components': report['components'],
               'native_inputs': {c['bundle_name']: c['input_sha256'] for c in components},
               'files': {p.relative_to(output).as_posix(): digest(p.read_bytes())
                         for p in sorted(output.rglob('*')) if p.is_file()}}
    (output / 'manifest.json').write_text(json.dumps(summary, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, default=Path(__file__).parent / 'licenses/native-inputs.json')
    parser.add_argument('--inputs', required=True, type=Path, help='JSON object: component id to actual dylib path')
    parser.add_argument('--provenance', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--cache', required=True, type=Path)
    parser.add_argument('--homebrew-prefix', type=Path)
    args = parser.parse_args()
    collect(json.loads(args.manifest.read_text()), json.loads(args.inputs.read_text()),
            json.loads(args.provenance.read_text()), args.output, args.cache, args.homebrew_prefix)


if __name__ == '__main__':
    main()
