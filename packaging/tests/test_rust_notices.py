"""Offline synthetic-archive tests; never build crates or contact registries."""
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('rust_notices', Path(__file__).parents[1] / 'collect_rust_notices.py')
notices = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(notices)


class RustNoticesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'registry/src/example/demo-1.0.0'
        self.source.mkdir(parents=True)
        self.archive = self.root / 'registry/cache/example/demo-1.0.0.crate'
        self.archive.parent.mkdir(parents=True)
        self.contents = {'LICENSE': b'Synthetic complete license test fixture, not legal text.',
                         'Cargo.toml': b'[package]\nname="demo"\nversion="1.0.0"\nlicense="MIT"\n',
                         'README.md': b'Original README',
                         'native/LICENSE-bsd': b'Native BSD notice',
                         'native/AUTHORS.txt': b'Original author names',
                         'fonts/OFL.txt': b'Original OFL',
                         'fonts/font.ttf': b'\0embedded copyright metadata',
                         'src/lib.rs': b'// Copyright Original author\nfn example() {}'}
        self.make_archive()
        self.package = {'id': 'demo', 'name': 'demo', 'version': '1.0.0',
                        'license': 'MIT', 'source': notices.REGISTRY,
                        'manifest_path': str(self.source / 'Cargo.toml')}
        self.metadata = {'packages': [self.package], 'workspace_members': ['demo'],
                         'resolve': {'nodes': [{'id': 'demo', 'dependencies': []}]}}
        self.rule = {'crate_sha256': notices.sha256(self.archive.read_bytes()),
                     'declared_license': 'MIT', 'selected_license': 'MIT',
                     'policy_kind': 'verbatim-license-text',
                     'required_files': {'LICENSE': notices.sha256(self.contents['LICENSE'])}}
        self.policy = {'schema_version': 1, 'packages': {'demo@1.0.0': self.rule}}
        self.lock = self.root / 'Cargo.lock'
        self.sync_lock()

    def make_archive(self, extra=None):
        with tarfile.open(self.archive, 'w:gz') as archive:
            for path, data in self.contents.items():
                member = tarfile.TarInfo('demo-1.0.0/' + path)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
            if extra:
                archive.addfile(extra, io.BytesIO(b'x' * extra.size))

    def sync_lock(self):
        self.lock.write_text('version=3\n[[package]]\nname="demo"\nversion="1.0.0"\n'
                             f'source="{notices.REGISTRY}"\nchecksum="{self.rule["crate_sha256"]}"\n')

    def collect(self, downloader=None):
        return notices.collect(self.metadata, self.root / 'output', self.policy, self.lock,
                               downloader=downloader or (lambda *_: self.fail('unexpected network')))

    def test_complete_preserves_nested_notices_fonts_authors_and_source(self):
        result = self.collect()
        self.assertEqual(result['status'], 'complete')
        base = self.root / 'output/rust/demo-1.0.0'
        for name, data in self.contents.items():
            self.assertEqual((base / name).read_bytes(), data)
        self.assertNotIn('manifest.json', result['files'])
        self.assertIn('NOTICES.md', result['files'])
        for path, checksum in result['files'].items():
            self.assertEqual(notices.sha256((self.root / 'output' / path).read_bytes()), checksum)

    def test_unpacked_cache_modifications_do_not_change_notices(self):
        (self.source / 'LICENSE').write_text('Invented local attribution')
        self.collect()
        self.assertEqual((self.root / 'output/rust/demo-1.0.0/LICENSE').read_bytes(), self.contents['LICENSE'])

    def test_archive_checksum_mismatch_fails_closed(self):
        self.archive.write_bytes(self.archive.read_bytes() + b'changed')
        result = self.collect()
        self.assertEqual(result['status'], 'incomplete')
        self.assertIn('checksum mismatch', result['unresolved'][0]['reason'])

    def test_unreviewed_version_is_unresolved(self):
        self.policy['packages'] = {}
        self.assertEqual(self.collect()['status'], 'incomplete')

    def test_changed_declared_license_is_unresolved(self):
        self.package['license'] = 'GPL-3.0-only'
        self.assertEqual(self.collect()['status'], 'incomplete')

    def test_missing_required_and_license_fails(self):
        self.rule['selected_license'] = 'MIT AND ISC'
        self.rule['required_files']['LICENSE-ISC'] = '0' * 64
        self.assertIn('required reviewed notice', self.collect()['unresolved'][0]['reason'])

    def test_filename_or_readme_is_not_automatic_approval(self):
        self.rule['required_files'] = {}
        self.assertEqual(self.collect()['status'], 'incomplete')

    def test_changed_reviewed_text_fails(self):
        self.rule['required_files']['LICENSE'] = '0' * 64
        self.assertEqual(self.collect()['status'], 'incomplete')

    def test_explicit_unresolved_review_fails(self):
        self.rule['unresolved'] = 'Needs permission review'
        self.assertEqual(self.collect()['unresolved'][0]['reason'], 'Needs permission review')

    def test_new_directory_required(self):
        (self.root / 'output').mkdir()
        with self.assertRaises(FileExistsError):
            self.collect()

    def test_reachability_excludes_other_workspace_members(self):
        other = dict(self.package, name='other', id='other')
        self.metadata['packages'].append(other)
        self.metadata['workspace_members'].append('other')
        self.metadata['resolve']['nodes'].append({'id': 'other', 'dependencies': []})
        self.assertEqual(notices.selected_packages(self.metadata, 'demo'), [self.package])
        with self.assertRaises(ValueError):
            notices.selected_packages(self.metadata, 'absent')

    def test_reachability_includes_build_dependencies(self):
        other = dict(self.package, name='other', id='other')
        self.metadata['packages'].append(other)
        self.metadata['resolve']['nodes'][0]['dependencies'] = ['other']
        self.metadata['resolve']['nodes'].append({'id': 'other', 'dependencies': []})
        self.assertEqual(len(notices.selected_packages(self.metadata, 'demo')), 2)

    def spec(self):
        return {'path': 'standard-terms/MIT.txt', 'sha256': notices.sha256(b'canonical fixture'),
                'url': 'https://raw.githubusercontent.com/spdx/license-list-data/' + 'a' * 40 + '/text/MIT.txt',
                'commit': 'a' * 40}

    def test_metadata_only_preserves_entire_archive_and_separates_standard_terms(self):
        spec = self.spec()
        self.rule.update(policy_kind='metadata-declaration-with-source', standard_files=[spec])
        self.rule['required_files'][spec['path']] = spec['sha256']
        result = self.collect(lambda *_: b'canonical fixture')
        self.assertEqual(result['status'], 'complete')
        self.assertEqual((self.root / 'output/rust/demo-1.0.0/original-source.crate').read_bytes(), self.archive.read_bytes())
        self.assertEqual(result['packages'][0]['policy_kind'], 'metadata-declaration-with-source')

    def test_metadata_only_without_terms_fails(self):
        self.rule['policy_kind'] = 'metadata-declaration-with-source'
        self.assertEqual(self.collect()['status'], 'incomplete')

    def test_upstream_changed_hash_fails_even_with_mock_downloader(self):
        self.rule.update(standard_files=[self.spec()])
        self.assertEqual(self.collect(lambda *_: b'altered')['status'], 'incomplete')

    def test_upstream_urls_must_pin_commit_and_https(self):
        for url in ['https://raw.githubusercontent.com/org/repo/main/LICENSE',
                    'http://raw.githubusercontent.com/org/repo/' + 'a' * 40 + '/LICENSE',
                    'https://evil.test/org/repo/' + 'a' * 40 + '/LICENSE',
                    self.spec()['url'] + '?token=secret']:
            spec = dict(self.spec(), url=url)
            with self.subTest(url=url), self.assertRaises(ValueError):
                notices.validate_upstream(spec, 'a' * 40)

    def test_unsafe_archive_names_and_symlinks_rejected(self):
        for name, kind in [('demo-1.0.0/../escape', tarfile.REGTYPE),
                           ('/absolute/path', tarfile.REGTYPE),
                           ('demo-1.0.0/link', tarfile.SYMTYPE)]:
            member = tarfile.TarInfo(name)
            member.type = kind
            member.linkname = '/etc/passwd'
            self.make_archive(member)
            with self.subTest(name=name), self.assertRaises(ValueError):
                notices.archive_files(self.archive, notices.sha256(self.archive.read_bytes()), 'demo-1.0.0')

    def test_whole_notice_directory_is_preserved(self):
        self.assertTrue(notices.retained('LICENSES/BoringSSL.txt', b'original terms'))
        self.assertTrue(notices.retained('notices/subfolder/attribution', b'original attribution'))

    def test_archive_size_bounds(self):
        with patch.object(notices, 'MAX_EXPANDED', 1), self.assertRaises(ValueError):
            notices.archive_files(self.archive, notices.sha256(self.archive.read_bytes()), 'demo-1.0.0')

    def test_repository_policy_pins_every_required_text_and_upstream(self):
        policy = json.loads(notices.DEFAULT_OVERRIDES.read_text())
        self.assertEqual(policy['schema_version'], 1)
        for key, rule in policy['packages'].items():
            with self.subTest(package=key):
                notices.checked_hash(rule['crate_sha256'])
                self.assertTrue(rule['required_files'])
                for path, digest in rule['required_files'].items():
                    notices.safe_path(path)
                    notices.checked_hash(digest)
                for spec in rule.get('upstream_files', []) + rule.get('standard_files', []):
                    notices.validate_upstream(spec, spec.get('commit', rule.get('upstream_commit')))


if __name__ == '__main__':
    unittest.main()
