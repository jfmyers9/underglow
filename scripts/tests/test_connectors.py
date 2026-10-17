"""Starter kit checks use only temporary files/processes/sockets; never an SDK/engine."""
import copy
import importlib.util
import io
import json
from pathlib import Path
import select
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

try:
    import tomllib
except ImportError:
    tomllib = None

ROOT = Path(__file__).resolve().parents[2]
SCAFFOLD = ROOT / 'scripts/scaffold-connector.py'
CHECK = ROOT / 'scripts/check-connector.py'
FIXTURES = ROOT / 'connectors/examples/fixtures'
SPEC = importlib.util.spec_from_file_location('connector_check', CHECK)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)
BASE = json.loads((FIXTURES / 'valid.ndjson').read_text().splitlines()[0])
NODE = shutil.which('node')


class FakeEngine:
    def __init__(self, path, response='good'):
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(str(path))
        self.server.listen()
        self.server.settimeout(.1)
        self.response = response
        self.messages = []
        self.errors = []
        self.stopped = threading.Event()
        self.worker = threading.Thread(target=self.run, daemon=True)

    def __enter__(self):
        self.worker.start()
        return self

    def run(self):
        sequences = {}
        while not self.stopped.is_set():
            try:
                conn, _ = self.server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            try:
                with conn:
                    conn.settimeout(1)
                    data = bytearray()
                    while b'\n' not in data and len(data) <= 4096:
                        chunk = conn.recv(4097 - len(data))
                        if not chunk:
                            break
                        data.extend(chunk)
                    message = checker.validate_message(bytes(data).rstrip(b'\n'), sequences)
                    self.messages.append(message)
                    if self.response == 'silent':
                        conn.recv(1)  # Wait for the producer's hard deadline to close.
                    elif self.response == 'oversized':
                        conn.sendall(b'x' * 4097)
                    else:
                        ack = {'schema_version': 1, 'ok': True,
                               'outcome': 'applied' if self.response == 'good' else 'ignored_stale'}
                        conn.sendall(json.dumps(ack).encode() + b'\n')
            except Exception as error:
                self.errors.append(error)

    def __exit__(self, *_):
        self.stopped.set()
        self.worker.join(2)
        self.server.close()
        if self.worker.is_alive():
            raise AssertionError('fake socket server did not stop')


