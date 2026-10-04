import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import assemble_notices as assemble
import macos


class AssemblyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.root = self.home / 'repo'
        (self.root / 'packaging/licenses').mkdir(parents=True)
        for path, content in [('Cargo.lock', 'app lock'), ('LICENSE', 'MIT James Myers fixture'),
                              ('packaging/licenses/MPL-2.0.txt', 'MPL fixture'),
                              ('packaging/licenses/rust-overrides.json', '{}')]:
            (self.root / path).write_text(content)
        self.patch = patch.object(assemble, 'ROOT', self.root)
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.output = self.home / 'assembled'
        self.collections = {k: self.home / k for k in ('application-rust', 'analog-rust', 'runtime', 'native')}
        for p in self.collections.values():
            p.mkdir()
            (p / 'notice.txt').write_text('original fixture notice')
        sdk_lock = self.collections['native'] / 'notices/analog/Cargo.lock'
        sdk_lock.parent.mkdir(parents=True)
        sdk_lock.write_text('SDK lock')
        self.binaries = {name: self.home / name for name in ('wooting-signals', 'wooting-gui', 'wooting-service')}
        for p in self.binaries.values():
            p.write_bytes(b'fake executable; never run')
        common = dict(schema_version=1, status='complete', unresolved=[])
        policy = assemble.hashlib.sha256(b'{}').hexdigest()
        records = {
            'application-rust': dict(lock_sha256=assemble.digest(self.root / 'Cargo.lock'), policy_sha256=policy),
            'analog-rust': dict(lock_sha256=assemble.digest(sdk_lock), policy_sha256=policy),
            'native': dict(architecture='arm64', native_inputs={'libwooting_analog_sdk_dist.dylib': 'a'*64}),
            'runtime': dict(input_binaries={**{n: assemble.digest(p) for n,p in self.binaries.items()},
                                           'libwooting_analog_sdk_dist.dylib': 'a'*64}),
        }
        for name, record in records.items():
            directory = self.collections[name]
            record.update(common)
            record['files'] = {p.relative_to(directory).as_posix(): assemble.digest(p)
                               for p in directory.rglob('*') if p.is_file()}
            (directory / 'manifest.json').write_text(json.dumps(record))
        self.findings = dict(schema_version=1, unresolved=[{'id': 'upstream-rights-question'}], review_notes=[])

    def test_unresolved_findings_survive_and_release_gate_rejects(self):
        audit = assemble.assemble(self.collections, self.binaries, self.findings, self.output)
        self.assertEqual(audit['status'], 'incomplete')
        self.assertEqual(audit['unresolved'], self.findings['unresolved'])
        self.assertEqual((self.output / 'APPLICATION-LICENSE.txt').read_bytes(), (self.root / 'LICENSE').read_bytes())
        self.assertNotIn('audit.json', audit['files'])
        for name, digest in audit['files'].items():
            self.assertEqual(assemble.digest(self.output / name), digest)
        with self.assertRaisesRegex(RuntimeError, 'reviewed'):
            macos.validate_audit(self.output, 'arm64', {})

    def test_changed_notice_rejected_before_output(self):
        (self.collections['native'] / 'notice.txt').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'file hashes'):
            assemble.assemble(self.collections, self.binaries, self.findings, self.output)
        self.assertFalse(self.output.exists())

    def test_changed_lock_policy_and_binary_rejected(self):
        for path, error in [(self.root/'Cargo.lock', 'Cargo.lock'),
                            (self.root/'packaging/licenses/rust-overrides.json', 'policy'),
                            (self.binaries['wooting-gui'], 'binary changed')]:
            original = path.read_bytes()
            path.write_bytes(b'{}' if 'Cargo.lock' in str(path) else b'{"changed":true}')
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, error):
                assemble.assemble(self.collections, self.binaries, self.findings, self.output)
            path.write_bytes(original)
        self.assertFalse(self.output.exists())

    def test_existing_output_preserved(self):
        self.output.mkdir()
        (self.output / 'keep').write_text('user data')
        with self.assertRaises(FileExistsError):
            assemble.assemble(self.collections, self.binaries, self.findings, self.output)
        self.assertEqual((self.output / 'keep').read_text(), 'user data')

    def test_collection_symlink_and_incomplete_status_rejected(self):
        directory = self.collections['native']
        (directory/'link').symlink_to(directory/'notice.txt')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            assemble.verify_collection(directory)
        (directory/'link').unlink()
        path = directory/'manifest.json'
        data = json.loads(path.read_text())
        data['unresolved'] = ['missing source']
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            assemble.verify_collection(directory)

    def test_copy_failure_removes_partial_output(self):
        with patch.object(assemble.shutil, 'copytree', side_effect=OSError('fixture failure')):
            with self.assertRaisesRegex(OSError, 'fixture failure'):
                assemble.assemble(self.collections, self.binaries, self.findings, self.output)
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
