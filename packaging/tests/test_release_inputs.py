import hashlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('release_inputs', ROOT / 'packaging/release_inputs.py')
inputs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inputs)


class ReleaseInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.archive = self.root / 'inputs.tar.gz'
        self.output = self.root / 'staged'

    def archive_with(self, extras=(), omit=None):
        with tarfile.open(self.archive, 'w:gz') as archive:
            for name in inputs.REQUIRED:
                if name != omit:
                    item = tarfile.TarInfo(name)
                    item.size = 7
                    archive.addfile(item, io.BytesIO(b'fixture'))
            for item, contents in extras:
                archive.addfile(item, io.BytesIO(contents) if contents is not None else None)

    def test_extract_required_and_optional_tree(self):
        extra = tarfile.TarInfo('deps/libfixture.dylib')
        extra.size = 3
        extra.mode = 0o4777
        self.archive_with([(extra, b'abc')])
        inputs.extract(self.archive, self.output)
        self.assertEqual((self.output / extra.name).read_bytes(), b'abc')
        self.assertEqual((self.output / extra.name).stat().st_mode & 0o7777, 0o644)
        for name in inputs.REQUIRED:
            self.assertTrue((self.output / name).is_file())

    def test_reject_unsafe_paths_and_cleanup(self):
        for name in ('../escape', '/escape', 'deps/../../escape', 'deps//file',
                     './file', 'deps/./file', 'deps\\escape', 'deps/\nfile'):
            with self.subTest(name=name):
                self.archive_with([(tarfile.TarInfo(name), b'')])
                with self.assertRaisesRegex(ValueError, 'unsafe'):
                    inputs.extract(self.archive, self.output)
                self.assertFalse(self.output.exists())
        self.assertFalse((self.root / 'escape').exists())

    def test_reject_links_and_special_files(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE,
                     tarfile.CHRTYPE, tarfile.BLKTYPE, tarfile.GNUTYPE_SPARSE,
                     tarfile.CONTTYPE):
            with self.subTest(kind=kind):
                item = tarfile.TarInfo('deps/unsafe')
                item.type = kind
                item.linkname = '../../escape'
                self.archive_with([(item, None)])
                with self.assertRaises(ValueError):
                    inputs.extract(self.archive, self.output)
                self.assertFalse(self.output.exists())

    def test_reject_duplicate_and_case_alias(self):
        for name in ('rgb-sdk.dylib', 'RGB-SDK.DYLIB'):
            self.archive_with([(tarfile.TarInfo(name), b'')])
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                inputs.extract(self.archive, self.output)
            self.assertFalse(self.output.exists())

    def test_missing_required_inputs(self):
        for name in inputs.REQUIRED:
            self.archive_with(omit=name)
            with self.assertRaisesRegex(ValueError, 'required input'):
                inputs.extract(self.archive, self.output)
            self.assertFalse(self.output.exists())

    def test_limits(self):
        self.archive_with()
        for name, limit in [('MAX_DOWNLOAD', 1), ('MAX_EXPANDED', 100),
                            ('MAX_FILE', 1), ('MAX_MEMBERS', 2)]:
            with self.subTest(limit=name), patch.object(inputs, name, limit):
                with self.assertRaisesRegex(ValueError, 'limit|too many'):
                    inputs.extract(self.archive, self.output)
                self.assertFalse(self.output.exists())

    def test_existing_destination_not_removed_or_overwritten(self):
        self.archive_with()
        self.output.mkdir()
        marker = self.output / 'keep'
        marker.write_text('keep')
        with self.assertRaises(FileExistsError):
            inputs.extract(self.archive, self.output)
        self.assertEqual(marker.read_text(), 'keep')

    def test_file_directory_collision_cleans_up(self):
        self.archive_with([(tarfile.TarInfo('rgb-sdk.dylib/child'), b'')])
        with self.assertRaises(OSError):
            inputs.extract(self.archive, self.output)
        self.assertFalse(self.output.exists())

    def test_invalid_gzip_cleans_up(self):
        self.archive.write_bytes(b'not gzip')
        with self.assertRaises(OSError):
            inputs.extract(self.archive, self.output)
        self.assertFalse(self.output.exists())

    def test_validate_source(self):
        digest = 'a' * 64
        inputs.validate_source('https://example.invalid/inputs.tar.gz?version=1', digest)
        for url in ('http://example.invalid/file', 'file:///file', 'https:///file',
                    'https://user:pass@example.invalid/file', 'https://example.invalid/#frag',
                    'https://example.invalid/\nfile'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                inputs.validate_source(url, digest)
        for sha in ('', 'a' * 63, 'a' * 65, 'z' * 64, 'a' * 64 + '\n'):
            with self.subTest(sha=sha), self.assertRaises(ValueError):
                inputs.validate_source('https://example.invalid/file', sha)

    def test_mock_download_checks_digest_and_security_flags(self):
        self.archive.write_bytes(b'fixture')
        digest = hashlib.sha256(b'fixture').hexdigest()
        with patch.object(inputs.subprocess, 'run') as run:
            inputs.download('https://example.invalid/inputs.tar.gz', digest.upper(), self.archive)
        args = run.call_args.args[0]
        self.assertEqual(args[:2], ['curl', '-q'])
        for flag in ('--proto', '--proto-redir'):
            self.assertEqual(args[args.index(flag) + 1], '=https')
        self.assertEqual(args[args.index('--max-filesize') + 1], str(inputs.MAX_DOWNLOAD))
        self.assertEqual(args[args.index('--max-time') + 1], str(inputs.TIMEOUT))
        self.assertEqual(run.call_args.kwargs, {'check': True, 'timeout': inputs.TIMEOUT + 5})
        with patch.object(inputs.subprocess, 'run'):
            with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
                inputs.download('https://example.invalid/file', '0' * 64, self.archive)
            with patch.object(inputs, 'MAX_DOWNLOAD', 1), self.assertRaisesRegex(ValueError, 'limit'):
                inputs.download('https://example.invalid/file', digest, self.archive)

    def test_download_failure_is_not_ignored(self):
        for error in (subprocess.CalledProcessError(22, 'curl'),
                      subprocess.TimeoutExpired('curl', inputs.TIMEOUT)):
            with patch.object(inputs.subprocess, 'run', side_effect=error):
                with self.assertRaises(type(error)):
                    inputs.download('https://example.invalid/file', '0' * 64, self.archive)


if __name__ == '__main__':
    unittest.main()
