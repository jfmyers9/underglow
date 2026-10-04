"""Opt-in real CLI integration, exclusively using temporary state and synthetic SDKs.

Run after cargo build with WOOTING_TEST_BINARY=/absolute/path/to/wooting-signals.
The authoritative SDK overrides are deliberate: a loader regression must never
fall through to any SDK installed on a developer's real host.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import signal
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

from test_packaging import ROOT, installer, service


@unittest.skipUnless(os.environ.get('WOOTING_TEST_BINARY'), 'set WOOTING_TEST_BINARY to test real CLI')
class RuntimeIntegration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wp-', dir='/tmp')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.environment = dict(os.environ)
        for key in list(self.environment):
            if key.startswith(('WOOTING_', 'MOCK_')):
                self.environment.pop(key)
        self.environment.update(HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / 'c'),
                                XDG_STATE_HOME=str(self.home / 's'), XDG_DATA_HOME=str(self.home / 'd'))
        self.system = platform.system()
        self.package = self.home / 'release'
        binary = self.package / 'bin/wooting-signals'
        binary.parent.mkdir(parents=True)
        shutil.copy2(Path(os.environ['WOOTING_TEST_BINARY']).resolve(), binary)
        shutil.copy2(ROOT / 'packaging/wooting-service', binary.with_name('wooting-service'))
        # Reuse the existing audited hardware-free ABI fixture, not an SDK.
        source = (ROOT / 'tests/toys_cli.rs').read_text().split('const MOCK_C: &str = r#"', 1)[1].split('"#;', 1)[0]
        c_file = self.home / 'mock.c'
        c_file.write_text(source)
        lib = self.package / 'lib/wooting-signals'
        lib.mkdir(parents=True)
        suffix = 'dylib' if self.system == 'Darwin' else 'so'
        self.rgb_name = 'libwooting-rgb-sdk.' + suffix
        self.analog_name = 'libwooting_analog_sdk_dist.' + suffix
        flags = ['-dynamiclib'] if self.system == 'Darwin' else ['-shared', '-fPIC']
        subprocess.run(['cc', *flags, str(c_file), '-o', str(lib / self.rgb_name)], check=True, capture_output=True)
        shutil.copy2(lib / self.rgb_name, lib / self.analog_name)
        share = self.package / 'share/wooting-signals'
        share.mkdir(parents=True)
        (share / 'NOTICES.md').write_text('Synthetic test payload; not a distributable release')
        self.manifest()
        self.prefix = self.home / 'prefix'
        self.log = self.home / 'sdk-calls.log'
        self.environment.update(WOOTING_RGB_SDK_PATH=str(self.prefix / 'lib/wooting-signals' / self.rgb_name),
                                WOOTING_ANALOG_SDK_PATH=str(self.prefix / 'lib/wooting-signals' / self.analog_name),
                                MOCK_LOG=str(self.log))
        # Fake executables intercept every helper call; never query host services.
        fakebin = self.home / 'fakebin'
        fakebin.mkdir()
        for name in ('launchctl', 'systemctl'):
            path = fakebin / name
            path.write_text('#!/bin/sh\nexit 1\n')
            path.chmod(0o755)
        self.environment['PATH'] = str(fakebin) + os.pathsep + os.environ['PATH']
        self.patch = patch.dict(os.environ, self.environment, clear=True)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.processes = []
        self.addCleanup(self.stop_processes)
        installer.install(self.package, self.prefix, self.system, self.home)
        self.binary = self.prefix / 'bin/wooting-signals'
        self.state = service.paths(self.system, self.home, self.binary)[2]

    def manifest(self):
        files = {str(p.relative_to(self.package)): hashlib.sha256(p.read_bytes()).hexdigest()
                 for p in self.package.rglob('*') if p.is_file() and p.name != 'manifest.json'}
        (self.package / 'manifest.json').write_text(json.dumps({
            'system': self.system, 'architecture': platform.machine(), 'files': files}))

    def stop_processes(self):
        for process in self.processes:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            process.stdout.close()
            process.stderr.close()

    def command(self, *args, check=True):
        result = subprocess.run([str(self.binary), 'control', *args], cwd=self.home,
                                env=self.environment, capture_output=True, text=True, timeout=10)
        if check:
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        return json.loads(result.stdout)

    def start(self, args=None):
        process = subprocess.Popen(args or [str(self.binary), 'engine'], cwd=self.home,
                                   env=self.environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.processes.append(process)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if process.poll() is not None:
                self.fail('engine exited: ' + process.stderr.read())
            result = self.command('status', check=False)
            if result['ok']:
                return process, result['status']
            time.sleep(.02)
        self.fail('engine did not become ready')

    def test_install_engine_upgrade_uninstall_boundary(self):
        process, status = self.start()
        self.assertEqual(status['state'], 'paused')
        self.assertFalse(self.log.exists(), 'fresh paused engine must never initialize SDKs')
        self.assertEqual(self.command('select', '--preset', 'rainbow')['status']['mode'], 'rainbow')
        self.command('settings', '--brightness', '27', '--fps', '25')
        self.assertFalse(self.log.exists())
        with self.assertRaisesRegex(RuntimeError, 'stop the engine'):
            installer.install(self.package, self.prefix, self.system, self.home)
        with self.assertRaisesRegex(RuntimeError, 'reachable'):
            installer.uninstall(self.prefix, self.system, self.home)
        self.command('stop')
        self.assertEqual(process.wait(timeout=5), 0)
        saved = (self.state / 'state.json').read_bytes()
        installer.install(self.package, self.prefix, self.system, self.home)
        self.assertEqual((self.state / 'state.json').read_bytes(), saved)
        restarted, status = self.start()
        self.assertEqual(status['state'], 'paused')
        self.assertEqual(status['mode'], 'rainbow')
        self.assertEqual(status['brightness'], 27)
        self.assertEqual(status['fps'], 25)
        self.command('stop')
        self.assertEqual(restarted.wait(timeout=5), 0)
        installer.uninstall(self.prefix, self.system, self.home)
        self.assertTrue((self.state / 'state.json').exists())
        self.assertFalse(self.prefix.exists())
        self.assertFalse(self.log.exists())

    def test_generated_service_arguments_and_sigterm_mock_cleanup(self):
        _, binary, state, registration = service.paths(self.system, self.home, self.binary)
        service.write_registration(self.system, binary, state, registration, False)
        if self.system == 'Darwin':
            plist = plistlib.loads(registration.read_bytes())
            args = plist['ProgramArguments']
            self.assertEqual(plist['EnvironmentVariables']['WOOTING_STATE_DIR'], str(state))
            self.assertEqual(plist['EnvironmentVariables']['WOOTING_RGB_SDK_PATH'], self.environment['WOOTING_RGB_SDK_PATH'])
            self.assertEqual(plist['EnvironmentVariables']['WOOTING_ANALOG_SDK_PATH'], self.environment['WOOTING_ANALOG_SDK_PATH'])
        else:
            # Paths in this fixture contain no quoting metacharacters/spaces.
            import shlex
            line = next(line for line in registration.read_text().splitlines() if line.startswith('ExecStart='))
            args = shlex.split(line.split('=', 1)[1])
        process, status = self.start(args)
        self.assertEqual(status['state'], 'paused')
        self.assertEqual(state, self.state)
        self.assertFalse(self.log.exists())
        # Only the two authoritative installed SYNTHETIC SDK paths can load.
        self.assertEqual(self.command('resume')['status']['state'], 'active')
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and (not self.log.exists() or 'read' not in self.log.read_text()):
            time.sleep(.02)
        process.send_signal(signal.SIGTERM)
        self.assertEqual(process.wait(timeout=5), 0)
        calls = self.log.read_text().splitlines()
        self.assertIn('rgb-open', calls)
        self.assertIn('init', calls)
        self.assertIn('uninit', calls)
        self.assertIn('close', calls)
        self.assertFalse(self.command('status', check=False)['ok'])


if __name__ == '__main__':
    unittest.main()
