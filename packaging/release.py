#!/usr/bin/env python3
"""Stage trusted native build inputs and their non-system shared-library closure."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
# libc / loader stay supplied by the distribution; never bundle glibc.
LINUX_SYSTEM = re.compile(r'^(linux-vdso|ld-linux|lib(c|m|pthread|dl|rt|resolv|util|anl)\.so)')


def run(*args):
    return subprocess.check_output([str(a) for a in args], text=True).strip()


def mac_dependencies(path):
    lines = run('otool', '-L', path).splitlines()[1:]
    return [line.strip().split(' (compatibility')[0] for line in lines]


def mac_rpaths(path):
    lines = run('otool', '-l', path).splitlines()
    return [lines[i + 2].strip().split('path ', 1)[1].split(' (offset')[0]
            for i, line in enumerate(lines) if line.strip() == 'cmd LC_RPATH']


def resolve_mac(name, source, executable):
    def expand(value):
        return value.replace('@loader_path', str(source.parent)).replace('@executable_path', str(executable.parent))
    if name.startswith('@rpath/'):
        for rpath in mac_rpaths(source) + mac_rpaths(executable):
            candidate = Path(expand(rpath)) / name[len('@rpath/'):]
            if candidate.is_file():
                return candidate
        raise RuntimeError('unresolved Mach-O rpath: ' + name)
    return Path(expand(name))


def bundle_dependencies(stage, system):
    lib = stage / 'lib/wooting-signals'
    # Retain original paths for resolving @loader_path and transitive imports.
    origins = {}
    bundled = {}
    queue = []

    def add(source, dest):
        source = source.resolve()
        if dest.name in bundled and bundled[dest.name] != source:
            raise RuntimeError('conflicting library basename: ' + dest.name)
        bundled[dest.name] = source
        if dest in origins:
            return
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
        dest.chmod(dest.stat().st_mode | 0o200)
        origins[dest] = source
        queue.append(dest)

    def process(inputs):
        for source, dest in inputs:
            add(source, dest)
        executable = inputs[0][0].resolve()
        while queue:
            dest = queue.pop(0)
            source = origins[dest]
            if system == 'Darwin':
                dependencies = mac_dependencies(source)
                for dependency in dependencies:
                    if dependency.startswith(('/usr/lib/', '/System/Library/')):
                        continue
                    # LC_ID_DYLIB is reported by otool too; it is not a dependency.
                    own_id = run('otool', '-D', source).splitlines()[1:]
                    if dependency in own_id:
                        continue
                    resolved = resolve_mac(dependency, source, executable)
                    if not resolved.is_file():
                        raise RuntimeError('missing dependency: ' + str(resolved))
                    target = lib / resolved.name
                    add(resolved, target)
                    relative = '@loader_path/' + ('../lib/wooting-signals/' if dest.parent.name == 'bin' else '') + target.name
                    run('install_name_tool', '-change', dependency, relative, dest)
                # All non-system imports are now loader-relative; remove stale
                # search paths so installed binaries do not retain checkout refs.
                for rpath in mac_rpaths(source):
                    run('install_name_tool', '-delete_rpath', rpath, dest)
                if dest.parent == lib:
                    run('install_name_tool', '-id', '@loader_path/' + dest.name, dest)
            else:
                # ldd only on trusted local build inputs; never use on arbitrary downloads.
                for line in run('ldd', source).splitlines():
                    if 'not found' in line:
                        raise RuntimeError('unresolved ELF dependency: ' + line.strip())
                    match = re.match(r'\s*(\S+) => (/.+?)\s+\(0x[0-9a-fA-F]+\)', line)
                    if match and not LINUX_SYSTEM.match(match[1]):
                        name = Path(match[1]).name
                        add(Path(match[2]), lib / name)
                        if match[1].startswith('/'):
                            run('patchelf', '--replace-needed', match[1], name, dest)
                    elif not match:
                        absolute = re.match(r'\s*(/.+?)\s+\(0x[0-9a-fA-F]+\)', line)
                        if absolute and not LINUX_SYSTEM.match(Path(absolute[1]).name):
                            name = Path(absolute[1]).name
                            add(Path(absolute[1]), lib / name)
                            run('patchelf', '--replace-needed', absolute[1], name, dest)
                rpath = '$ORIGIN/../lib/wooting-signals' if dest.parent.name == 'bin' else '$ORIGIN'
                run('patchelf', '--set-rpath', rpath, dest)
        if system == 'Darwin':
            # Rewriting Mach-O invalidates signatures; ad-hoc signing is NOT notarization.
            for dest in origins:
                run('codesign', '--force', '--sign', '-', dest)
        return {str(dest.relative_to(stage)): str(source) for dest, source in origins.items()}
    return process


def validate_notices(directory, native_files):
    mapping = json.loads((directory / 'native-licenses.json').read_text())
    for name in native_files:
        notice = mapping.get(Path(name).name)
        if not isinstance(notice, str):
            raise RuntimeError('missing native license mapping: ' + Path(name).name)
        path = directory / notice
        if not path.resolve().is_relative_to(directory.resolve()) or not path.is_file() or not path.stat().st_size:
            raise RuntimeError('invalid license/notice file: ' + notice)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/wooting-signals')
    parser.add_argument('--gui', type=Path, help='optional native GUI binary')
    parser.add_argument('--rgb-sdk', type=Path, required=True)
    parser.add_argument('--analog-sdk', type=Path, required=True)
    parser.add_argument('--notices', type=Path, required=True,
                        help='directory with NOTICES.md and license texts for all shipped inputs/dependencies')
    parser.add_argument('--output', type=Path, required=True, help='new .tar.gz artifact path')
    args = parser.parse_args()
    system = platform.system()
    if system not in ('Darwin', 'Linux'):
        parser.error('build on macOS or Linux; cross-packaging is not supported')
    if args.output.exists():
        parser.error('output already exists; refusing overwrite')
    for path in (args.binary, args.rgb_sdk, args.analog_sdk, args.notices / 'NOTICES.md',
                 args.notices / 'MPL-2.0.txt', args.notices / 'native-licenses.json'):
        if not path.is_file():
            parser.error('missing required input: ' + str(path))
    if args.gui and not args.gui.is_file():
        parser.error('GUI binary missing')
    suffix = 'dylib' if system == 'Darwin' else 'so'
    with tempfile.TemporaryDirectory(prefix='wooting-release-') as temporary:
        stage = Path(temporary) / 'wooting-signals'
        (stage / 'bin').mkdir(parents=True)
        inputs = [(args.binary, stage / 'bin/wooting-signals'),
                  (args.rgb_sdk, stage / 'lib/wooting-signals' / ('libwooting-rgb-sdk.' + suffix)),
                  (args.analog_sdk, stage / 'lib/wooting-signals' / ('libwooting_analog_sdk_dist.' + suffix))]
        if args.gui:
            inputs.append((args.gui, stage / 'bin/wooting-gui'))
        origins = bundle_dependencies(stage, system)(inputs)
        validate_notices(args.notices, origins)
        share = stage / 'share/wooting-signals'
        shutil.copytree(args.notices, share)
        shutil.copytree(ROOT / 'examples', share / 'examples')
        shutil.copy2(ROOT / 'packaging/wooting-service', stage / 'bin/wooting-service')
        shutil.copy2(ROOT / 'packaging/install.py', stage / 'install.py')
        shutil.copy2(ROOT / 'docs/install.md', stage / 'INSTALL.md')
        manifest = {'format': 1, 'system': system, 'architecture': platform.machine(),
                    'source_revision': run('git', '-C', ROOT, 'rev-parse', 'HEAD'),
                    'rgb_source_revision': run('git', '-C', ROOT / 'external/wooting-rgb-sdk', 'rev-parse', 'HEAD'),
                    'analog_sdk_version': '0.9.1 (operator must verify supplied artifact)',
                    'files': {str(p.relative_to(stage)): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in sorted(stage.rglob('*')) if p.is_file()},
                    'native_dependencies': sorted(origins)}
        (stage / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(args.output, 'w:gz') as archive:
            archive.add(stage, arcname='wooting-signals')
    digest = hashlib.sha256(args.output.read_bytes()).hexdigest()
    args.output.with_name(args.output.name + '.sha256').write_text(digest + '  ' + args.output.name + '\n')
    print(args.output)


if __name__ == '__main__':
    main()