class ValidationTests(unittest.TestCase):
    def test_committed_fixtures(self):
        self.assertEqual(checker.check_stream(io.BytesIO((FIXTURES / 'valid.ndjson').read_bytes())), 6)
        for path in FIXTURES.glob('invalid-*.ndjson'):
            with self.subTest(path=path.name), self.assertRaises(ValueError):
                checker.check_stream(io.BytesIO(path.read_bytes()))

    def test_strict_envelope_and_payload(self):
        cases = [
            ('schema_version', True), ('schema_version', 2), ('source', ''),
            ('source', 'space name'), ('instance', 'é'), ('instance', 'a' * 129),
            ('sequence', -1), ('sequence', True), ('sequence', 1.0),
            ('sequence', 2**64), ('payload', []),
            ('payload', {'kind': 'state', 'state': 'idle', 'ttl_ms': 0}),
            ('payload', {'kind': 'state', 'state': 'idle', 'ttl_ms': True}),
            ('payload', {'kind': 'state', 'state': 'idle', 'ttl_ms': 1.5}),
            ('payload', {'kind': 'event', 'event': 'bad label', 'ttl_ms': 1}),
            ('payload', {'kind': 'command'}), ('payload', {'kind': []}),
        ]
        for field, value in cases:
            message = copy.deepcopy(BASE)
            message[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                checker.validate_message(json.dumps(message).encode(), {})
        for raw in (b'{}', b'[]', b'{', b'\xff', b' ' * 4097,
                    json.dumps(BASE).encode('utf-16'),
                    b'{"schema_version":1,"schema_version":1}', b'[' * 3000):
            with self.subTest(raw=raw[:40]), self.assertRaises(ValueError):
                checker.validate_message(raw, {})

    def test_sequence_and_numeric_boundaries(self):
        sequences = {}
        message = copy.deepcopy(BASE)
        message['sequence'] = 0
        message['payload']['ttl_ms'] = 1
        checker.validate_message(json.dumps(message).encode(), sequences)
        message['sequence'] = 2**64 - 1
        message['payload']['ttl_ms'] = 300000
        checker.validate_message(json.dumps(message).encode(), sequences)
        with self.assertRaises(ValueError):
            checker.validate_message(json.dumps(message).encode(), sequences)
        message['instance'] = 'another-instance'
        checker.validate_message(json.dumps(message).encode(), sequences)

    def test_framing_and_bounded_fixtures(self):
        line = json.dumps(BASE).encode()
        with self.assertRaises(ValueError):
            checker.check_stream(io.BytesIO(line))
        with self.assertRaises(ValueError):
            checker.check_stream(io.BytesIO(b' ' * 4097 + b'\n'))
        sequences = {('source', str(i)): 1 for i in range(256)}
        with self.assertRaises(ValueError):
            checker.validate_message(line, sequences)
        result = subprocess.run([sys.executable, str(CHECK)], input=b'', capture_output=True)
        self.assertNotEqual(result.returncode, 0)


class StarterTests(unittest.TestCase):
    def setUp(self):
        # Short Unix socket names are portable to macOS's small sockaddr_un.
        self.temp = tempfile.TemporaryDirectory(prefix='ug-kit-', dir='/tmp')
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)

    def generate(self, language, name='focus', capabilities=False):
        destination = self.directory / language
        command = [sys.executable, str(SCAFFOLD), '--language', language,
                   '--name', name, '--output', str(destination)]
        if capabilities:
            command.append('--capabilities')
        result = subprocess.run(command, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return destination

    def command(self, language, destination):
        if language == 'typescript':
            if NODE is None:
                self.skipTest('Node 22.19+ required for TypeScript example')
            return [NODE, str(destination / 'connector.ts')]
        return [sys.executable, str(destination / 'connector.py')]

    def test_scaffold_validates_names_and_refuses_existing_paths(self):
        for name in ('', 'bad name', "a';evil", 'é', 'a' * 129):
            destination = self.directory / 'bad'
            result = subprocess.run([sys.executable, str(SCAFFOLD), '--language', 'python',
                                     '--name', name, '--output', str(destination)], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(destination.exists())
        target = self.directory / 'existing'
        target.mkdir()
        for occupied in (False, True):
            if occupied:
                (target / 'keep').write_text('untouched')
            result = subprocess.run([sys.executable, str(SCAFFOLD), '--language', 'python',
                                     '--name', 'focus', '--output', str(target)], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
        self.assertEqual((target / 'keep').read_text(), 'untouched')
        link = self.directory / 'link'
        link.symlink_to(target, target_is_directory=True)
        result = subprocess.run([sys.executable, str(SCAFFOLD), '--language', 'python',
                                 '--name', 'focus', '--output', str(link)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)

    def test_generated_examples_and_fixtures(self):
        identities = set()
        for language in ('python', 'typescript'):
            with self.subTest(language=language):
                destination = self.generate(language, 'custom:focus/task-1')
                self.assertFalse((destination / 'capabilities.toml').exists())
                result = subprocess.run([sys.executable, str(CHECK), str(destination / 'fixture.ndjson')],
                                        capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                for _ in range(2):
                    output = subprocess.run(self.command(language, destination) +
                                            ['--stdout', '--duration-seconds', '0'],
                                            capture_output=True, timeout=3)
                    self.assertEqual(output.returncode, 0, output.stderr)
                    self.assertEqual(checker.check_stream(io.BytesIO(output.stdout)), 3)
                    messages = [json.loads(line) for line in output.stdout.splitlines()]
                    self.assertEqual([m['payload']['kind'] for m in messages], ['state', 'event', 'remove'])
                    self.assertEqual({m['source'] for m in messages}, {'custom:focus/task-1'})
                    identity = messages[0]['instance']
                    self.assertNotIn(identity, identities)
                    identities.add(identity)

    @unittest.skipUnless(tomllib, 'Python 3.11+ required for TOML test parsing')
    def test_optional_capability_manifest_matches_both_generated_producers(self):
        for language in ('python', 'typescript'):
            with self.subTest(language=language):
                destination = self.generate(language, 'custom:focus/task-1', capabilities=True)
                manifest = tomllib.loads((destination / 'capabilities.toml').read_text())
                self.assertEqual(manifest['schema_version'], 1)
                self.assertEqual(manifest['source'], 'custom:focus/task-1')
                self.assertEqual(set(manifest), {'schema_version', 'source', 'display_name',
                                                'instances', 'lifecycle', 'states', 'events'})
                self.assertEqual(manifest['instances'], {'semantics': 'session', 'selectors': ['any', 'exact']})
                result = subprocess.run(self.command(language, destination) + ['--stdout', '--duration-seconds', '0'],
                                        capture_output=True, timeout=3)
                self.assertEqual(result.returncode, 0, result.stderr)
                messages = [json.loads(line) for line in result.stdout.splitlines()]
                for message in messages:
                    payload = message['payload']
                    if payload['kind'] == 'remove':
                        continue
                    kind = payload['kind']
                    self.assertEqual(payload[kind], manifest[kind + 's'][0]['id'])
                    ttl = manifest['lifecycle'][kind + '_ttl']
                    self.assertLessEqual(ttl['min_ms'], payload['ttl_ms'])
                    self.assertGreaterEqual(ttl['max_ms'], payload['ttl_ms'])
                self.assertLess(manifest['lifecycle']['renewal_interval_ms'],
                                manifest['lifecycle']['state_ttl']['min_ms'])

    def test_generated_examples_publish_to_real_temporary_sockets(self):
        for language in ('python', 'typescript'):
            with self.subTest(language=language):
                destination = self.generate(language)
                path = self.directory / f'{language}.sock'
                with FakeEngine(path) as engine:
                    result = subprocess.run(self.command(language, destination) +
                                            ['--socket', str(path), '--duration-seconds', '.02'],
                                            capture_output=True, timeout=3)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertFalse(engine.errors)
                # A fractional timer can wake just before the deadline and
                # legitimately renew state before completion. Check lifecycle
                # and contiguous delivery, not an exact timer-dependent count.
                self.assertGreaterEqual(len(engine.messages), 3)
                self.assertEqual([m['sequence'] for m in engine.messages],
                                 list(range(1, len(engine.messages) + 1)))
                self.assertEqual([m['payload']['kind'] for m in engine.messages[-2:]],
                                 ['event', 'remove'])
                for message in engine.messages[:-2]:
                    self.assertEqual(message['payload'],
                                     {'kind': 'state', 'state': 'focusing', 'ttl_ms': 15000})

    def test_sigterm_removes_without_completion(self):
        for language in ('python', 'typescript'):
            with self.subTest(language=language):
                destination = self.generate(language)
                process = subprocess.Popen(self.command(language, destination) + ['--stdout'],
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                try:
                    self.assertTrue(select.select([process.stdout], [], [], 3)[0])
                    first = process.stdout.readline()
                    self.assertEqual(json.loads(first)['payload']['kind'], 'state')
                    process.send_signal(signal.SIGTERM)
                    output, errors = process.communicate(timeout=3)
                    self.assertEqual(process.returncode, 0, errors)
                    self.assertEqual(checker.check_stream(io.BytesIO(first + output)), 2)
                    self.assertEqual(json.loads(output)['payload']['kind'], 'remove')
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.communicate()

    def test_heartbeats_renew_state_not_events(self):
        for language in ('python', 'typescript'):
            with self.subTest(language=language):
                destination = self.generate(language)
                result = subprocess.run(self.command(language, destination) +
                                        ['--stdout', '--duration-seconds', '5.05'],
                                        capture_output=True, timeout=8)
                self.assertEqual(result.returncode, 0, result.stderr)
                messages = [json.loads(line) for line in result.stdout.splitlines()]
                self.assertEqual(checker.check_stream(io.BytesIO(result.stdout)), len(messages))
                self.assertGreaterEqual(len(messages), 4)
                self.assertEqual([m['payload']['kind'] for m in messages[-2:]], ['event', 'remove'])
                # Timer granularity can permit another renewal near completion.
                # Every preceding message must renew the same state, never events.
                for message in messages[:-2]:
                    self.assertEqual(message['payload']['kind'], 'state')
                    self.assertEqual(message['payload'], messages[0]['payload'])

    def test_socket_failure_deadlines_and_bounded_ack(self):
        for language in ('python', 'typescript'):
            destination = self.generate(language)
            command = self.command(language, destination)
            for response in ('silent', 'oversized', 'stale'):
                with self.subTest(language=language, response=response):
                    path = self.directory / f'{language}-{response}.sock'
                    start = time.monotonic()
                    with FakeEngine(path, response) as engine:
                        result = subprocess.run(command + ['--socket', str(path), '--duration-seconds', '0'],
                                                capture_output=True, timeout=3)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertLess(time.monotonic() - start, 2.5)
                    self.assertLessEqual(len(engine.messages), 2)  # State + best-effort remove, never retries.
            result = subprocess.run(command + ['--socket', str(self.directory / 'missing.sock')],
                                    capture_output=True, timeout=3)
            self.assertNotEqual(result.returncode, 0)


if __name__ == '__main__':
    unittest.main()
