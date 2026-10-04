import hashlib
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


service = load('service', ROOT / 'packaging/wooting-service')
installer = load('installer', ROOT / 'packaging/install.py')
release = load('release', ROOT / 'packaging/release.py')


class Isolated(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wooting packaging ')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.env = patch.dict(os.environ, {
            'HOME': str(self.home), 'WOOTING_STATE_DIR': '', 'XDG_CONFIG_HOME': str(self.home / 'config'),
            'XDG_STATE_HOME': str(self.home / 'state'), 'XDG_DATA_HOME': str(self.home / 'data')})
        self.env.start()
        self.addCleanup(self.env.stop)

    def executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('#!/bin/sh\n' + body + '\n')
        path.chmod(0o755)
        return path


class InstallTests(Isolated):
    def package(self, system, gui=True):
        package = self.home / 'release'
        package.mkdir(exist_ok=True)
        self.executable(package / 'bin/wooting-signals', 'exit 1')
        self.executable(package / 'bin/wooting-service', '''printf '%s\\n' '{"ok":true,"supported":true,"enabled":false,"running":false}' ''')
        if gui:
            self.executable(package / 'bin/wooting-gui', 'exit 0')
        suffix = 'dylib' if system == 'Darwin' else 'so'
        lib = package / 'lib/wooting-signals'
        lib.mkdir(parents=True, exist_ok=True)
        (lib / ('libwooting-rgb-sdk.' + suffix)).write_text('fake RGB')
        (lib / ('libwooting_analog_sdk_dist.' + suffix)).write_text('fake Analog')
        share = package / 'share/wooting-signals'
        share.mkdir(parents=True, exist_ok=True)
        (share / 'NOTICES.md').write_text('test fixture notices only')
        manifest = {'system': system, 'architecture': platform.machine(),
                    'files': {str(p.relative_to(package)): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in package.rglob('*') if p.is_file() and p.name != 'manifest.json'}}
        (package / 'manifest.json').write_text(json.dumps(manifest))
        return package

    def test_dry_run_has_no_side_effects(self):
        prefix = self.home / 'prefix'
        subprocess.run([sys.executable, str(ROOT / 'packaging/install.py'), 'install', '--prefix', str(prefix)],
                       check=True, capture_output=True)
        self.assertFalse(prefix.exists())
        self.assertFalse((self.home / 'Library').exists())

    def test_install_upgrade_uninstall_preserve_state_both_platforms(self):
        for system in ('Darwin', 'Linux'):
            with self.subTest(system=system):
                package = self.package(system)
                prefix = self.home / 'installed'
                config = self.home / 'config/wooting-signals/config.toml'
                state = self.home / 'state/wooting-signals/state.json'
                config.parent.mkdir(parents=True, exist_ok=True)
                state.parent.mkdir(parents=True, exist_ok=True)
                config.write_text('user-config')
                state.write_text('user-state')
                installer.install(package, prefix, system, self.home)
                self.assertTrue((prefix / installer.MARKER).exists())
                self.assertFalse((self.home / 'Library/LaunchAgents').exists())
                self.assertFalse((self.home / 'config/systemd').exists())
                installer.install(package, prefix, system, self.home)
                installer.uninstall(prefix, system, self.home)
                self.assertFalse(prefix.exists())
                self.assertEqual(config.read_text(), 'user-config')
                self.assertEqual(state.read_text(), 'user-state')
                shutil.rmtree(package)

    def test_missing_sdk_fails_without_writes(self):
        package = self.package('Linux')
        (package / 'lib/wooting-signals/libwooting_analog_sdk_dist.so').unlink()
        prefix = self.home / 'installed'
        with self.assertRaises((RuntimeError, FileNotFoundError)):
            installer.install(package, prefix, 'Linux', self.home)
        self.assertFalse(prefix.exists())

    def test_hash_mismatch_rejected(self):
        package = self.package('Linux')
        (package / 'bin/wooting-signals').write_text('modified')
        with self.assertRaisesRegex(RuntimeError, 'checksum'):
            installer.install(package, self.home / 'installed', 'Linux', self.home)

    def test_unmanaged_prefix_not_overwritten(self):
        package = self.package('Linux')
        prefix = self.home / 'unmanaged'
        prefix.mkdir()
        with self.assertRaisesRegex(RuntimeError, 'managed'):
            installer.install(package, prefix, 'Linux', self.home)
        self.assertTrue(prefix.exists())

    def test_running_engine_blocks_upgrade(self):
        package = self.package('Linux')
        prefix = self.home / 'installed'
        installer.install(package, prefix, 'Linux', self.home)
        self.executable(prefix / 'bin/wooting-signals', 'exit 0')
        with self.assertRaisesRegex(RuntimeError, 'stop the engine'):
            installer.install(package, prefix, 'Linux', self.home)
        with self.assertRaisesRegex(RuntimeError, 'reachable'):
            installer.uninstall(prefix, 'Linux', self.home)
        self.assertTrue(prefix.exists())

    def test_uninstall_requires_removing_inactive_registration(self):
        package = self.package('Linux')
        prefix = self.home / 'installed'
        installer.install(package, prefix, 'Linux', self.home)
        registration = self.home / 'config/systemd/user/wooting-signals.service'
        registration.parent.mkdir(parents=True)
        registration.write_text('inactive managed unit fixture')
        with self.assertRaisesRegex(RuntimeError, 'disable/stop'):
            installer.uninstall(prefix, 'Linux', self.home)
        self.assertTrue(prefix.exists())

    def test_upgrade_rolls_back_on_launcher_failure(self):
        package = self.package('Linux')
        prefix = self.home / 'installed'
        installer.install(package, prefix, 'Linux', self.home)
        original = (prefix / 'bin/wooting-signals').read_bytes()
        with patch.object(installer, 'integrations', side_effect=RuntimeError('launcher failure')):
            with self.assertRaisesRegex(RuntimeError, 'launcher failure'):
                installer.install(package, prefix, 'Linux', self.home)
        self.assertEqual((prefix / 'bin/wooting-signals').read_bytes(), original)
        self.assertFalse(prefix.with_name(prefix.name + '.previous').exists())

    def test_mac_launcher_quotes_paths(self):
        prefix = self.home / "space and ' quote"
        self.executable(prefix / 'bin/wooting-gui', 'exit 0')
        installer.integrations(prefix, 'Darwin', self.home)
        app = self.home / 'Applications/Wooting Signals.app'
        plist = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        self.assertEqual(plist['CFBundleExecutable'], 'wooting-gui')
        subprocess.run(['sh', '-n', str(app / 'Contents/MacOS/wooting-gui')], check=True)
        self.assertNotIn(str(ROOT), (app / 'Contents/MacOS/wooting-gui').read_text())


