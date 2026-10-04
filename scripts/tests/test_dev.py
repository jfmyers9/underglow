"""Development supervisor tests: no Rust binaries, SDKs, or real GUI are launched."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location('dev', Path(__file__).parents[1] / 'dev.py')
dev = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(dev)

FAKE = '''#!{python}
import json, pathlib, sys, time
if len(sys.argv) > 1 and sys.argv[1] == 'control':
    print(json.dumps({{'ok': True}}))
    sys.exit(0)
while True:
    time.sleep(.1)
'''.format(python=sys.executable)


class FakeSupervisor(dev.Supervisor):
    """Exercise real copying, subprocess launch, control, and cleanup; fake compilation."""
    fail = False

    def build(self):
        self.builder = self.spawn([sys.executable, '-c', f'raise SystemExit({int(self.fail)})'])
        result = self.builder.wait() == 0
        self.builder = None
        return result


class WatchTests(unittest.TestCase):
    def test_classification(self):
        self.assertTrue(dev.gui_only({'src/bin/wooting-gui.rs', 'src/bin/gui/preview.rs'}))
        for paths in (set(), {'src/engine.rs'}, {'Cargo.lock'},
                      {'src/bin/gui/preview.rs', 'src/main.rs'}, {'build.rs'}):
            self.assertFalse(dev.gui_only(paths))

    def test_hashes_not_timestamps_and_deletion(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'src').mkdir()
            path = root / 'src/main.rs'
            path.write_text('one')
            before = dev.snapshot(root)
            original = path.stat()
            os.utime(path, None)
            self.assertEqual(before, dev.snapshot(root))
            path.write_text('two')
            os.utime(path, ns=(original.st_atime_ns, original.st_mtime_ns))
            after = dev.snapshot(root)
            self.assertEqual(dev.changes(before, after), {'src/main.rs'})
            path.unlink()
            self.assertEqual(dev.changes(after, dev.snapshot(root)), {'src/main.rs'})

    def test_embedded_preset_is_a_build_input(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'examples').mkdir()
            before = dev.snapshot(root)
            (root / 'examples/ripples.toml').write_text('[toy]')
            changed = dev.changes(before, dev.snapshot(root))
            self.assertEqual(changed, {'examples/ripples.toml'})
            self.assertFalse(dev.gui_only(changed))

    def test_debounce_and_no_retry_storm(self):
        watcher = dev.Watcher({'x': 1})
        self.assertFalse(watcher.poll({'x': 2}, 1))
        self.assertFalse(watcher.poll({'x': 3}, 1.2))
        self.assertFalse(watcher.poll({'x': 3}, 1.4))
        self.assertTrue(watcher.poll({'x': 3}, 1.6))
        self.assertFalse(watcher.poll({'x': 3}, 5))
        self.assertFalse(watcher.poll({'x': 4}, 6))
        self.assertTrue(watcher.poll({'x': 4}, 7))

    def test_mode_environment(self):
        with mock.patch.dict(os.environ, {'WOOTING_STATE_DIR': 'danger', 'WOOTING_DEV_SIMULATION': '1'}):
            env = dev.environment(Path('/tmp/test'), True)
            self.assertNotIn('WOOTING_STATE_DIR', env)
            self.assertNotIn('WOOTING_DEV_SIMULATION', env)
            self.assertEqual(env['WOOTING_DEV_SUPERVISED'], '1')
            env = dev.environment(Path('/tmp/test'), False)
            self.assertEqual(env['WOOTING_STATE_DIR'], '/tmp/test')
            self.assertEqual(env['WOOTING_DEV_SIMULATION'], '1')

    def test_short_stable_socket_path(self):
        root = Path('/some/' + 'long/' * 100)
        self.assertEqual(dev.runtime_path(root), dev.runtime_path(root))
        self.assertLess(len(str(dev.runtime_path(root) / 'simulation/control.sock')), 104)
        self.assertNotEqual(dev.runtime_path(root), dev.runtime_path(root / 'other'))

    def test_private_directory_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            (base / 'real').mkdir()
            (base / 'link').symlink_to(base / 'real')
            with self.assertRaises(RuntimeError):
                dev.private_dir(base / 'link')
            self.assertEqual(dev.private_dir(base / 'real').stat().st_mode & 0o777, 0o700)

    def test_duplicate_supervisor_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with dev.acquire_lock(root):
                with self.assertRaisesRegex(RuntimeError, 'already running'):
                    dev.acquire_lock(root)
            with dev.acquire_lock(root):
                pass

    def test_private_files_reject_symlinks_and_fifo(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'real').touch()
            (root / 'link').symlink_to(root / 'real')
            with self.assertRaises(OSError):
                dev.private_file(root / 'link', append=True)
            os.mkfifo(root / 'fifo')
            with self.assertRaises(RuntimeError):
                dev.private_file(root / 'fifo', append=True)
            with dev.private_file(root / 'real', append=True) as stream:
                stream.write(b'0123456789')
            self.assertEqual(dev.tail_log(root / 'real', 4), '6789')
            self.assertEqual((root / 'real').stat().st_mode & 0o777, 0o600)

    def test_sdk_discovery_respects_overrides_and_user_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            apps = [root / 'system', root / 'user']
            frameworks = apps[1] / 'Wooting Signals.app/Contents/Frameworks'
            frameworks.mkdir(parents=True)
            analog = frameworks / 'libwooting_analog_sdk_dist.dylib'
            analog.touch()
            (frameworks / 'libwooting-rgb-sdk.dylib').touch()
            env = {'WOOTING_RGB_SDK_PATH': 'explicit/nonexistent/is/authoritative'}
            dev.discover_sdks(env, apps)
            self.assertEqual(env['WOOTING_RGB_SDK_PATH'], 'explicit/nonexistent/is/authoritative')
            self.assertEqual(env['WOOTING_ANALOG_SDK_PATH'], str(analog))

    def test_stop_does_not_signal_reaped_pid(self):
        child = mock.Mock()
        child.poll.return_value = 0
        with mock.patch.object(dev.os, 'killpg') as kill:
            dev.stop(child)
            kill.assert_not_called()

    def test_stop_handles_kill_race(self):
        child = mock.Mock()
        child.poll.return_value = None
        child.wait.side_effect = [subprocess.TimeoutExpired('fake', 1), 0]
        with mock.patch.object(dev.os, 'killpg', side_effect=[None, ProcessLookupError]):
            dev.stop(child, timeout=.01)
        self.assertEqual(child.wait.call_count, 2)


class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        binaries = self.root / 'target/debug'
        binaries.mkdir(parents=True)
        for name in dev.BINARIES:
            path = binaries / name
            path.write_text(FAKE)
            path.chmod(0o700)
        self.supervisor = FakeSupervisor(self.root, self.root / 'runtime')
        self.initial = {'src/main.rs': b'one', 'src/bin/wooting-gui.rs': b'one'}
        self.addCleanup(self.tmp.cleanup)
        self.addCleanup(self.supervisor.close)

    def test_failure_keeps_working_processes_and_generation(self):
        s = self.supervisor
        self.assertTrue(s.rebuild(self.initial))
        engine, gui, generation = s.engine, s.gui, s.engine_generation
        s.fail = True
        self.assertFalse(s.rebuild(dict(self.initial, **{'src/main.rs': b'broken'})))
        self.assertIs(s.engine, engine)
        self.assertIs(s.gui, gui)
        self.assertIsNone(engine.poll())
        self.assertIsNone(gui.poll())
        self.assertEqual(s.engine_generation, generation)
        self.assertEqual(s.applied, self.initial)

    def test_quitting_during_build_does_not_resurrect_gui(self):
        s = self.supervisor
        s.rebuild(self.initial)
        gui, engine = s.gui, s.engine
        def build():
            dev.stop(gui)
            return True
        with mock.patch.object(s, 'build', side_effect=build):
            self.assertFalse(s.rebuild(dict(self.initial, **{'src/main.rs': b'two'})))
        self.assertIs(s.gui, gui)
        self.assertIs(s.engine, engine)
        self.assertEqual(len(list(s.generations.iterdir())), 1)

    def test_gui_restart_leaves_engine_untouched(self):
        s = self.supervisor
        s.rebuild(self.initial)
        engine, gui, generation = s.engine, s.gui, s.engine_generation
        with mock.patch.object(s, 'control', wraps=s.control) as control:
            s.rebuild(dict(self.initial, **{'src/bin/wooting-gui.rs': b'two'}))
            control.assert_not_called()
        self.assertIs(s.engine, engine)
        self.assertIsNot(s.gui, gui)
        self.assertIsNotNone(gui.poll())
        self.assertTrue(generation.exists())
        self.assertNotEqual(s.gui_generation, generation)
        self.assertTrue((s.gui_generation / 'wooting-signals').is_file())

    def test_shared_change_restarts_both_paused_and_preserves_state(self):
        s = self.supervisor
        s.rebuild(self.initial)
        engine, gui = s.engine, s.gui
        (s.state / 'selection').write_text('ripples #112233')
        s.rebuild(dict(self.initial, **{'src/main.rs': b'two'}))
        self.assertIsNot(s.engine, engine)
        self.assertIsNot(s.gui, gui)
        self.assertIsNotNone(engine.poll())
        self.assertIsNotNone(gui.poll())
        self.assertIn('--paused', s.engine.args)
        self.assertEqual((s.state / 'selection').read_text(), 'ripples #112233')
        self.assertEqual(len(list(s.generations.iterdir())), 1)

    def test_failed_shared_then_gui_change_still_restarts_engine(self):
        s = self.supervisor
        s.rebuild(self.initial)
        engine = s.engine
        s.fail = True
        shared = dict(self.initial, **{'src/main.rs': b'two'})
        s.rebuild(shared)
        s.fail = False
        s.rebuild(dict(shared, **{'src/bin/wooting-gui.rs': b'two'}))
        self.assertIsNot(s.engine, engine)

    def test_build_output_replacement_does_not_change_generation(self):
        s = self.supervisor
        s.rebuild(self.initial)
        staged = s.engine_generation / 'wooting-signals'
        before = staged.read_bytes()
        (self.root / 'target/debug/wooting-signals').write_text('replacement')
        self.assertEqual(staged.read_bytes(), before)
        self.assertNotEqual(staged.stat().st_ino,
                            (self.root / 'target/debug/wooting-signals').stat().st_ino)

    def test_shutdown_reaps_owned_processes(self):
        s = self.supervisor
        s.rebuild(self.initial)
        engine, gui = s.engine, s.gui
        s.close()
        self.assertIsNotNone(engine.poll())
        self.assertIsNotNone(gui.poll())
        s.close()  # idempotent

    def test_partial_stage_failure_does_not_stop_running_processes(self):
        s = self.supervisor
        s.rebuild(self.initial)
        engine, gui = s.engine, s.gui
        (self.root / 'target/debug/wooting-gui').unlink()
        with self.assertRaises(FileNotFoundError):
            s.rebuild(dict(self.initial, **{'src/main.rs': b'two'}))
        self.assertIsNone(engine.poll())
        self.assertIsNone(gui.poll())
        self.assertEqual(len(list(s.generations.iterdir())), 1)

    def test_cleanup_attempts_every_owned_child(self):
        s = self.supervisor
        s.builder, s.gui, s.engine = mock.Mock(), mock.Mock(), mock.Mock()
        owned = [s.builder, s.gui, s.engine]
        with mock.patch.object(dev, 'stop', side_effect=[OSError('failure'), None, None]) as stop:
            with self.assertRaisesRegex(RuntimeError, 'cleanup failed'):
                s.close()
            self.assertEqual([call.args[0] for call in stop.call_args_list], owned)

    def test_control_timeout_cleans_up_group(self):
        s = self.supervisor
        s.engine_generation = self.root
        process = mock.Mock()
        process.communicate.side_effect = subprocess.TimeoutExpired('fake', 2)
        with mock.patch.object(dev.subprocess, 'Popen', return_value=process), \
             mock.patch.object(dev, 'stop') as stop:
            with self.assertRaises(subprocess.TimeoutExpired):
                s.control('status')
            stop.assert_called_once_with(process)
            process.stdout.close.assert_called_once()
            process.stderr.close.assert_called_once()

    def test_bounded_shutdown_kills_stubborn_child(self):
        ready = self.root / 'ready'
        process = subprocess.Popen([sys.executable, '-c',
            'import signal,time,pathlib; signal.signal(signal.SIGTERM,signal.SIG_IGN); '
            f'pathlib.Path({str(ready)!r}).touch(); time.sleep(60)'], start_new_session=True)
        self.addCleanup(dev.stop, process)
        deadline = time.monotonic() + 3
        while not ready.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(ready.exists())
        dev.stop(process, timeout=.2)
        self.assertIsNotNone(process.poll())


if __name__ == '__main__':
    unittest.main()
