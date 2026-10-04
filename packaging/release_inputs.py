#!/usr/bin/env python3
"""Fetch a maintainer-reviewed, checksum-pinned input tree (never execute it).

Archive root: rgb-sdk.dylib, analog-sdk.dylib, notices/NOTICES.md,
notices/MPL-2.0.txt, notices/native-licenses.json, other notice files, and
optional deps/. Keep this tree outside the packaging output directory.
Checksum verification establishes identity, not redistribution permission.
"""
import argparse
import gzip
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
from urllib.parse import urlsplit

MAX_DOWNLOAD = 256 * 1024 * 1024
MAX_EXPANDED = 512 * 1024 * 1024
MAX_FILE = 128 * 1024 * 1024
MAX_MEMBERS = 4096
TIMEOUT = 120
REQUIRED = ('rgb-sdk.dylib', 'analog-sdk.dylib', 'notices/NOTICES.md',
            'notices/MPL-2.0.txt', 'notices/native-licenses.json')


def validate_source(url, sha256):
    parsed = urlsplit(url)
    if (parsed.scheme != 'https' or not parsed.hostname or parsed.username is not None
            or parsed.password is not None or parsed.fragment
            or any(ord(c) < 33 for c in url)):
        raise ValueError('input URL must be HTTPS without credentials, fragments, or whitespace')
    if not re.fullmatch(r'[0-9a-fA-F]{64}', sha256):
        raise ValueError('SHA256 must be exactly 64 hexadecimal characters')


def download(url, sha256, destination):
    validate_source(url, sha256)
    # -q disables user curl configuration; redirects may never downgrade HTTPS.
    # The subprocess deadline also bounds DNS, TLS, and slow/stalled providers.
    subprocess.run([
        'curl', '-q', '--fail', '--silent', '--show-error', '--location',
        '--max-redirs', '5', '--proto', '=https', '--proto-redir', '=https',
        '--connect-timeout', '15', '--max-time', str(TIMEOUT),
        '--max-filesize', str(MAX_DOWNLOAD), '--output', str(destination), url,
    ], check=True, timeout=TIMEOUT + 5)
    if destination.stat().st_size > MAX_DOWNLOAD:
        raise ValueError('download exceeds size limit')
    digest = hashlib.sha256()
    with destination.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    if digest.hexdigest() != sha256.lower():
        raise ValueError('input SHA256 mismatch')


def extract(archive, destination):
    """Extract into a NEW directory; remove partial output on any failure."""
    destination = Path(destination)
    destination.mkdir(parents=False, exist_ok=False)
    try:
        if Path(archive).stat().st_size > MAX_DOWNLOAD:
            raise ValueError('archive exceeds size limit')
        # Bound decompression BEFORE tarfile parses PAX/long-name metadata.
        with tempfile.TemporaryFile() as raw:
            with gzip.open(archive, 'rb') as compressed:
                total = 0
                while True:
                    block = compressed.read(min(1024 * 1024, MAX_EXPANDED - total + 1))
                    if not block:
                        break
                    total += len(block)
                    if total > MAX_EXPANDED:
                        raise ValueError('expanded archive exceeds size limit')
                    raw.write(block)
            raw.seek(0)
            seen = set()
            with tarfile.open(fileobj=raw, mode='r:') as bundle:
                for count, member in enumerate(bundle, 1):
                    if count > MAX_MEMBERS:
                        raise ValueError('archive has too many entries')
                    name = member.name.rstrip('/') if member.isdir() else member.name
                    parts = name.split('/')
                    if (not name or len(name) > 1024 or len(parts) > 32
                            or any(p in ('', '.', '..') for p in parts)
                            or '\\' in name or any(ord(c) < 32 for c in name)
                            or not name.isascii()):
                        raise ValueError('unsafe archive path')
                    # Case-folding also protects the default macOS filesystem.
                    key = name.casefold()
                    if key in seen:
                        raise ValueError('duplicate archive path')
                    seen.add(key)
                    if member.type not in (tarfile.DIRTYPE, tarfile.REGTYPE, tarfile.AREGTYPE) or member.issparse():
                        raise ValueError('only regular files and directories are allowed')
                    if member.size < 0 or member.size > MAX_FILE:
                        raise ValueError('archive member exceeds size limit')
                    target = destination.joinpath(*parts)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    if member.isdir():
                        target.mkdir(exist_ok=True)
                    else:
                        with bundle.extractfile(member) as source, target.open('xb') as output:
                            shutil.copyfileobj(source, output, 1024 * 1024)
                        target.chmod(0o644)
        for name in REQUIRED:
            path = destination / name
            if not path.is_file() or path.stat().st_size == 0:
                raise ValueError('missing or empty required input: ' + name)
    except BaseException:
        shutil.rmtree(destination)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    validate_source(args.url, args.sha256)
    if args.output.exists() or args.output.is_symlink():
        parser.error('--output must be a new directory')
    with tempfile.TemporaryDirectory(prefix='wooting-input-download-') as temporary:
        archive = Path(temporary) / 'inputs.tar.gz'
        download(args.url, args.sha256, archive)
        extract(archive, args.output.absolute())


if __name__ == '__main__':
    main()