class ServiceTests(Isolated):
    def setUp(self):
        super().setUp()
        self.binary = self.executable(self.home / 'prefix/bin/wooting-signals', 'exit 1')
        self.calls = []
        self.loaded = False
        self.running = False
        self.enabled = False

    def fake_run(self, args, check=True):
        self.calls.append(args)
        code, output = 0, ''
        if args[0] == 'launchctl':
            command = args[1]
            if command == 'print':
                code = 0 if self.loaded else 113
                output = 'state = running' if self.running else 'state = not running'
            elif command == 'bootstrap':
                self.loaded = True
            elif command == 'kickstart':
                self.running = True
            elif command == 'bootout':
                self.loaded = self.running = False
        else:
            command = args[2]
            if command == 'is-enabled':
                output = 'enabled' if self.enabled else 'disabled'
                code = 0 if self.enabled else 1
            elif command == 'is-active':
                output = 'active' if self.running else 'inactive'
                code = 0 if self.running else 3
            elif command == 'enable':
                self.enabled = True
            elif command == 'disable':
                self.enabled = False
            elif command == 'start':
                self.running = True
            elif command == 'stop':
                self.running = False
        return subprocess.CompletedProcess(args, code, output, '')

    def test_opt_in_actions_both_platforms(self):
        for system in ('Darwin', 'Linux'):
            with self.subTest(system=system), patch.object(service, 'run', self.fake_run), patch.object(service.shutil, 'which', return_value='/fake/manager'):
                def act(command):
                    return service.action(command, system, self.home, self.binary)
                initial = act('status')
                self.assertFalse(initial['enabled'])
                self.assertFalse(initial['running'])
                enabled = act('enable')
                self.assertTrue(enabled['enabled'])
                self.assertFalse(enabled['running'])
                self.assertTrue(act('start')['running'])
                stopped = act('stop')
                self.assertFalse(stopped['running'])
                self.assertTrue(stopped['enabled'])
                disabled = act('disable')
                self.assertFalse(disabled['enabled'])
                self.assertFalse(disabled['running'])
                self.assertTrue(act('disable')['ok'])

    def test_start_does_not_enable_login(self):
        for system in ('Darwin', 'Linux'):
            with self.subTest(system=system), patch.object(service, 'run', self.fake_run), patch.object(service.shutil, 'which', return_value='/fake/manager'):
                result = service.action('start', system, self.home, self.binary)
                self.assertTrue(result['running'])
                self.assertFalse(result['enabled'])
                service.action('disable', system, self.home, self.binary)

    def test_service_sdk_paths_are_installed(self):
        for system in ('Darwin', 'Linux'):
            _, binary, state, registration = service.paths(system, self.home, self.binary)
            service.write_registration(system, binary, state, registration, True)
            contents = registration.read_text()
            self.assertIn('lib/wooting-signals/libwooting', contents)
            self.assertNotIn(str(ROOT), contents)
            if system == 'Darwin':
                args = plistlib.loads(registration.read_bytes())['ProgramArguments']
                self.assertEqual(args[1], 'engine')
                self.assertEqual(args[-1], str(self.home / 'Library/Application Support/wooting-signals/runtime'))
            else:
                self.assertIn(str(self.home / 'state/wooting-signals'), contents)

    def test_unavailable_manager_status_is_safe(self):
        with patch.object(service.shutil, 'which', return_value=None):
            self.assertEqual(service.action('status', 'Linux')['supported'], False)
            self.assertEqual(service.action('start', 'Linux')['ok'], False)

    def test_custom_state_rejects_mutating_service_actions(self):
        with patch.dict(os.environ, {'WOOTING_STATE_DIR': str(self.home / 'custom')}), patch.object(service, 'run') as run:
            for command in ('enable', 'disable', 'start', 'stop'):
                with self.assertRaisesRegex(RuntimeError, 'default state only'):
                    service.action(command, 'Linux', self.home, self.binary)
            run.assert_not_called()

    def test_cli_json_errors(self):
        result = subprocess.run([sys.executable, str(ROOT / 'packaging/wooting-service'), 'invalid'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertFalse(json.loads(result.stdout)['ok'])


class ReleaseTests(Isolated):
    def test_native_license_coverage_required(self):
        (self.home / 'native-licenses.json').write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'missing native license'):
            release.validate_notices(self.home, ['lib/library.so'])
        (self.home / 'notice.txt').write_text('Fixture license')
        (self.home / 'native-licenses.json').write_text('{"library.so":"notice.txt"}')
        release.validate_notices(self.home, ['lib/library.so'])

    def test_linux_recursive_dependency_relocation(self):
        origin = self.home / 'original'
        origin.mkdir()
        binary = origin / 'wooting-signals'
        binary.write_text('ELF fixture')
        dependency = origin / 'libhidapi.so.0'
        dependency.write_text('ELF fixture dependency')
        calls = []
        def run(*args):
            calls.append(args)
            if args[0] == 'ldd':
                if Path(args[1]).resolve() == binary.resolve():
                    return f'libhidapi.so.0 => {dependency} (0x1)\nlibc.so.6 => /lib/libc.so.6 (0x2)'
                return 'libc.so.6 => /lib/libc.so.6 (0x2)'
            return ''
        # ldd paths cannot have whitespace; use a fixture path without spaces.
        with tempfile.TemporaryDirectory(prefix='wooting-elf-') as no_space:
            binary2 = Path(no_space) / binary.name
            dependency2 = Path(no_space) / dependency.name
            shutil.copy2(binary, binary2)
            shutil.copy2(dependency, dependency2)
            binary, dependency = binary2, dependency2
            stage = self.home / 'stage'
            with patch.object(release, 'run', run):
                result = release.bundle_dependencies(stage, 'Linux')([(binary, stage / 'bin/wooting-signals')])
            self.assertIn('lib/wooting-signals/libhidapi.so.0', result)
            self.assertFalse((stage / 'lib/wooting-signals/libc.so.6').exists())
            self.assertTrue(any('$ORIGIN/../lib/wooting-signals' in args for args in calls))
            self.assertTrue(any('$ORIGIN' in args for args in calls))

    def test_native_fixture_runs_after_sources_removed(self):
        system = platform.system()
        required = ['cc', 'otool', 'install_name_tool', 'codesign'] if system == 'Darwin' else ['cc', 'patchelf', 'ldd']
        if system not in ('Darwin', 'Linux') or any(not shutil.which(tool) for tool in required):
            self.skipTest('native fixture toolchain unavailable')
        origin = self.home / 'native-origin'
        origin.mkdir()
        suffix = 'dylib' if system == 'Darwin' else 'so'
        dependency = origin / ('libfixture.' + suffix)
        (origin / 'library.c').write_text('int fixture(void) { return 0; }')
        (origin / 'main.c').write_text('extern int fixture(void); int main(void) { return fixture(); }')
        library_flags = ['-dynamiclib', '-Wl,-install_name,' + str(dependency)] if system == 'Darwin' else ['-shared', '-fPIC', '-Wl,-soname,' + dependency.name]
        subprocess.run(['cc', *library_flags, str(origin / 'library.c'), '-o', str(dependency)], check=True, capture_output=True)
        binary = origin / 'wooting-signals'
        flags = ['-Wl,-headerpad_max_install_names'] if system == 'Darwin' else ['-Wl,-rpath,' + str(origin)]
        subprocess.run(['cc', str(origin / 'main.c'), str(dependency), *flags, '-o', str(binary)], check=True, capture_output=True)
        stage = self.home / 'native-stage'
        result = release.bundle_dependencies(stage, system)([(binary, stage / 'bin/wooting-signals')])
        self.assertIn('lib/wooting-signals/' + dependency.name, result)
        shutil.rmtree(origin)
        subprocess.run([str(stage / 'bin/wooting-signals')], check=True, capture_output=True)
        if system == 'Darwin':
            dependencies = release.mac_dependencies(stage / 'bin/wooting-signals')
            self.assertTrue(any(item.startswith('@loader_path/') for item in dependencies))
            self.assertFalse(any('native-origin' in item for item in dependencies))

    def test_unresolved_dependency_rejected(self):
        binary = self.home / 'binary'
        binary.write_text('fixture')
        stage = self.home / 'stage'
        with patch.object(release, 'run', return_value='libmissing.so => not found'):
            with self.assertRaisesRegex(RuntimeError, 'unresolved ELF'):
                release.bundle_dependencies(stage, 'Linux')([(binary, stage / 'bin/wooting-signals')])

    def test_shell_syntax(self):
        for script in (ROOT / 'scripts').glob('*.sh'):
            subprocess.run(['bash', '-n', str(script)], check=True)


if __name__ == '__main__':
    unittest.main()
