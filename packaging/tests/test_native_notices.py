import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import collect_native_notices as native


class NativeNoticesTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.cache = self.root / 'cache'
        self.cache.mkdir()
        self.source = self.cache / 'source.tar.gz'
        with tarfile.open(self.source, 'w:gz') as tar:
            info = tarfile.TarInfo('root/LICENSE')
            info.size = len(b'original license')
            tar.addfile(info, io.BytesIO(b'original license'))
        self.binary = self.root / 'library.dylib'
        self.binary.write_bytes(b'synthetic; never loaded')
        self.component = {
            'id': 'fixture', 'version': '1', 'bundle_name': 'library.dylib',
            'license_selection': 'MIT', 'input_sha256': native.digest(self.binary.read_bytes()),
            'source': {'file': self.source.name, 'root': 'root',
                       'url': 'https://invalid.example/source',
                       'sha256': native.digest(self.source.read_bytes())},
            'notice_members': ['LICENSE'],
        }
        self.manifest = {'schema_version': 1, 'architecture': 'arm64',
                         'components': [self.component]}
        self.provenance = {'native_inputs': {'Contents/Frameworks/library.dylib':
                           {'sha256': self.component['input_sha256']}}}
        self.output = self.root / 'result'

    def collect(self):
        with patch.object(native.urllib.request, 'build_opener', side_effect=AssertionError('network forbidden')):
            return native.collect(self.manifest, {'fixture': self.binary}, self.provenance,
                                  self.output, self.cache)

    def test_complete_inventory_preserves_bytes_and_hashes(self):
        self.collect()
        self.assertEqual((self.output / 'notices/fixture/LICENSE').read_bytes(), b'original license')
        result = json.loads((self.output / 'manifest.json').read_text())
        self.assertEqual(result['status'], 'complete')
        self.assertNotIn('manifest.json', result['files'])
        for name, expected in result['files'].items():
            self.assertEqual(native.digest((self.output / name).read_bytes()), expected)
        self.assertEqual(result['native_inputs']['library.dylib'], self.component['input_sha256'])

    def test_changed_input_rejected_before_output(self):
        self.binary.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
            self.collect()
        self.assertFalse(self.output.exists())

    def test_provenance_mismatch_rejected(self):
        self.provenance['native_inputs']['Contents/Frameworks/library.dylib']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'provenance disagrees'):
            self.collect()

    def test_changed_cache_rejected(self):
        self.source.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'SHA256 mismatch'):
            self.collect()
        self.assertFalse((self.output / 'manifest.json').exists())
        self.assertFalse(self.output.exists())

    def test_stale_output_rejected(self):
        self.output.mkdir()
        (self.output / 'keep').write_text('existing')
        with self.assertRaisesRegex(ValueError, 'must not exist'):
            self.collect()
        self.assertEqual((self.output / 'keep').read_text(), 'existing')

    def test_archive_cache_read_bounded(self):
        with patch.object(native, 'MAX_ARCHIVE_BYTES', 1):
            with self.assertRaisesRegex(ValueError, 'size limit'):
                self.collect()
        self.assertFalse(self.output.exists())

    def test_selected_member_size_bounded(self):
        with patch.object(native, 'MAX_MEMBER_BYTES', 1):
            with self.assertRaisesRegex(ValueError, 'size limit'):
                native.member_bytes(self.source, 'root/LICENSE')

    def test_download_size_bounded_and_timeout(self):
        self.source.unlink()
        response = MagicMock()
        response.__enter__.return_value = response
        response.geturl.return_value = 'https://example.test/source'
        response.read.return_value = b'12345'
        opener = MagicMock()
        opener.open.return_value = response
        with patch.object(native.urllib.request, 'build_opener', return_value=opener), \
                patch.object(native, 'MAX_ARCHIVE_BYTES', 4):
            with self.assertRaisesRegex(ValueError, 'size limit'):
                native.fetch(self.component['source'], self.cache)
        response.read.assert_called_once_with(5)
        opener.open.assert_called_once_with(self.component['source']['url'], timeout=90)
        self.assertFalse(self.source.exists())

    def test_non_https_redirect_rejected_before_request(self):
        handler = native.HTTPSRedirectHandler()
        with self.assertRaisesRegex(ValueError, 'redirects must use HTTPS'):
            handler.redirect_request(None, None, 302, '', {}, 'http://example.test/source')

    def test_non_https_final_response_rejected_without_read(self):
        self.source.unlink()
        response = MagicMock()
        response.__enter__.return_value = response
        response.geturl.return_value = 'http://example.test/source'
        opener = MagicMock()
        opener.open.return_value = response
        with patch.object(native.urllib.request, 'build_opener', return_value=opener):
            with self.assertRaisesRegex(ValueError, 'response must use HTTPS'):
                native.fetch(self.component['source'], self.cache)
        response.read.assert_not_called()
        self.assertFalse(self.source.exists())

    def test_unsafe_member_rejected(self):
        for name in ('../outside', '/outside', 'dir/../../outside', 'dir\\outside'):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'unsafe path'):
                native.member_bytes(self.source, name)

    def test_symlink_member_rejected(self):
        with tarfile.open(self.source, 'w:gz') as tar:
            info = tarfile.TarInfo('root/LICENSE')
            info.type = tarfile.SYMTYPE
            info.linkname = '/etc/passwd'
            tar.addfile(info)
        with self.assertRaisesRegex(ValueError, 'regular archive member'):
            native.member_bytes(self.source, 'root/LICENSE')

    def test_duplicate_member_rejected(self):
        with tarfile.open(self.source, 'w:gz') as tar:
            for _ in range(2):
                info = tarfile.TarInfo('root/LICENSE')
                tar.addfile(info, io.BytesIO(b''))
        with self.assertRaisesRegex(ValueError, 'exactly one'):
            native.member_bytes(self.source, 'root/LICENSE')

    def test_actual_review_pins_have_unique_components_and_hashes(self):
        manifest = json.loads((Path(__file__).resolve().parents[1] / 'licenses/native-inputs.json').read_text())
        self.assertEqual({c['id'] for c in manifest['components']}, {'rgb', 'analog', 'hidapi', 'libusb'})
        for c in manifest['components']:
            self.assertRegex(c['input_sha256'], '^[0-9a-f]{64}$')
            self.assertRegex(c['source']['sha256'], '^[0-9a-f]{64}$')


if __name__ == '__main__':
    unittest.main()
