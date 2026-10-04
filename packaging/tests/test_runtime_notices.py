import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'packaging'))
import collect_runtime_notices as runtime


class RuntimeNoticeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='runtime notices fixture ')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.cache = self.home / 'cache'
        self.cache.mkdir()
        self.output = self.home / 'output'
        self.binaries = [self.home / 'gui', self.home / 'engine']
        self.analog = self.home / 'analog'
        for binary in self.binaries:
            binary.write_bytes(b'Mach-O fixture /rustc/' + runtime.PINS['1.94.1'][0].encode() + b'/library')
        self.analog.write_bytes(b'Mach-O fixture /rustc/' + runtime.PINS['1.91.1'][0].encode() + b'/library')

    def archive(self, version='1.94.1', extra=(), complete=True):
        path = self.cache / f'rustc-{version}-aarch64-apple-darwin.tar.xz'
        prefix = f'rustc-{version}-aarch64-apple-darwin/rustc/share/doc/rust/'
        members = [('COPYRIGHT-library.html', b'copyright'), ('licenses/MIT.txt', b'MIT fixture')] if complete else []
        with tarfile.open(path, 'w:xz') as archive:
            # Compiler payload must never be extracted or executed.
            payload = tarfile.TarInfo(f'rustc-{version}-aarch64-apple-darwin/rustc/bin/rustc')
            payload.size = 9
            archive.addfile(payload, io.BytesIO(b'not code!'))
            for name, content in [*members, *extra]:
                info = tarfile.TarInfo(prefix + name)
                if isinstance(content, tarfile.TarInfo):
                    info.type = content.type
                    info.linkname = content.linkname
                    info.size = content.size
                    archive.addfile(info)
                else:
                    info.size = len(content)
                    archive.addfile(info, io.BytesIO(content))
        return path

    def pins(self):
        return {version: (revision, runtime.digest(self.archive(version)))
                for version, (revision, _) in runtime.PINS.items()}

    def test_revision_checks_bytes_and_rejects_missing_wrong_or_mixed(self):
        binary = self.binaries[0]
        revision = runtime.PINS['1.94.1'][0]
        runtime.verify_revision(binary, revision)
        for content in (b'no evidence', b'/rustc/' + b'0' * 40,
                        binary.read_bytes() + b'/rustc/' + b'0' * 40):
            with self.subTest(content=content):
                binary.write_bytes(content)
                with self.assertRaisesRegex(ValueError, 'rustc revision'):
                    runtime.verify_revision(binary, revision)

    def test_revision_failure_precedes_download_or_output_creation(self):
        self.analog.write_bytes(b'unknown build')
        with patch.object(runtime.subprocess, 'run') as command:
            with self.assertRaisesRegex(ValueError, 'rustc revision'):
                runtime.collect(self.binaries, self.analog, self.output, self.cache)
        command.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_duplicate_input_basenames_rejected_before_collection(self):
        other = self.home / 'other' / self.binaries[0].name
        other.parent.mkdir()
        other.write_bytes(self.binaries[0].read_bytes() + b'different binary')
        pins = self.pins()
        with patch.object(runtime, 'PINS', pins):
            with self.assertRaisesRegex(ValueError, 'duplicate|basename|conflict'):
                runtime.collect([*self.binaries, other], self.analog, self.output, self.cache)
        self.assertFalse(self.output.exists())

    def test_cached_verified_archives_only_extract_notices_and_record_evidence(self):
        pins = self.pins()
        with patch.object(runtime, 'PINS', pins), patch.object(runtime.subprocess, 'run') as command:
            runtime.collect(self.binaries, self.analog, self.output, self.cache)
        command.assert_not_called()
        manifest = json.loads((self.output / 'manifest.json').read_text())
        self.assertEqual(manifest['status'], 'complete')
        self.assertEqual(manifest['unresolved'], [])
        self.assertEqual(manifest['input_binaries'],
                         {p.name: runtime.digest(p) for p in [*self.binaries, self.analog]})
        self.assertEqual(len(manifest['components']), 2)
        self.assertEqual(len(manifest['files']), 4)
        for name, checksum in manifest['files'].items():
            self.assertEqual(runtime.digest(self.output / name), checksum)
        self.assertFalse(list(self.output.rglob('rustc')))

    def test_checksum_mismatch_precedes_extract_and_cleans_output(self):
        pins = self.pins()
        pins['1.94.1'] = (pins['1.94.1'][0], '0' * 64)
        with patch.object(runtime, 'PINS', pins), patch.object(runtime, 'extract_notices') as extract:
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                runtime.collect(self.binaries, self.analog, self.output, self.cache)
        extract.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_later_archive_failure_removes_earlier_extracted_notices(self):
        pins = self.pins()
        pins['1.91.1'] = (pins['1.91.1'][0], '0' * 64)
        with patch.object(runtime, 'PINS', pins):
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                runtime.collect(self.binaries, self.analog, self.output, self.cache)
        self.assertFalse(self.output.exists())

    def test_existing_output_is_preserved(self):
        self.output.mkdir()
        sentinel = self.output / 'keep'
        sentinel.write_text('user data')
        with self.assertRaises(FileExistsError):
            runtime.collect(self.binaries, self.analog, self.output, self.cache)
        self.assertEqual(sentinel.read_text(), 'user data')

    def test_download_is_https_curl_only_and_failure_cleans_output(self):
        def fail(command, **kwargs):
            self.assertEqual(command[0], 'curl')
            self.assertIn('--proto', command)
            self.assertIn('--proto-redir', command)
            self.assertEqual(command[-1].split('/')[2], 'static.rust-lang.org')
            self.assertTrue(command[-1].startswith('https://'))
            raise subprocess.CalledProcessError(22, command)
        with patch.object(runtime.subprocess, 'run', side_effect=fail):
            with self.assertRaises(subprocess.CalledProcessError):
                runtime.collect(self.binaries, self.analog, self.output, self.cache)
        self.assertFalse(self.output.exists())

    def test_traversal_and_duplicate_notice_paths_rejected(self):
        for name in ('licenses/../../escaped', 'licenses/MIT.txt',
                     'licenses/./MIT.txt', 'licenses//MIT.txt'):
            with self.subTest(name=name):
                archive = self.archive(extra=[(name, b'malicious')])
                with self.assertRaisesRegex(ValueError, 'unsafe|duplicate'):
                    runtime.extract_notices(archive, self.home / 'extract', '1.94.1')
                self.assertFalse((self.home / 'escaped').exists())

    def test_links_and_special_notice_members_rejected(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind):
                info = tarfile.TarInfo()
                info.type = kind
                info.linkname = '/etc/passwd'
                archive = self.archive(extra=[('licenses/unsafe', info)])
                with self.assertRaisesRegex(ValueError, 'unsafe'):
                    runtime.extract_notices(archive, self.home / 'extract', '1.94.1')

    def test_oversized_notice_rejected(self):
        archive = self.archive(extra=[('licenses/huge', b'x' * (2 * 1024 * 1024 + 1))])
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            runtime.extract_notices(archive, self.home / 'extract', '1.94.1')

    def test_incomplete_archive_cleanup(self):
        pins = self.pins()
        archive = self.archive('1.91.1', complete=False)
        pins['1.91.1'] = (pins['1.91.1'][0], runtime.digest(archive))
        with patch.object(runtime, 'PINS', pins):
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                runtime.collect(self.binaries, self.analog, self.output, self.cache)
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
