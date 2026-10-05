#!/usr/bin/env python3
"""Assemble collected notices and preserve unresolved review findings.

A complete collector is not release approval. Findings remain in audit.json;
macOS --verified-inputs refuses an incomplete audit.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_collection(directory):
    if directory.is_symlink():
        raise ValueError('collection directory must not be a symlink')
    manifest = json.loads((directory / 'manifest.json').read_text())
    if (manifest.get('schema_version') != 1 or manifest.get('status') != 'complete'
            or manifest.get('unresolved') != []):
        raise ValueError('incomplete collection: ' + str(directory))
    paths = list(directory.rglob('*'))
    if any(p.is_symlink() or (not p.is_file() and not p.is_dir()) for p in paths):
        raise ValueError('collection contains a symlink or special file')
    files = {p.relative_to(directory).as_posix(): digest(p)
             for p in paths if p.is_file() and p != directory / 'manifest.json'}
    if manifest.get('files') != files:
        raise ValueError('collection file hashes differ: ' + str(directory))
    return manifest


def assemble(collections, binaries, findings, output):
    reports = {name: verify_collection(path) for name, path in collections.items()}
    if reports['application-rust']['lock_sha256'] != digest(ROOT / 'Cargo.lock'):
        raise ValueError('application Cargo.lock changed since collection')
    sdk_lock = collections['native'] / 'notices/analog/Cargo.lock'
    if reports['analog-rust']['lock_sha256'] != digest(sdk_lock):
        raise ValueError('SDK lockfile differs from corresponding source archive')
    if reports['native']['architecture'] != 'arm64':
        raise ValueError('this audit currently covers arm64 only')
    policy = json.loads((ROOT / 'packaging/licenses/rust-overrides.json').read_text())
    policy_sha = hashlib.sha256(json.dumps(policy, sort_keys=True).encode()).hexdigest()
    if any(reports[k]['policy_sha256'] != policy_sha for k in ('application-rust', 'analog-rust')):
        raise ValueError('Rust collection policy changed; recollect notices')
    native = dict(reports['native']['native_inputs'])
    runtime = reports['runtime']['input_binaries']
    for name, path in binaries.items():
        value = digest(path)
        if runtime.get(name) != value:
            raise ValueError('binary changed since runtime collection: ' + name)
        native[name] = value
    analog = 'libwooting_analog_sdk_dist.dylib'
    if runtime.get(analog) != native.get(analog):
        raise ValueError('SDK runtime and native inputs differ')
    if (findings.get('schema_version') != 1 or not isinstance(findings.get('unresolved'), list)
            or not isinstance(findings.get('review_notes'), list)):
        raise ValueError('review findings require schema 1 and explicit lists')
    output.mkdir(parents=True, exist_ok=False)
    try:
        for name, directory in collections.items():
            shutil.copytree(directory, output / name)
        shutil.copy2(ROOT / 'LICENSE', output / 'APPLICATION-LICENSE.txt')
        shutil.copy2(ROOT / 'packaging/licenses/MPL-2.0.txt', output / 'MPL-2.0.txt')
        (output / 'review-findings.json').write_text(json.dumps(findings, indent=2) + '\n')
        status = 'incomplete' if findings['unresolved'] else 'reviewed'
        (output / 'NOTICES.md').write_text(
            '# Underglow — licenses and corresponding source\n\n'
            'Copyright (c) 2026 James Myers. Application license: APPLICATION-LICENSE.txt (MIT).\n'
            'This project is not an official Wooting product.\n\n'
            'Release review status: **' + status + '**. See review-findings.json. '
            'Notice collection is engineering evidence, not a legal opinion or permission to publish.\n\n'
            '- application-rust/: Cargo dependency notices, license choices and provenance.\n'
            '- analog-rust/: locked SDK Rust dependencies, including embedded HIDAPI C notices.\n'
            '- runtime/: Rust 1.94.1 and 1.91.1 standard-library copyright/exception notices.\n'
            '- native/sources/: complete corresponding source for both MPL SDKs, BSD hidapi, and LGPL libusb.\n'
            '- native/REBUILDING.md: original build recipes, modification/replacement rights and instructions.\n\n'
            'libusb 1.0.30 is used as a separate shared library under LGPL 2.1 or later. '
            'Its full license, attribution, and source accompany this app. Users may modify or '
            'replace compatible libraries in their own copy and reverse engineer for debugging '
            'those modifications. No additional restriction is imposed by this application.\n\n'
            'Some crates declare SPDX terms without complete notice files. Their original source '
            'packages, declarations, and separately labeled canonical terms are preserved; no '
            'copyright holders or dates were invented. See each manifest policy_kind/review_note.\n')
        mapping = {name: 'NOTICES.md' for name in native}
        (output / 'native-licenses.json').write_text(json.dumps(mapping, indent=2) + '\n')
        audit = {'schema_version': 1, 'status': status, 'architecture': 'arm64',
                 'unresolved': findings['unresolved'], 'review_notes': findings['review_notes'],
                 'native_inputs': native,
                 'scope': 'Hash-bound engineering review; not legal clearance or publication approval',
                 'files': {p.relative_to(output).as_posix(): digest(p)
                           for p in sorted(output.rglob('*')) if p.is_file()}}
        (output / 'audit.json').write_text(json.dumps(audit, indent=2) + '\n')
        return audit
    except Exception:
        shutil.rmtree(output)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('application-rust', 'analog-rust', 'runtime', 'native', 'binary',
                 'gui', 'service', 'output'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--findings', type=Path,
                        default=ROOT / 'packaging/licenses/review-findings.json')
    args = parser.parse_args()
    collections = {name: getattr(args, name.replace('-', '_'))
                   for name in ('application-rust', 'analog-rust', 'runtime', 'native')}
    binaries = {'underglow': args.binary, 'underglow-gui': args.gui, 'underglow-service': args.service}
    audit = assemble(collections, binaries, json.loads(args.findings.read_text()), args.output)
    print(f"Notice bundle created; release review: {audit['status']}")


if __name__ == '__main__':
    main()
