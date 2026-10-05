#!/usr/bin/env python3
"""Private, supervised development loop. Python standard library only."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time

BINARIES = ('underglow', 'underglow-gui', 'underglow-service')


def snapshot(root):
    paths = [root / name for name in ('Cargo.toml', 'Cargo.lock', 'build.rs',
                                     'rust-toolchain', 'rust-toolchain.toml')]
    # Preset TOML is compiled into the engine via include_str! too.
    for directory in ('src', '.cargo', 'examples'):
        paths.extend(p for p in (root / directory).rglob('*') if p.is_file())
    paths.append(root / 'external/wooting-rgb-sdk/src/wooting-rgb-sdk.h')
    result = {}
    for path in paths:
        try:
            result[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).digest()
        except FileNotFoundError:
            pass
    return result


def changes(before, after):
    return {p for p in before.keys() | after.keys() if before.get(p) != after.get(p)}


def gui_only(paths):
    return bool(paths) and all(p == 'src/bin/underglow-gui.rs' or p.startswith('src/bin/gui/')
                               for p in paths)


def private_dir(path):
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid():
        raise RuntimeError(f'Not a private owned directory: {path}')
    path.chmod(0o700)
    return path


def private_file(path, append=False):
    flags = os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK
    if append:
        flags |= os.O_APPEND
    fd = os.open(path, flags, 0o600)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise RuntimeError(f'Not an owned regular file: {path}')
        os.fchmod(fd, 0o600)
        return os.fdopen(fd, 'ab' if append else 'r+b')
    except BaseException:
        os.close(fd)
        raise


def acquire_lock(runtime):
    lock = private_file(runtime / 'supervisor.lock')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BaseException as error:
        lock.close()
        if isinstance(error, BlockingIOError):
            raise RuntimeError('make dev is already running for this checkout') from None
        raise
    return lock


def runtime_path(root):
    digest = hashlib.sha256(os.fsencode(root.resolve())).hexdigest()[:12]
    # Deliberately avoid macOS's long per-user TMPDIR: Unix sockets have a short limit.
    return Path('/tmp') / f'wsdev-{os.getuid()}-{digest}'


def environment(state, hardware):
    env = dict(os.environ, WOOTING_DEV_SUPERVISED='1')
    if hardware:
        env.pop('WOOTING_STATE_DIR', None)
        env.pop('WOOTING_DEV_SIMULATION', None)
    else:
        env['WOOTING_STATE_DIR'] = str(state)
        env['WOOTING_DEV_SIMULATION'] = '1'
    return env


def discover_sdks(env, app_roots=None):
    """Read-only convenience for hardware mode; explicit overrides are authoritative."""
    if app_roots is None:
        if sys.platform != 'darwin':
            return
        app_roots = [Path('/Applications'), Path.home() / 'Applications']
    for key, name in (
        ('WOOTING_RGB_SDK_PATH', 'libwooting-rgb-sdk.dylib'),
        ('WOOTING_ANALOG_SDK_PATH', 'libwooting_analog_sdk_dist.dylib'),
    ):
        if key not in env:
            # Prefer either Underglow install over legacy bundles, per library.
            candidates = (root / app / 'Contents/Frameworks' / name
                          for app in ('Underglow.app', 'Wooting Signals.app')
                          for root in app_roots)
            for candidate in candidates:
                if candidate.is_file():
                    env[key] = str(candidate)
                    break
        print(f'{key}: {env.get(key, "normal SDK discovery")}', flush=True)


def tail_log(path, size=2000):
    with private_file(path) as stream:
        stream.seek(0, os.SEEK_END)
        stream.seek(max(0, stream.tell() - size))
        return stream.read(size).decode(errors='replace')


def stop(process, timeout=3):
    if process is None or process.poll() is not None:
        return
    # Each child owns a new session; terminate its helpers as well, never arbitrary PIDs.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=timeout)


class Supervisor:
    def __init__(self, root, runtime, hardware=False):
        self.root = root
        self.runtime = private_dir(runtime)
        self.state = private_dir(runtime / ('hardware' if hardware else 'simulation'))
        self.logs = private_dir(runtime / 'logs')
        self.generations = private_dir(runtime / 'generations')
        self.env = environment(self.state, hardware)
        if hardware:
            discover_sdks(self.env)
        self.hardware = hardware
        self.engine = self.gui = self.builder = None
        self.engine_generation = self.gui_generation = None
        self.applied = {}

    def spawn(self, args, log=None):
        if log is None:
            return subprocess.Popen(args, cwd=self.root, env=self.env, start_new_session=True)
        with private_file(self.logs / log, append=True) as output:
            return subprocess.Popen(args, cwd=self.root, env=self.env, start_new_session=True,
                                    stdout=output, stderr=subprocess.STDOUT)

    def build(self):
        self.builder = self.spawn(['cargo', 'build', '--features', 'gui', '--bins',
                                   '--target-dir', str(self.root / 'target')])
        try:
            return self.builder.wait() == 0
        finally:
            if self.builder.poll() is not None:
                self.builder = None

    def stage(self):
        generation = Path(tempfile.mkdtemp(prefix='build-', dir=self.generations))
        try:
            for name in BINARIES:
                shutil.copy2(self.root / 'target/debug' / name, generation / name)
        except BaseException:
            shutil.rmtree(generation)
            raise
        return generation

    def control(self, action):
        args = [str(self.engine_generation / 'underglow'), 'control',
                '--state-dir', str(self.state), action]
        process = subprocess.Popen(args, cwd=self.root, env=self.env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        try:
            output, _ = process.communicate(timeout=2)
        except BaseException:
            stop(process)
            raise
        finally:
            process.stdout.close()
            process.stderr.close()
        if process.returncode:
            return False
        try:
            return json.loads(output).get('ok') is True
        except (ValueError, AttributeError):
            return False

    def ready(self):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if self.engine.poll() is not None:
                break
            try:
                if self.control('status'):
                    return
            except subprocess.TimeoutExpired:
                pass
            time.sleep(.1)
        detail = tail_log(self.logs / 'engine.log')
        raise RuntimeError('Development engine did not start. If another engine owns the lease, '
                           'stop it explicitly in the installed app, then retry. '
                           'No installed engine or login service was changed.\n' + detail)

    def rebuild(self, current):
        changed = changes(self.applied, current)
        only_gui = self.engine is not None and gui_only(changed)
        print('Building (GUI only)' if only_gui else 'Building (engine + GUI)', flush=True)
        if not self.build():
            print('Build failed; last working processes left running. Save to retry.', flush=True)
            return False
        if self.gui is not None and self.gui.poll() is not None:
            # Quitting during compilation must not resurrect the window afterward.
            return False
        if self.engine is not None and self.engine.poll() is not None:
            raise RuntimeError('Owned engine exited during build; inspect engine.log')
        generation = self.stage()  # Complete copying before touching the running generation.
        stop(self.gui)
        self.gui = None
        if not only_gui:
            stop(self.engine)
            self.engine = None
            self.engine_generation = generation
            self.engine = self.spawn([str(generation / 'underglow'), 'engine',
                                      '--state-dir', str(self.state), '--paused'], 'engine.log')
            self.ready()
        self.gui_generation = generation
        self.gui = self.spawn([str(generation / 'underglow-gui'), '--state-dir', str(self.state)],
                              'gui.log')
        self.applied = current
        self.prune()
        status = 'engine unchanged' if only_gui else 'paused'
        print(f'Ready ({status}). Save source to rebuild; Ctrl-C stops owned processes.', flush=True)
        return True

    def prune(self):
        for path in self.generations.iterdir():
            if path not in (self.engine_generation, self.gui_generation):
                shutil.rmtree(path)

    def close(self):
        errors = []
        for name in ('builder', 'gui', 'engine'):
            try:
                stop(getattr(self, name))
            except (OSError, subprocess.SubprocessError) as error:
                errors.append(error)
            finally:
                setattr(self, name, None)
        if errors:
            raise RuntimeError(f'Child cleanup failed: {errors}')


class Watcher:
    def __init__(self, initial, debounce=.3):
        self.observed = initial
        self.attempted = initial
        self.changed_at = None
        self.debounce = debounce

    def poll(self, current, now):
        if current != self.observed:
            self.observed = current
            self.changed_at = now
        if current != self.attempted and self.changed_at is not None and now - self.changed_at >= self.debounce:
            self.attempted = current
            return True
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--hardware', action='store_true', help='use real hardware leases, never isolated SDK access')
    args = parser.parse_args()
    os.umask(0o077)
    root = Path(__file__).resolve().parent.parent
    runtime = private_dir(runtime_path(root))
    with acquire_lock(runtime) as lock:
        supervisor = Supervisor(root, runtime, args.hardware)
        print(f'Dev {"hardware" if args.hardware else "simulation"}: {supervisor.state}\nLogs: {supervisor.logs}', flush=True)
        def interrupt(_signum, _frame):
            raise KeyboardInterrupt
        signal.signal(signal.SIGTERM, interrupt)
        try:
            current = snapshot(root)
            watcher = Watcher(current)
            supervisor.rebuild(current)
            while True:
                time.sleep(.15)
                if supervisor.engine is not None and supervisor.engine.poll() is not None:
                    raise RuntimeError('Owned engine exited; inspect engine.log and restart make dev')
                if supervisor.gui is not None and supervisor.gui.poll() is not None:
                    print('GUI exited; stopping development session.', flush=True)
                    break
                current = snapshot(root)
                if watcher.poll(current, time.monotonic()):
                    supervisor.rebuild(current)
        finally:
            supervisor.close()
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(0)
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f'dev: {error}', file=sys.stderr)
        sys.exit(1)
