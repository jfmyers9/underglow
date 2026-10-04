#!/usr/bin/env python3
"""Collect auditable Rust notices from checksum-verified registry archives.

Generate metadata with --locked --features gui --filter-platform
 aarch64-apple-darwin. The graph is conservative (includes build/optional
packages); it is not a claim that every listed package is linked. New versions
require a policy review. Exit 1 and manifest.status=incomplete preserve evidence
of gaps, never certify a partial notice tree. Python 3.11+; no Cargo builds.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tempfile
import tomllib
from urllib.parse import urlsplit

DEFAULT_OVERRIDES = Path(__file__).parent / 'licenses/rust-overrides.json'
MAX_ARCHIVE = 64 * 1024 * 1024
MAX_EXPANDED = 256 * 1024 * 1024
MAX_FILE = 32 * 1024 * 1024
REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'
NOTICE = re.compile(r'(?i)(licenses?|licences?|copying|copyrights?|notices?|unlicense)(?:$|[-_.])')


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def safe_path(value):
    path = PurePosixPath(value)
    if (not value or path.is_absolute() or '..' in path.parts or '\\' in value
            or any(ord(c) < 32 for c in value) or value != path.as_posix()):
        raise ValueError(f'unsafe path: {value!r}')
    return path


def checked_hash(value):
    if not isinstance(value, str) or not re.fullmatch('[0-9a-f]{64}', value):
        raise ValueError('invalid SHA256')
    return value


def validate_upstream(spec, commit):
    checked_hash(spec['sha256'])
    safe_path(spec['path'])
    parsed = urlsplit(spec['url'])
    parts = parsed.path.split('/')
    if (parsed.scheme != 'https' or parsed.netloc != 'raw.githubusercontent.com'
            or parsed.query or parsed.fragment or len(parts) < 5
            or not re.fullmatch('[0-9a-f]{40}', commit or '')
            or parts[3] != commit or any(ord(c) <= 32 for c in spec['url'])):
        raise ValueError('upstream URL must be a full-commit-pinned GitHub raw HTTPS URL')
    safe_path('/'.join(parts[1:]))


def fetch(spec, commit):
    validate_upstream(spec, commit)
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'notice'
        subprocess.run(['curl', '-q', '--fail', '--silent', '--show-error',
                        '--proto', '=https', '--connect-timeout', '15',
                        '--max-time', '60', '--max-filesize', str(MAX_FILE),
                        '--output', str(path), spec['url']], check=True, timeout=65)
        data = path.read_bytes()
    if len(data) > MAX_FILE or sha256(data) != spec['sha256']:
        raise ValueError('upstream notice SHA256/size mismatch')
    return data


def archive_files(archive, checksum, prefix):
    if archive.stat().st_size > MAX_ARCHIVE:
        raise ValueError('registry archive too large')
    raw = archive.read_bytes()
    if sha256(raw) != checked_hash(checksum):
        raise ValueError('registry archive checksum mismatch')
    result = {}
    total = 0
    # Streaming mode bounds decompression while traversing members; never extract
    # symlinks or let tarfile choose paths on the filesystem.
    with tarfile.open(fileobj=io.BytesIO(raw), mode='r|gz') as source:
        for index, member in enumerate(source):
            if index > 30000:
                raise ValueError('too many archive members')
            name = safe_path(member.name)
            if name.parts[0] != prefix:
                raise ValueError('unexpected archive root')
            if member.isdir():
                continue
            if not member.isfile() or len(name.parts) < 2:
                raise ValueError('non-regular archive member')
            relative = PurePosixPath(*name.parts[1:]).as_posix()
            total += member.size
            if member.size > MAX_FILE or total > MAX_EXPANDED or relative in result:
                raise ValueError('archive size/duplicate member limit')
            result[relative] = source.extractfile(member).read()
    return result


def selected_packages(metadata, root_package=None):
    packages = {p['id']: p for p in metadata['packages']}
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    if root_package:
        roots = [p['id'] for p in packages.values() if p['name'] == root_package]
        if len(roots) != 1:
            raise ValueError('root package must match exactly one metadata package')
    else:
        roots = metadata['workspace_members']
    seen, todo = set(), list(roots)
    while todo:
        ident = todo.pop()
        if ident in seen:
            continue
        if ident not in packages or ident not in nodes:
            raise ValueError('incomplete metadata resolve graph')
        seen.add(ident)
        todo.extend(nodes[ident]['dependencies'])
    return sorted((packages[i] for i in seen), key=lambda p: (p['name'], p['version']))


def retained(path, data):
    """Keep full notice folders, READMEs, font metadata, and copyright-bearing code."""
    parts = PurePosixPath(path).parts
    if any(NOTICE.match(part) for part in parts):
        # copying.rs is code, not a license; retained only as supplemental evidence.
        return True
    if parts[-1].lower().startswith(('readme', 'authors')):
        return True
    if parts[0] == 'fonts':
        return True  # Includes original embedded copyright/name tables.
    if len(data) < MAX_FILE and b'\0' not in data:
        return b'copyright' in data.lower()
    return False


def collect(metadata, output, policy, lockfile, root_package=None, downloader=fetch):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    lock_raw = Path(lockfile).read_bytes()
    lock = {(p['name'], p['version'], p.get('source')): p
            for p in tomllib.loads(lock_raw.decode())['package']}
    report = {'schema_version': 1, 'status': 'incomplete',
              'scope': 'Conservative reachable Cargo graph including build/dev/optional dependencies; not a linked-code inventory.',
              'root_package': root_package, 'lock_sha256': sha256(lock_raw),
              'metadata_sha256': sha256(json.dumps(metadata, sort_keys=True).encode()),
              'policy_sha256': sha256(json.dumps(policy, sort_keys=True).encode()),
              'packages': [], 'unresolved': [], 'excluded_local_packages': []}
    for package in selected_packages(metadata, root_package):
        name, version = package['name'], package['version']
        key = name + '@' + version
        if package['source'] is None:
            report['excluded_local_packages'].append(key)
            continue
        entry = {'name': name, 'version': version, 'source': package['source'],
                 'declared_license': package['license'], 'files': []}
        report['packages'].append(entry)
        try:
            if package['source'] != REGISTRY:
                raise ValueError('non-crates.io dependency requires separate source review')
            if not re.fullmatch(r'[A-Za-z0-9_-]+', name) or not re.fullmatch(r'[A-Za-z0-9.+_-]+', version):
                raise ValueError('invalid crate name/version')
            rule = policy['packages'].get(key)
            if not rule:
                raise ValueError('unreviewed crate/version; add checksum-pinned policy record')
            locked = lock[(name, version, package['source'])]
            checksum = locked['checksum']
            if checksum != rule['crate_sha256'] or package['license'] != rule['declared_license']:
                raise ValueError('policy differs from Cargo.lock/metadata')
            entry.update(crate_sha256=checksum, selected_license=rule['selected_license'],
                         policy_kind=rule['policy_kind'], review_note=rule.get('review_note', ''))
            root = Path(package['manifest_path']).parent
            archive = root.parents[2] / 'cache' / root.parent.name / (root.name + '.crate')
            files = archive_files(archive, checksum, name + '-' + version)
            declared_package = tomllib.loads(files['Cargo.toml'].decode())['package']
            if (declared_package.get('name') != name or declared_package.get('version') != version
                    or declared_package.get('license') != rule['declared_license']):
                raise ValueError('registry archive package declaration differs from policy')
            vcs = json.loads(files.get('.cargo_vcs_info.json', b'{}'))
            entry['vcs'] = vcs
            upstream = rule.get('upstream_files', [])
            standards = rule.get('standard_files', [])
            if upstream:
                actual_commit = vcs.get('git', {}).get('sha1')
                if actual_commit and actual_commit != rule['upstream_commit']:
                    raise ValueError('upstream commit differs from registry provenance')
            kept = {path: data for path, data in files.items() if retained(path, data)}
            for path in ['Cargo.toml', 'Cargo.toml.orig', '.cargo_vcs_info.json']:
                if path in files:
                    kept[path] = files[path]
            for spec in upstream + standards:
                commit = spec.get('commit', rule.get('upstream_commit'))
                validate_upstream(spec, commit)
                data = downloader(spec, commit)
                if len(data) > MAX_FILE or sha256(data) != spec['sha256']:
                    raise ValueError('upstream notice SHA256/size mismatch')
                if spec['path'] in kept:
                    raise ValueError('duplicate upstream notice destination')
                kept[spec['path']] = data
            if rule['policy_kind'] == 'metadata-declaration-with-source':
                if not standards:
                    raise ValueError('metadata-only policy requires canonical terms')
                # Preserve exact source/declaration, not a fabricated copyright statement.
                declared = tomllib.loads(files['Cargo.toml'].decode())['package']['license']
                if declared != rule['declared_license']:
                    raise ValueError('archive license declaration differs from policy')
                kept['original-source.crate'] = archive.read_bytes()
            elif rule['policy_kind'] != 'verbatim-license-text':
                raise ValueError('unknown license evidence policy')
            for path, digest in rule.get('required_files', {}).items():
                safe_path(path)
                if path not in kept or sha256(kept[path]) != checked_hash(digest):
                    raise ValueError(f'required reviewed notice missing/changed: {path}')
            directory = output / 'rust' / (name + '-' + version)
            for path, data in sorted(kept.items()):
                target = directory.joinpath(*safe_path(path).parts)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(data)
                spec = next((s for s in upstream + standards if s['path'] == path), None)
                entry['files'].append({'path': str(target.relative_to(output)),
                                       'sha256': sha256(data),
                                       'origin': spec['url'] if spec else 'verified registry archive'})
            if rule.get('unresolved'):
                raise ValueError(rule['unresolved'])
            if not rule.get('required_files'):
                raise ValueError('no reviewed full license text; filename/README alone is not evidence')
            entry['status'] = 'collected'
        except (ValueError, KeyError, OSError, tarfile.TarError, subprocess.SubprocessError) as error:
            entry['status'] = 'unresolved'
            entry['error'] = str(error)
            report['unresolved'].append({'package': key, 'reason': str(error)})
    report['status'] = 'incomplete' if report['unresolved'] else 'complete'
    (output / 'NOTICES.md').write_text(
        '# Rust dependency notices\n\nStatus: ' + report['status'] + '\n\n' + report['scope'] +
        '\n\nOriginal notices and copyright-bearing source files are retained verbatim. '
        'OR choices and all AND obligations are recorded in manifest.json. '
        'Metadata-declaration-with-source entries preserve the complete registry archive and '
        'original license declaration; standard-terms/ contains separately labeled canonical '
        'SPDX terms, not an invented upstream copyright notice. '
        'Font files retain original embedded attribution metadata. Local workspace packages, '
        'Rust toolchain/runtime, SDKs, and separately linked native libraries need separate notices. '
        'Complete means this pinned collection policy passed; it is not legal approval or authorization to publish.\n\n' +
        '\n'.join('- ' + gap['package'] + ': ' + gap['reason'] for gap in report['unresolved']) + '\n')
    report['files'] = {str(p.relative_to(output)): sha256(p.read_bytes())
                       for p in sorted(output.rglob('*')) if p.is_file()}
    (output / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--metadata', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--overrides', type=Path, default=DEFAULT_OVERRIDES)
    parser.add_argument('--lockfile', type=Path)
    parser.add_argument('--root-package')
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text())
    policy = json.loads(args.overrides.read_text())
    if policy.get('schema_version') != 1:
        parser.error('unsupported override schema')
    report = collect(metadata, args.output, policy,
                     args.lockfile or Path(metadata['workspace_root']) / 'Cargo.lock', args.root_package)
    print(f"{len(report['packages'])} registry crates; {len(report['unresolved'])} unresolved; {args.output}/manifest.json")
    return 1 if report['unresolved'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
