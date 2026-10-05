#!/usr/bin/env python3
"""Install a staged release, never build or activate services implicitly."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shlex
import shutil
import subprocess
import tempfile

MARKER = '.wooting-install.json'


def installed_binary(prefix, name, legacy):
    """Manage both pre-rename installations and current ones."""
    canonical = prefix / 'bin' / name
    return canonical if canonical.exists() else prefix / 'bin' / legacy


def desktop_entry(binary):
    if '\n' in str(binary) or '\r' in str(binary):
        raise ValueError('desktop paths cannot contain newlines')
    escaped = str(binary).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
    return '[Desktop Entry]\nType=Application\nName=Underglow\nExec="' + escaped + '"\nTerminal=false\nCategories=Utility;\n'


def integrations(prefix, system, home, remove=False):
    if system == 'Darwin':
        app = home / 'Applications/Underglow.app'
        legacy = home / 'Applications/Wooting Signals.app'
        legacy_marker = legacy / 'Contents/wooting-prefix'
        # Only retire our old prefix launcher, never a self-contained/unrelated app.
        if remove:
            if legacy_marker.is_file() and legacy_marker.read_text() == str(prefix):
                shutil.rmtree(legacy)
            # Only remove a launcher that belongs to this prefix.
            marker = app / 'Contents/wooting-prefix'
            if marker.exists() and marker.read_text() == str(prefix):
                shutil.rmtree(app)
            return
        if not (prefix / 'bin/underglow-gui').exists():
            return
        marker = app / 'Contents/wooting-prefix'
        if app.exists() and (not marker.exists() or marker.read_text() != str(prefix)):
            raise RuntimeError('refusing to replace unrelated application: ' + str(app))
        macos = app / 'Contents/MacOS'
        macos.mkdir(parents=True, exist_ok=True)
        (app / 'Contents/wooting-prefix').write_text(str(prefix))
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
            'CFBundleName': 'Underglow', 'CFBundleIdentifier': 'io.github.jfmyers9.wooting-signals.gui',
            'CFBundleExecutable': 'underglow-gui', 'CFBundlePackageType': 'APPL',
            'NSHighResolutionCapable': True}))
        launcher = macos / 'underglow-gui'
        launcher.write_text('#!/bin/sh\nexec ' + shlex.quote(str(prefix / 'bin/underglow-gui')) + ' "$@"\n')
        launcher.chmod(0o755)
        if legacy_marker.is_file() and legacy_marker.read_text() == str(prefix):
            shutil.rmtree(legacy)
    else:
        data = Path(os.environ.get('XDG_DATA_HOME', home / '.local/share'))
        desktop = data / 'applications/wooting-signals.desktop'
        content = desktop_entry(prefix / 'bin/underglow-gui')
        old_content = desktop_entry(prefix / 'bin/wooting-gui').replace('Name=Underglow', 'Name=Wooting Signals')
        if remove:
            if desktop.exists() and desktop.read_text() in (content, old_content):
                desktop.unlink()
        elif (prefix / 'bin/underglow-gui').exists():
            if desktop.exists() and desktop.read_text() not in (content, old_content):
                raise RuntimeError('refusing to replace unrelated desktop entry')
            desktop.parent.mkdir(parents=True, exist_ok=True)
            desktop.write_text(content)


def install(package, prefix, system, home):
    if os.environ.get('WOOTING_STATE_DIR'):
        raise RuntimeError('unset WOOTING_STATE_DIR before managing the default installation')
    manifest = json.loads((package / 'manifest.json').read_text())
    if prefix.is_symlink():
        raise RuntimeError('installation prefix must not be a symlink')
    if manifest.get('architecture') != platform.machine():
        raise RuntimeError('release architecture does not match this system')
    if manifest['system'] != system:
        raise RuntimeError('release platform does not match this system')
    files = manifest.get('files')
    if not isinstance(files, dict) or not files:
        raise RuntimeError('release manifest is missing file hashes')
    for candidate in package.rglob('*'):
        if candidate.is_symlink():
            raise RuntimeError('release must not contain symlinks: ' + str(candidate))
        if candidate.is_file() and candidate.name != 'manifest.json' and str(candidate.relative_to(package)) not in files:
            raise RuntimeError('untracked release file: ' + str(candidate))
    for name, digest in files.items():
        path = package / name
        if not path.resolve().is_relative_to(package.resolve()) or path.is_symlink():
            raise RuntimeError('unsafe manifest path: ' + name)
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise RuntimeError('release checksum mismatch: ' + name)
    required = ['bin/underglow', 'bin/underglow-service', 'share/wooting-signals/NOTICES.md']
    suffix = 'dylib' if system == 'Darwin' else 'so'
    required += ['lib/wooting-signals/libwooting-rgb-sdk.' + suffix,
                 'lib/wooting-signals/libwooting_analog_sdk_dist.' + suffix]
    for name in required:
        if name not in files or not (package / name).is_file():
            raise RuntimeError('incomplete release: ' + name)
    if prefix.exists() and not (prefix / MARKER).is_file():
        raise RuntimeError('prefix must be absent or a managed Wooting install (use a dedicated directory)')
    if prefix.exists():
        helper = installed_binary(prefix, 'underglow-service', 'wooting-service')
        result = subprocess.run([str(helper), 'status'], capture_output=True, text=True, check=True)
        direct = subprocess.run([str(installed_binary(prefix, 'underglow', 'wooting-signals')), 'control', 'status'],
                                capture_output=True, text=True)
        if json.loads(result.stdout)['running'] or direct.returncode == 0:
            raise RuntimeError('stop the engine before upgrading (underglow-service stop); nothing changed')
    prefix.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.wooting-stage-', dir=prefix.parent))
    backup = prefix.with_name(prefix.name + '.previous')
    try:
        for name in ('bin', 'lib', 'share'):
            shutil.copytree(package / name, staging / name)
        # Generate only known, sibling-relative aliases after strict manifest validation.
        # Release archives still reject all supplied symlinks.
        for legacy, canonical in [('wooting-signals', 'underglow'),
                                  ('wooting-gui', 'underglow-gui'),
                                  ('wooting-service', 'underglow-service')]:
            if (staging / 'bin' / canonical).exists():
                (staging / 'bin' / legacy).symlink_to(canonical)
        (staging / MARKER).write_text(json.dumps(manifest))
        if backup.exists():
            raise RuntimeError('previous upgrade backup exists: ' + str(backup))
        if prefix.exists():
            prefix.rename(backup)
        try:
            staging.rename(prefix)
            integrations(prefix, system, home)
        except Exception:
            if prefix.exists():
                shutil.rmtree(prefix)
            if backup.exists():
                backup.rename(prefix)
            raise
        if backup.exists():
            shutil.rmtree(backup)
    finally:
        if staging.exists():
            shutil.rmtree(staging)


def uninstall(prefix, system, home):
    if os.environ.get('WOOTING_STATE_DIR'):
        raise RuntimeError('unset WOOTING_STATE_DIR before managing the default installation')
    if prefix.is_symlink():
        raise RuntimeError('installation prefix must not be a symlink')
    if not (prefix / MARKER).is_file():
        raise RuntimeError('not a managed installation: ' + str(prefix))
    helper = installed_binary(prefix, 'underglow-service', 'wooting-service')
    result = subprocess.run([str(helper), 'status'], capture_output=True, text=True, check=True)
    status = json.loads(result.stdout)
    registration = (home / 'Library/LaunchAgents/io.github.jfmyers9.wooting-signals.plist'
                    if system == 'Darwin' else
                    Path(os.environ.get('XDG_CONFIG_HOME', home / '.config')) / 'systemd/user/wooting-signals.service')
    if status['enabled'] or status['running'] or registration.exists():
        raise RuntimeError('disable/stop the service before uninstalling; nothing changed')
    # Engine may have been started outside the service manager.
    result = subprocess.run([str(installed_binary(prefix, 'underglow', 'wooting-signals')), 'control', 'status'], capture_output=True, text=True)
    if result.returncode == 0:
        raise RuntimeError('engine is reachable; use control stop before uninstalling')
    integrations(prefix, system, home, remove=True)
    shutil.rmtree(prefix)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['install', 'uninstall'])
    parser.add_argument('--package', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--prefix', type=Path, default=Path.home() / '.local/opt/wooting-signals')
    parser.add_argument('--apply', action='store_true', help='perform changes (default is dry run)')
    args = parser.parse_args()
    prefix = args.prefix.expanduser().absolute()
    if not args.apply:
        print(f'Dry run: {args.command} {prefix}; no service, config, or state changes')
        return
    system = platform.system()
    if system not in ('Darwin', 'Linux'):
        parser.error('only macOS and Linux are supported')
    try:
        if args.command == 'install':
            install(args.package.resolve(), prefix, system, Path.home())
        else:
            uninstall(prefix, system, Path.home())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, str(error) + '\n')
    print(f'{args.command} complete; config and runtime state preserved; no service started')


if __name__ == '__main__':
    main()
