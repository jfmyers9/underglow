#!/usr/bin/env python3
"""Collect standard-library notices from checksum-pinned Rust distributions.

Only notice files are extracted. Downloaded compilers are never run or installed.
An embedded rustc revision is corroborating evidence, not a reproducible-build proof.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

PINS = {
    '1.94.1': ('e408947bfd200af42db322daf0fadfe7e26d3bd1',
               '3ae26df6f8c83a13e4190469d124de49970432b6f4517a0762d9bfffdff5b89b'),
    '1.91.1': ('ed61e7d7e242494fb7057f2657300d9e77bb4fcb',
               '327c9017195b4ad0465ab6b1f1036377415cc6dc3612301af6ed9e3414ff53c8'),
}


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def verify_revision(binary, revision):
    revisions = set(re.findall(rb'/rustc/([a-f0-9]{40})', binary.read_bytes()))
    if revisions != {revision.encode()}:
        raise ValueError('missing or unexpected embedded rustc revision: ' + str(binary))


def extract_notices(archive, output, version):
    prefix = f'rustc-{version}-aarch64-apple-darwin/rustc/share/doc/rust/'
    copied = []
    with tarfile.open(archive, 'r:xz') as source:
        for member in source:
            if not member.name.startswith(prefix):
                continue
            relative = member.name[len(prefix):]
            if relative != 'COPYRIGHT-library.html' and not relative.startswith('licenses/'):
                continue
            if member.isdir():
                continue
            path = Path(relative)
            if (not member.isfile() or path.is_absolute() or '..' in path.parts
                    or path.as_posix() != relative
                    or member.size > 2 * 1024 * 1024
                    or relative.casefold() in {name.casefold() for name in copied}):
                raise ValueError('unsafe or duplicate runtime notice')
            destination = output / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.extractfile(member).read())
            copied.append(relative)
    if 'COPYRIGHT-library.html' not in copied or 'licenses/MIT.txt' not in copied:
        raise ValueError('runtime notice archive is incomplete')


def collect(binaries, analog, output, cache):
    if len({p.name for p in [*binaries, analog]}) != len(binaries) + 1:
        raise ValueError('input binary basenames must be distinct')
    for binary in binaries:
        verify_revision(binary, PINS['1.94.1'][0])
    verify_revision(analog, PINS['1.91.1'][0])
    output.mkdir(parents=True, exist_ok=False)
    try:
        cache.mkdir(parents=True, exist_ok=True)
        components = []
        for version, (revision, checksum) in PINS.items():
            name = f'rustc-{version}-aarch64-apple-darwin.tar.xz'
            url = 'https://static.rust-lang.org/dist/' + name
            archive = cache / name
            downloaded = not archive.exists()
            if downloaded:
                try:
                    subprocess.run(['curl', '-q', '--fail', '--silent', '--show-error',
                                    '--location', '--proto', '=https', '--proto-redir', '=https',
                                    '--max-time', '120', '--max-filesize', '140000000',
                                    '--output', str(archive), url], check=True, timeout=125)
                except Exception:
                    archive.unlink(missing_ok=True)
                    raise
            if digest(archive) != checksum:
                if downloaded:
                    archive.unlink(missing_ok=True)
                raise ValueError('runtime archive checksum mismatch: ' + name)
            extract_notices(archive, output / version, version)
            components.append({'version': version, 'rustc_revision': revision,
                               'archive_url': url, 'archive_sha256': checksum,
                               'license_selection': 'MIT with preserved component exceptions'})
        files = {str(p.relative_to(output)): digest(p)
                 for p in sorted(output.rglob('*')) if p.is_file()}
        manifest = {'schema_version': 1, 'status': 'complete', 'unresolved': [],
                    'scope': 'Rust standard-library notices; includes conservative exception license texts',
                    'components': components, 'files': files,
                    'input_binaries': {p.name: digest(p) for p in [*binaries, analog]}}
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    except Exception:
        shutil.rmtree(output)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', action='append', required=True, type=Path)
    parser.add_argument('--analog-sdk', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--cache', required=True, type=Path)
    args = parser.parse_args()
    collect(args.binary, args.analog_sdk, args.output, args.cache)


if __name__ == '__main__':
    main()
