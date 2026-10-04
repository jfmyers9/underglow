#!/usr/bin/env python3
"""Build a self-contained macOS app and disk image from trusted, reviewed inputs.

No downloads, mounts, installs, service activation or publishing. By default the
result is ad-hoc signed for LOCAL TESTING ONLY, not a Gatekeeper-ready release.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import struct
import subprocess
import tempfile
import zlib

import release

APP = 'Wooting Signals.app'
MACH_MAGICS = (b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf',
               b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca',
               b'\xca\xfe\xba\xbf', b'\xbf\xba\xfe\xca')


def run(*args):
    # Pin platform tools rather than trusting user PATH entries.
    tool = str(args[0])
    return release.run('/usr/bin/' + tool, *args[1:])


def digest(path):
    with path.open('rb') as stream:
        value = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
        return value.hexdigest()


def create_icon(path):
    """Dependency-free ICNS: dark rounded tile, mint W and keyboard signal bars."""
    def chunk(kind, payload):
        data = kind + payload
        return struct.pack('>I', len(payload)) + data + struct.pack('>I', zlib.crc32(data))

    images = []
    for size, kind in ((128, b'ic07'), (256, b'ic08'), (512, b'ic09')):
        rows = bytearray()
        segments = ((.23, .26, .34, .64), (.34, .64, .50, .40),
                    (.50, .40, .66, .64), (.66, .64, .77, .26))
        for y in range(size):
            rows.append(0)  # PNG no-filter row
            py = (y + .5) / size
            for x in range(size):
                px = (x + .5) / size
                dx, dy = max(.19 - px, 0, px - .81), max(.19 - py, 0, py - .81)
                if dx * dx + dy * dy > .14 ** 2:
                    rows.extend((0, 0, 0, 0))
                    continue
                color = (21, 29, 38, 255)
                for ax, ay, bx, by in segments:
                    vx, vy = bx - ax, by - ay
                    t = max(0, min(1, ((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy)))
                    if (px - ax - t * vx) ** 2 + (py - ay - t * vy) ** 2 < .032 ** 2:
                        color = (118, 235, 198, 255)
                if .76 < py < .80 and .24 < px < .76 and int((px - .24) / .065) % 2 == 0:
                    color = (71, 141, 129, 255)
                rows.extend(color)
        png = (b'\x89PNG\r\n\x1a\n'
               + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
               + chunk(b'IDAT', zlib.compress(bytes(rows), 9)) + chunk(b'IEND', b''))
        images.append(kind + struct.pack('>I', len(png) + 8) + png)
    payload = b''.join(images)
    path.write_bytes(b'icns' + struct.pack('>I', len(payload) + 8) + payload)


def version_tuple(value):
    return tuple(int(v) for v in value.split('.')) + (0,) * (3 - len(value.split('.')))


def validate_native(path, architecture, minimum_os):
    with path.open('rb') as stream:
        if stream.read(4) not in MACH_MAGICS:
            raise RuntimeError('not a 64-bit Mach-O input: ' + str(path))
    architectures = set(run('lipo', '-archs', path).split())
    required = {'arm64', 'x86_64'} if architecture == 'universal2' else {architecture}
    if not required.issubset(architectures):
        raise RuntimeError('incompatible architecture: ' + str(path))
    commands = run('otool', '-l', path)
    platforms = re.findall(r'\bplatform\s+(\w+)', commands)
    if any(value not in ('1', 'MACOS', 'macos') for value in platforms):
        raise RuntimeError('input is not a macOS binary: ' + str(path))
    minimums = re.findall(r'\bminos\s+(\d+(?:\.\d+){0,2})', commands)
    minimums += re.findall(r'cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version (\d+(?:\.\d+){0,2})', commands)
    if not minimums:
        raise RuntimeError('missing macOS deployment target: ' + str(path))
    if any(version_tuple(v) > version_tuple(minimum_os) for v in minimums):
        raise RuntimeError('input deployment target exceeds --minimum-os: ' + str(path))


def validate_notice_tree(directory):
    # Reject ALL symlinks, including otherwise-unused embedded files/directories.
    for path in [directory, *directory.rglob('*')]:
        if path.is_symlink() or (not path.is_file() and not path.is_dir()):
            raise RuntimeError('notices must contain only regular files/directories: ' + str(path))
    for name in ('NOTICES.md', 'MPL-2.0.txt', 'native-licenses.json'):
        path = directory / name
        if not path.is_file() or not path.stat().st_size:
            raise RuntimeError('missing required notice: ' + name)


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    for flag in ('binary', 'gui', 'service', 'rgb-sdk', 'analog-sdk', 'notices', 'output'):
        p.add_argument('--' + flag, type=Path, required=True)
    p.add_argument('--version', required=True, help='numeric major.minor.patch')
    p.add_argument('--identifier', required=True, help='reverse-DNS application identifier')
    p.add_argument('--minimum-os', required=True, help='must cover every native input deployment target')
    p.add_argument('--architecture', choices=('arm64', 'x86_64', 'universal2'), required=True)
    review = p.add_mutually_exclusive_group(required=True)
    review.add_argument('--verified-inputs', action='store_true', help='confirm all inputs/dependencies are trusted and redistribution notices reviewed')
    review.add_argument('--local-test', action='store_true', help='trusted local inputs; license review pending; NOT FOR DISTRIBUTION')
    p.add_argument('--rgb-sdk-version', default='operator-supplied; see input SHA-256')
    p.add_argument('--analog-sdk-version', default='operator-supplied; see input SHA-256')
    p.add_argument('--icon', type=Path, help='optional .icns file')
    p.add_argument('--sign-identity', help='explicit Developer ID Application identity (not a secret)')
    p.add_argument('--notary-profile', help='existing notarytool keychain profile; sends artifacts to Apple')
    return p


def validate(args):
    if platform.system() != 'Darwin':
        raise RuntimeError('macOS packaging requires a macOS host')
    if not args.verified_inputs and not args.local_test:
        raise RuntimeError('--verified-inputs requires operator review of inputs and notices, or use --local-test')
    if args.local_test and (args.sign_identity or args.notary_profile):
        raise RuntimeError('--local-test forbids Developer ID signing and notarization')
    if args.output.exists() or args.output.is_symlink():
        raise RuntimeError('output already exists; refusing overwrite')
    if not re.fullmatch(r'\d+\.\d+\.\d+', args.version):
        raise RuntimeError('--version must be numeric major.minor.patch')
    if not re.fullmatch(r'[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+', args.identifier):
        raise RuntimeError('invalid --identifier')
    if not re.fullmatch(r'\d+\.\d+(?:\.\d+)?', args.minimum_os):
        raise RuntimeError('invalid --minimum-os')
    if args.sign_identity and not args.sign_identity.startswith('Developer ID Application: '):
        raise RuntimeError('--sign-identity must be a Developer ID Application identity')
    if args.notary_profile and not args.sign_identity:
        raise RuntimeError('notarization requires --sign-identity')
    if args.icon and (args.icon.suffix != '.icns' or not args.icon.is_file()):
        raise RuntimeError('--icon must be an existing .icns file')
    validate_notice_tree(args.notices)
    for path in (args.binary, args.gui, args.service, args.rgb_sdk, args.analog_sdk):
        if not path.is_file():
            raise RuntimeError('missing native input: ' + str(path))
        validate_native(path, args.architecture, args.minimum_os)


def sign(path, identity):
    options = ('--options', 'runtime', '--timestamp') if identity else ('--timestamp=none',)
    run('codesign', '--force', '--sign', identity or '-', *options, path)


def notarize(path, profile):
    result = json.loads(run('xcrun', 'notarytool', 'submit', path,
                            '--keychain-profile', profile, '--wait', '--output-format', 'json'))
    if result.get('status') != 'Accepted':
        raise RuntimeError('Apple notarization was not accepted: ' + str(result.get('status')))


def publish_directory(source, output):
    # Darwin's exclusive rename is atomic and cannot replace even an empty dir.
    libc = ctypes.CDLL(None, use_errno=True)
    rename = libc.renamex_np
    rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(os.fsencode(source), os.fsencode(output), 4):  # RENAME_EXCL
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), str(output))


def build(args):
    validate(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.wooting-macos-', dir=args.output.parent) as temporary:
        work = Path(temporary)
        product = work / 'product'
        product.mkdir()
        app = product / APP
        macos = app / 'Contents/MacOS'
        frameworks = app / 'Contents/Frameworks'
        resources = app / 'Contents/Resources'
        resources.mkdir(parents=True)
        inputs = [(args.gui, macos / 'wooting-gui'), (args.binary, macos / 'wooting-signals'),
                  (args.service, macos / 'wooting-service'),
                  (args.rgb_sdk, frameworks / 'libwooting-rgb-sdk.dylib'),
                  (args.analog_sdk, frameworks / 'libwooting_analog_sdk_dist.dylib')]
        origins = release.bundle_dependencies(app, 'Darwin', library_dir='Contents/Frameworks', sign=False)(inputs)
        for relative in origins:
            validate_native(app / relative, args.architecture, args.minimum_os)
        release.validate_notices(args.notices, origins)
        shutil.copytree(args.notices, resources / 'notices')
        shutil.copytree(release.ROOT / 'examples', resources / 'examples')
        for path in macos.iterdir():
            path.chmod(0o755)
        info = {'CFBundleExecutable': 'wooting-gui', 'CFBundleName': 'Wooting Signals',
                'CFBundleDisplayName': 'Wooting Signals', 'CFBundleIdentifier': args.identifier,
                'CFBundlePackageType': 'APPL', 'CFBundleInfoDictionaryVersion': '6.0',
                'CFBundleShortVersionString': args.version, 'CFBundleVersion': args.version,
                'LSMinimumSystemVersion': args.minimum_os, 'NSHighResolutionCapable': True,
                'LSApplicationCategoryType': 'public.app-category.utilities',
                'NSInputMonitoringUsageDescription': 'Access your Wooting keyboard for lighting and key-travel effects. Key input is not recorded.',
                'NSHumanReadableCopyright': 'Copyright © 2026 The Keyboard Goblins (Wooting Signals contributors)'}
        if args.icon:
            shutil.copy2(args.icon, resources / 'AppIcon.icns')
        else:
            create_icon(resources / 'AppIcon.icns')
        info['CFBundleIconFile'] = 'AppIcon'
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
        mode = 'notarized' if args.notary_profile else ('developer-id-unnotarized' if args.sign_identity else 'ad-hoc-local-test-only')
        provenance = {'format': 1, 'version': args.version, 'identifier': args.identifier,
                      'architecture': args.architecture, 'minimum_os': args.minimum_os,
                      'license_review': 'pending-not-for-distribution' if args.local_test else 'operator-verified',
                      'sdk_versions': {'rgb': args.rgb_sdk_version, 'analog': args.analog_sdk_version},
                      'distribution': mode, 'source_revision': release.run('git', '-C', release.ROOT, 'rev-parse', 'HEAD'),
                      'source_dirty': bool(release.run('git', '-C', release.ROOT, 'status', '--porcelain')),
                      'native_inputs': {name: {'source_name': Path(source).name, 'sha256': digest(Path(source))}
                                        for name, source in sorted(origins.items())}}
        (resources / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
        for relative in sorted(origins):
            sign(app / relative, args.sign_identity)
        sign(app, args.sign_identity)
        run('codesign', '--verify', '--deep', '--strict', app)
        if args.notary_profile:
            archive = work / 'notarization.zip'
            run('ditto', '-c', '-k', '--keepParent', app, archive)
            notarize(archive, args.notary_profile)
            run('xcrun', 'stapler', 'staple', app)
            run('xcrun', 'stapler', 'validate', app)
        image_root = work / 'image'
        image_root.mkdir()
        shutil.copytree(app, image_root / APP, symlinks=True)
        (image_root / 'Applications').symlink_to('/Applications')
        (image_root / 'READ ME.txt').write_text(
            'Drag Wooting Signals.app to Applications, then launch it.\n'
            'Login startup is optional. Updates: quit the app and stop its engine before replacing.\n'
            + ('LOCAL TEST BUILD: ad-hoc signed, not notarized; not for public distribution.\n'
               if not args.sign_identity else 'Signing status: ' + mode + '\n'))
        dmg = product / ('Wooting-Signals-' + args.version + '-' + args.architecture + '.dmg')
        run('hdiutil', 'create', '-volname', 'Wooting Signals', '-srcfolder', image_root,
            '-format', 'UDZO', '-ov', dmg)
        if args.sign_identity:
            run('codesign', '--force', '--sign', args.sign_identity, '--timestamp', dmg)
        if args.notary_profile:
            notarize(dmg, args.notary_profile)
            run('xcrun', 'stapler', 'staple', dmg)
            run('xcrun', 'stapler', 'validate', dmg)
        provenance['files'] = {str(p.relative_to(product)): digest(p)
                               for p in sorted(product.rglob('*')) if p.is_file()}
        (product / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
        (product / 'SHA256SUMS').write_text(digest(dmg) + '  ' + dmg.name + '\n')
        publish_directory(product, args.output)
    return args.output


def main():
    p = parser()
    args = p.parse_args()
    try:
        print(build(args))
    except (RuntimeError, OSError, ValueError, subprocess.CalledProcessError) as error:
        p.exit(1, str(error) + '\n')


if __name__ == '__main__':
    main()
