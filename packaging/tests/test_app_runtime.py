"""Opt-in app relocation/SDK discovery with synthetic SDKs and a fail-closed loader.

No WOOTING_* SDK override or DYLD_LIBRARY_PATH is used. A test-only dyld
interposer permits non-system dlopen ONLY for our two compiled fixture files; all other
non-system paths exit before loading code. Apple system runtimes are allowed. Verify the guard on a standalone
probe, then verify its constructor runs in the copied CLI via --version before
invoking any command capable of initializing an SDK. Never launches GUI/services.
"""
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
RGB = 'libwooting-rgb-sdk.dylib'
ANALOG = 'libwooting_analog_sdk_dist.dylib'
GUARD_C = r'''
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <limits.h>
static void record(const char *verb, const char *path) {
    const char *log = getenv("TEST_LOADER_LOG");
    if (!log) _exit(90);
    FILE *f = fopen(log, "a");
    if (!f) _exit(90);
    fprintf(f, "%s:%s\n", verb, path ? path : "NULL");
    fclose(f);
}
__attribute__((constructor)) static void ready(void) { record("ready", "guard"); }
static void *guarded_dlopen(const char *path, int flags) {
    /* Apple runtime dependencies on the sealed system volume, never vendor SDKs. */
    if (path && !strstr(path, "..") && !strstr(path, "wooting")
        && (!strncmp(path, "/System/Library/", 16) || !strncmp(path, "/usr/lib/", 9)))
        return dlopen(path, flags);
    char actual[PATH_MAX];
    const char *rgb = getenv("TEST_ALLOWED_RGB");
    const char *analog = getenv("TEST_ALLOWED_ANALOG");
    if (!path || !rgb || !analog || !realpath(path, actual)
        || (strcmp(actual, rgb) && strcmp(actual, analog))) {
        record("denied", path);
        _exit(91);
    }
    record("allowed", path);
    return dlopen(path, flags);
}
__attribute__((used, section("__DATA,__interpose")))
static struct { const void *replacement; const void *original; } mapping = {
    (const void *)guarded_dlopen, (const void *)dlopen
};
'''


@unittest.skipUnless(platform.system() == 'Darwin' and os.environ.get('WOOTING_TEST_BINARY'),
                     'macOS and WOOTING_TEST_BINARY required for guarded app integration')
class AppRuntimeIntegration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wa-', dir='/tmp')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve()
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(('WOOTING_', 'MOCK_', 'DYLD_', 'TEST_'))}
        self.env.update(HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / 'c'),
                        XDG_STATE_HOME=str(self.home / 's'), XDG_DATA_HOME=str(self.home / 'd'),
                        WOOTING_STATE_DIR=str(self.home / 'state'))
        self.app = self.home / 'Original App.app'
        self.binary = self.app / 'Contents/MacOS/wooting-signals'
        self.binary.parent.mkdir(parents=True)
        shutil.copy2(Path(os.environ['WOOTING_TEST_BINARY']).resolve(), self.binary)
        self.frameworks = self.app / 'Contents/Frameworks'
        self.frameworks.mkdir()
        fixture = (ROOT / 'tests/toys_cli.rs').read_text().split('const MOCK_C: &str = r#"', 1)[1].split('"#;', 1)[0]
        self.compile('mock', fixture, self.frameworks / RGB, library=True)
        shutil.copy2(self.frameworks / RGB, self.frameworks / ANALOG)
        guard = self.home / 'loader-guard.dylib'
        self.compile('guard', GUARD_C, guard, library=True)
        self.probe = self.home / 'probe'
        self.compile('probe', '#include <dlfcn.h>\nint main(int n,char **v) { return dlopen(v[1], RTLD_NOW) ? 0 : 2; }', self.probe)
        self.log = self.home / 'sdk-calls.log'
        self.loader_log = self.home / 'loader.log'
        self.env.update(MOCK_LOG=str(self.log), TEST_LOADER_LOG=str(self.loader_log),
                        DYLD_INSERT_LIBRARIES=str(guard))
        self.set_allowed_paths()
        self.processes = []
        self.addCleanup(self.stop_processes)
        # Probe calls only fixture/nonexistent files; never any real SDK.
        probe = self.invoke(self.probe, str(self.frameworks / RGB))
        self.assertEqual(probe.returncode, 0, probe.stderr)
        self.assertIn('allowed:', self.loader_log.read_text())
        denied = self.invoke(self.probe, str(self.home / 'nonexistent.dylib'))
        self.assertEqual(denied.returncode, 91, 'dyld guard did not intercept dlopen')
        self.assertIn('denied:', self.loader_log.read_text())
        self.verify_cli_guard()
        self.loader_log.unlink()

    def compile(self, name, source, output, library=False):
        path = self.home / (name + '.c')
        path.write_text(source)
        subprocess.run(['cc', *(['-dynamiclib'] if library else []), str(path), '-o', str(output)],
                       check=True, capture_output=True, timeout=30)

    def set_allowed_paths(self):
        self.env.update(TEST_ALLOWED_RGB=str((self.frameworks / RGB).resolve()),
                        TEST_ALLOWED_ANALOG=str((self.frameworks / ANALOG).resolve()))

    def invoke(self, binary, *args):
        return subprocess.run([str(binary), *args], cwd=self.home, env=self.env,
                              capture_output=True, text=True, timeout=10)

    def verify_cli_guard(self):
        self.loader_log.unlink(missing_ok=True)
        result = self.invoke(self.binary, '--version')
        self.assertEqual(result.returncode, 0, result.stderr + (self.loader_log.read_text() if self.loader_log.exists() else 'no guard log'))
        self.assertTrue(self.loader_log.exists(), 'CLI ignored DYLD_INSERT_LIBRARIES; unsafe to continue')
        self.assertIn('ready:guard', self.loader_log.read_text())

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

    def control(self, *args, check=True):
        result = self.invoke(self.binary, 'control', *args)
        if check:
            self.assertEqual(result.returncode, 0, result.stderr + result.stdout + self.loader_log.read_text())
        return json.loads(result.stdout)

    def start(self):
        process = subprocess.Popen([str(self.binary), 'engine'], cwd=self.home, env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.processes.append(process)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if process.poll() is not None:
                self.fail('engine exited: ' + process.stderr.read())
            result = self.control('status', check=False)
            if result['ok']:
                return process, result['status']
            time.sleep(.02)
        self.fail('engine did not become ready')

    def test_moved_app_discovers_both_sdks_and_preserves_paused_state(self):
        self.assertNotIn('WOOTING_RGB_SDK_PATH', self.env)
        self.assertNotIn('WOOTING_ANALOG_SDK_PATH', self.env)
        self.assertNotIn('DYLD_LIBRARY_PATH', self.env)
        process, status = self.start()
        self.assertEqual(status['state'], 'paused')
        self.control('select', '--preset', 'ripples')
        self.control('settings', '--brightness', '27', '--fps', '25')
        self.assertFalse(self.log.exists(), 'paused engine initialized an SDK')
        self.control('stop')
        self.assertEqual(process.wait(timeout=5), 0)
        saved = (self.home / 'state/state.json').read_bytes()
        old_app = self.app
        self.app = self.home / 'Moved App.app'
        shutil.move(str(old_app), self.app)
        self.assertFalse(old_app.exists())
        self.binary = self.app / 'Contents/MacOS/wooting-signals'
        self.frameworks = self.app / 'Contents/Frameworks'
        self.set_allowed_paths()
        self.verify_cli_guard()
        self.assertEqual((self.home / 'state/state.json').read_bytes(), saved)
        process, status = self.start()
        self.assertEqual(status['state'], 'paused')
        self.assertEqual(status['mode'], 'ripples')
        self.assertEqual(status['brightness'], 27)
        self.assertEqual(status['fps'], 25)
        self.assertFalse(self.log.exists())
        self.assertEqual(self.control('resume')['status']['state'], 'active')
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if self.log.exists() and 'read' in self.log.read_text().splitlines():
                break
            time.sleep(.02)
        self.control('pause')
        self.control('stop')
        self.assertEqual(process.wait(timeout=5), 0)
        calls = self.log.read_text().splitlines()
        for call in ('rgb-open', 'init', 'read', 'uninit', 'close'):
            self.assertIn(call, calls)
        records = self.loader_log.read_text().splitlines()
        self.assertFalse(any(line.startswith('denied:') for line in records), records)
        requested = [line.removeprefix('allowed:') for line in records if line.startswith('allowed:')]
        self.assertTrue(requested, records)
        self.assertEqual({Path(path).resolve() for path in requested},
                         {self.frameworks / RGB, self.frameworks / ANALOG})
        for path in requested:
            self.assertTrue(Path(path).is_absolute())
            self.assertIn('/Contents/', path)
            self.assertIn('/Frameworks/', path)


if __name__ == '__main__':
    unittest.main()
