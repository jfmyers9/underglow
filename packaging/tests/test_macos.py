import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import struct
import sys
import tempfile
import unittest
import zlib
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'packaging'))
import macos


class IconTests(unittest.TestCase):
    def test_icon_uses_underglow_u_instead_of_the_legacy_w(self):
        with tempfile.TemporaryDirectory() as temporary:
            icon = Path(temporary) / 'AppIcon.icns'
            macos.create_icon(icon)
            data = icon.read_bytes()
            self.assertEqual(data[:4], b'icns')
            self.assertEqual(data[8:12], b'ic07')
            length = struct.unpack('>I', data[12:16])[0]
            png = data[16:8 + length]
            self.assertEqual(png[:8], b'\x89PNG\r\n\x1a\n')
            offset, compressed = 8, bytearray()
            while offset < len(png):
                size = struct.unpack('>I', png[offset:offset + 4])[0]
                if png[offset + 4:offset + 8] == b'IDAT':
                    compressed.extend(png[offset + 8:offset + 8 + size])
                offset += size + 12
            pixels = zlib.decompress(compressed)
            def pixel(x, y):
                offset = int(y * 128) * (128 * 4 + 1) + 1 + int(x * 128) * 4
                return tuple(pixels[offset:offset + 4])
            self.assertEqual(pixel(.28, .35), (118, 235, 198, 255))
            self.assertEqual(pixel(.50, .68), (118, 235, 198, 255))
            self.assertEqual(pixel(.50, .40), (21, 29, 38, 255))


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='wooting macos tests ')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.notices = self.home / 'notices'
        self.notices.mkdir()
        for name in ('NOTICES.md', 'MPL-2.0.txt'):
            (self.notices / name).write_text('fixture only')
        self.inputs = {}
        for flag, name in [('binary', 'underglow'), ('gui', 'underglow-gui'),
                           ('service', 'underglow-service'), ('rgb-sdk', 'libwooting-rgb-sdk.dylib'),
                           ('analog-sdk', 'libwooting_analog_sdk_dist.dylib')]:
            path = self.home / name
            path.write_bytes(macos.MACH_MAGICS[0] + b'synthetic fixture')
            self.inputs[flag] = path
        self.mapping = {path.name: 'NOTICES.md' for path in self.inputs.values()}
        (self.notices / 'native-licenses.json').write_text(json.dumps(self.mapping))
        flags = [item for flag, path in self.inputs.items() for item in ('--' + flag, str(path))]
        self.args = macos.parser().parse_args(flags + [
            '--notices', str(self.notices), '--output', str(self.home / 'result'),
            '--version', '1.2.3', '--identifier', 'org.example.underglow',
            '--minimum-os', '13.0', '--architecture', 'arm64', '--verified-inputs'])
        shutil.copy2(ROOT / 'LICENSE', self.notices / 'APPLICATION-LICENSE.txt')
        self.write_audit()
        self.calls = []
        self.mock_runner = patch.object(macos, 'run', side_effect=self.fake_run)
        self.mock_runner.start()
        self.addCleanup(self.mock_runner.stop)
        self.os_patch = patch.object(macos.platform, 'system', return_value='Darwin')
        self.os_patch.start()
        self.addCleanup(self.os_patch.stop)
        self.release_patch = patch.object(macos.release, 'run', side_effect=self.fake_run)
        self.release_patch.start()
        self.addCleanup(self.release_patch.stop)
        self.rename_patch = patch.object(macos, 'publish_directory', side_effect=lambda src, dst: src.rename(dst))
        self.rename_patch.start()
        self.addCleanup(self.rename_patch.stop)

    def write_audit(self, **overrides):
        audit = {
            'schema_version': 1, 'status': 'reviewed', 'architecture': 'arm64',
            'unresolved': [],
            'native_inputs': {p.name: macos.digest(p) for p in self.inputs.values()},
            'files': {p.relative_to(self.notices).as_posix(): macos.digest(p)
                      for p in self.notices.rglob('*')
                      if p.is_file() and p.name != 'audit.json'},
        }
        audit.update(overrides)
        (self.notices / 'audit.json').write_text(json.dumps(audit))

    def test_pre_rename_audit_cannot_authorize_new_names(self):
        old_names = {'underglow': 'wooting-signals', 'underglow-gui': 'wooting-gui',
                     'underglow-service': 'wooting-service'}
        self.write_audit(native_inputs={old_names.get(p.name, p.name): macos.digest(p)
                                        for p in self.inputs.values()})
        with self.assertRaisesRegex(RuntimeError, 'native_inputs must exactly match'):
            macos.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_audit_missing_rejected(self):
        (self.notices / 'audit.json').unlink()
        with self.assertRaisesRegex(RuntimeError, 'require notices/audit.json'):
            macos.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_audit_incomplete_wrong_architecture_and_native_inputs_rejected(self):
        for override, error in [
                ({'schema_version': 2}, 'schema_version'),
                ({'status': 'pending'}, 'reviewed'),
                ({'unresolved': ['missing notice']}, 'unresolved'),
                ({'architecture': 'x86_64'}, 'architecture'),
                ({'native_inputs': {}}, 'native_inputs')]:
            with self.subTest(override=override):
                self.write_audit(**override)
                with self.assertRaisesRegex(RuntimeError, error):
                    macos.build(self.args)
                self.assertFalse(self.args.output.exists())

    def test_audit_missing_fields_and_extra_native_rejected(self):
        base = json.loads((self.notices / 'audit.json').read_text())
        for field in base:
            with self.subTest(field=field):
                audit = dict(base)
                del audit[field]
                (self.notices / 'audit.json').write_text(json.dumps(audit))
                with self.assertRaises(RuntimeError):
                    macos.build(self.args)
        native = dict(base['native_inputs'])
        native['unused.dylib'] = '0' * 64
        self.write_audit(native_inputs=native)
        with self.assertRaisesRegex(RuntimeError, 'native_inputs'):
            macos.build(self.args)

    def test_audit_uses_staged_basename_for_renamed_input(self):
        renamed = self.home / 'rgb-sdk.dylib'
        shutil.copy2(self.args.rgb_sdk, renamed)
        self.args.rgb_sdk = renamed
        macos.build(self.args)
        self.assertTrue(self.args.output.exists())

    def test_audit_modified_native_input_rejected(self):
        with self.inputs['gui'].open('ab') as stream:
            stream.write(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'native_inputs'):
            macos.build(self.args)

    def test_audit_modified_missing_and_extra_artifacts_rejected(self):
        for action in ('modify', 'remove', 'add'):
            with self.subTest(action=action):
                artifact = self.notices / 'source.tar.gz'
                artifact.write_bytes(b'source')
                self.write_audit()
                if action == 'modify':
                    artifact.write_bytes(b'changed')
                elif action == 'remove':
                    artifact.unlink()
                else:
                    (self.notices / 'extra.txt').write_text('unreviewed')
                with self.assertRaisesRegex(RuntimeError, 'audit files'):
                    macos.build(self.args)

    def test_audit_requires_complete_application_license(self):
        for text in (None, 'MIT License\nCopyright James Myers'):
            with self.subTest(text=text):
                license_file = self.notices / 'APPLICATION-LICENSE.txt'
                if text is None:
                    license_file.unlink()
                else:
                    license_file.write_text(text)
                self.write_audit()
                with self.assertRaisesRegex(RuntimeError, 'application MIT license'):
                    macos.build(self.args)

    def test_audit_binds_transitive_original_not_relocated_binary(self):
        library = self.home / 'libfixture.dylib'
        library.write_bytes(macos.MACH_MAGICS[0] + b'original library')
        self.mapping[library.name] = 'NOTICES.md'
        (self.notices / 'native-licenses.json').write_text(json.dumps(self.mapping))
        self.write_audit()
        def dependency(*args):
            if args[:2] == ('otool', '-L') and Path(args[-1]).resolve() == self.inputs['gui'].resolve():
                return 'fixture:\n\t' + str(library) + ' (compatibility version 1.0.0)'
            if args[0] == 'install_name_tool':
                with Path(args[-1]).open('ab') as stream:
                    stream.write(b'relocated')
            return self.fake_run(*args)
        with patch.object(macos.release, 'run', side_effect=dependency):
            with self.assertRaisesRegex(RuntimeError, 'native_inputs'):
                macos.build(self.args)
            self.inputs['transitive'] = library
            self.write_audit()
            macos.build(self.args)
        staged = self.args.output / macos.APP / 'Contents/Frameworks' / library.name
        self.assertNotEqual(macos.digest(library), macos.digest(staged))

    def fake_run(self, *args):
        self.calls.append(tuple(str(a) for a in args))
        tool = Path(args[0]).name
        if tool == 'lipo':
            return 'arm64'
        if tool == 'otool':
            if args[1] == '-l':
                return 'cmd LC_BUILD_VERSION\ncmdsize 32\nplatform 1\nminos 11.0\nsdk 14.0'
            return str(args[-1]) + ':\n'
        if tool == 'git':
            return 'abc123' if 'rev-parse' in args else ''
        if tool == 'hdiutil':
            root = Path(args[args.index('-srcfolder') + 1])
            self.assertEqual(os.readlink(root / 'Applications'), '/Applications')
            self.assertTrue((root / macos.APP / 'Contents/Info.plist').is_file())
            self.image_readme = (root / 'READ ME.txt').read_text()
            Path(args[-1]).write_bytes(b'fixture disk image')
        if tool == 'xcrun' and args[1] == 'notarytool':
            return '{"status":"Accepted"}'
        return ''

    def test_self_contained_layout_metadata_and_no_host_actions(self):
        macos.build(self.args)
        provenance = json.loads((self.args.output / 'provenance.json').read_text())
        self.assertEqual(provenance['distribution'], 'ad-hoc-unnotarized')
        self.assertEqual(provenance['audit_sha256'], macos.digest(self.notices / 'audit.json'))
        self.assertIn('Open Anyway is not guaranteed', self.image_readme)
        self.assertNotIn('not for public distribution', self.image_readme)
        app = self.args.output / macos.APP
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleExecutable'], 'underglow-gui')
        self.assertEqual(info['CFBundleShortVersionString'], '1.2.3')
        self.assertEqual(info['LSMinimumSystemVersion'], '13.0')
        self.assertEqual(info['LSApplicationCategoryType'], 'public.app-category.utilities')
        self.assertIn('Key input is not recorded', info['NSInputMonitoringUsageDescription'])
        self.assertEqual(info['CFBundleIconFile'], 'AppIcon')
        self.assertEqual((app / 'Contents/Resources/AppIcon.icns').read_bytes()[:4], b'icns')
        self.assertEqual(sorted(p.name for p in (app / 'Contents/MacOS').iterdir()),
                         ['underglow', 'underglow-gui', 'underglow-service',
                          'wooting-gui', 'wooting-service', 'wooting-signals'])
        self.assertEqual(info['CFBundleName'], 'Underglow')
        self.assertEqual(info['CFBundleIdentifier'], self.args.identifier)
        for old, new in [('wooting-signals', 'underglow'),
                         ('wooting-gui', 'underglow-gui'),
                         ('wooting-service', 'underglow-service')]:
            alias = app / 'Contents/MacOS' / old
            self.assertTrue(alias.is_symlink())
            self.assertEqual(os.readlink(alias), new)
            self.assertEqual(alias.resolve(), (app / 'Contents/MacOS' / new).resolve())
        self.assertEqual(len(list((app / 'Contents/Frameworks').iterdir())), 2)
        provenance = json.loads((self.args.output / 'provenance.json').read_text())
        self.assertEqual(provenance['distribution'], 'ad-hoc-unnotarized')
        self.assertEqual(len(provenance['native_inputs']), 5)
        self.assertIn('Contents/MacOS/underglow-service', provenance['native_inputs'])
        dmg = next(self.args.output.glob('*.dmg'))
        self.assertEqual((self.args.output / 'SHA256SUMS').read_text(), macos.digest(dmg) + '  ' + dmg.name + '\n')
        self.assertFalse(any('attach' in c or 'launchctl' in c or 'notarytool' in c for c in self.calls))
        self.assertFalse(list(self.home.glob('.wooting-macos-*')))

    def test_missing_service_license_mapping_rejected(self):
        del self.mapping['underglow-service']
        (self.notices / 'native-licenses.json').write_text(json.dumps(self.mapping))
        with self.assertRaisesRegex(RuntimeError, 'missing native license mapping: underglow-service'):
            macos.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_all_notice_symlinks_rejected_including_unused(self):
        (self.notices / 'unused').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(RuntimeError, 'regular files'):
            macos.build(self.args)

    def test_notice_escape_mapping_rejected(self):
        self.mapping['underglow-service'] = '../underglow-service'
        (self.notices / 'native-licenses.json').write_text(json.dumps(self.mapping))
        with self.assertRaisesRegex(RuntimeError, 'invalid license'):
            macos.build(self.args)

    def test_shell_helper_rejected(self):
        self.args.service.write_text('#!/bin/sh\nexit 0\n')
        with self.assertRaisesRegex(RuntimeError, 'Mach-O'):
            macos.build(self.args)

    def test_incompatible_architecture_and_deployment_target(self):
        self.args.architecture = 'universal2'
        with self.assertRaisesRegex(RuntimeError, 'architecture'):
            macos.build(self.args)
        self.args.architecture = 'arm64'
        self.args.minimum_os = '10.15'
        with self.assertRaisesRegex(RuntimeError, 'deployment target'):
            macos.build(self.args)

    def test_local_test_allows_pending_review_but_forbids_distribution_signing(self):
        self.args.verified_inputs = False
        self.args.local_test = True
        (self.notices / 'audit.json').unlink()
        macos.build(self.args)
        provenance = json.loads((self.args.output / 'provenance.json').read_text())
        self.assertEqual(provenance['license_review'], 'pending-not-for-distribution')
        self.assertEqual(provenance['distribution'], 'ad-hoc-local-test-only')
        self.assertIn('not for public distribution', self.image_readme)
        self.args.output = self.home / 'signed-output'
        self.args.sign_identity = 'Developer ID Application: Test (TEST)'
        with self.assertRaisesRegex(RuntimeError, 'forbids'):
            macos.build(self.args)

    def test_missing_operator_confirmation(self):
        self.args.verified_inputs = False
        with self.assertRaisesRegex(RuntimeError, 'operator review'):
            macos.build(self.args)

    def test_output_refuses_existing_directory_or_dangling_symlink(self):
        self.args.output.mkdir()
        with self.assertRaisesRegex(RuntimeError, 'refusing overwrite'):
            macos.build(self.args)
        self.args.output.rmdir()
        self.args.output.symlink_to(self.home / 'nonexistent')
        with self.assertRaisesRegex(RuntimeError, 'refusing overwrite'):
            macos.build(self.args)

    def test_notarization_requires_developer_identity(self):
        self.args.notary_profile = 'test-profile'
        with self.assertRaisesRegex(RuntimeError, 'requires --sign-identity'):
            macos.build(self.args)

    def test_explicit_signing_and_notarization_order_mock_only(self):
        self.args.sign_identity = 'Developer ID Application: Test Only (TEST)'
        self.args.notary_profile = 'test-profile'
        macos.build(self.args)
        submissions = [c for c in self.calls if 'notarytool' in c]
        self.assertEqual(len(submissions), 2)
        self.assertTrue(submissions[0][3].endswith('.zip'))
        self.assertTrue(submissions[1][3].endswith('.dmg'))
        self.assertTrue(all('--keychain-profile' in c and 'test-profile' in c for c in submissions))
        staples = [c for c in self.calls if 'stapler' in c and 'staple' in c]
        self.assertEqual(len(staples), 2)
        signatures = [c for c in self.calls if c[0] == 'codesign' and '--sign' in c]
        self.assertEqual(len(signatures), 7)
        self.assertTrue(all('--timestamp' in c for c in signatures))

    def test_failure_leaves_no_partial_output(self):
        original = self.fake_run
        def fail(*args):
            if args[0] == 'hdiutil':
                raise RuntimeError('fixture failure')
            return original(*args)
        with patch.object(macos, 'run', side_effect=fail):
            with self.assertRaisesRegex(RuntimeError, 'fixture failure'):
                macos.build(self.args)
        self.assertFalse(self.args.output.exists())
        self.assertFalse(list(self.home.glob('.wooting-macos-*')))

    def test_notary_rejection_cleans_up_without_creating_dmg(self):
        self.args.sign_identity = 'Developer ID Application: Test Only (TEST)'
        self.args.notary_profile = 'test-profile'
        def reject(*args):
            if args[:2] == ('xcrun', 'notarytool'):
                return '{"status":"Invalid"}'
            return self.fake_run(*args)
        with patch.object(macos, 'run', side_effect=reject):
            with self.assertRaisesRegex(RuntimeError, 'not accepted'):
                macos.build(self.args)
        self.assertFalse(self.args.output.exists())
        self.assertFalse(any(c[0] == 'hdiutil' for c in self.calls))
        self.assertFalse(list(self.home.glob('.wooting-macos-*')))

    def test_transitive_library_deployment_target_checked(self):
        library = self.home / 'libfixture.dylib'
        library.write_bytes(macos.MACH_MAGICS[0])
        def dependency(*args):
            if args[:2] == ('otool', '-L') and Path(args[-1]).name == 'underglow-gui':
                return 'fixture:\n\t' + str(library) + ' (compatibility version 1.0.0, current version 1.0.0)'
            return self.fake_run(*args)
        def too_new(*args):
            if args[:2] == ('otool', '-l') and Path(args[-1]).name == library.name:
                return 'cmd LC_BUILD_VERSION\ncmdsize 32\nplatform 1\nminos 26.0'
            return self.fake_run(*args)
        with patch.object(macos.release, 'run', side_effect=dependency), patch.object(macos, 'run', side_effect=too_new):
            with self.assertRaisesRegex(RuntimeError, 'deployment target'):
                macos.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_ios_binary_rejected(self):
        def ios(*args):
            if args[:2] == ('otool', '-l'):
                return 'cmd LC_BUILD_VERSION\ncmdsize 32\nplatform 2\nminos 11.0'
            return self.fake_run(*args)
        with patch.object(macos, 'run', side_effect=ios):
            with self.assertRaisesRegex(RuntimeError, 'not a macOS binary'):
                macos.build(self.args)

    def test_fat_dependency_headers_and_duplicate_rpaths(self):
        with patch.object(macos.release, 'run', return_value='fixture (architecture arm64):\n\t@rpath/libfixture.dylib (compatibility version 1.0.0)\nfixture (architecture x86_64):\n\t@rpath/libfixture.dylib (compatibility version 1.0.0)'):
            self.assertEqual(macos.release.mac_dependencies(Path('fixture')), ['@rpath/libfixture.dylib'])
        with patch.object(macos.release, 'run', return_value=('cmd LC_RPATH\ncmdsize 40\npath @loader_path (offset 12)\n' * 2)):
            self.assertEqual(macos.release.mac_rpaths(Path('fixture')), ['@loader_path'])

    def test_closure_relocation_preserves_prefix_and_app_layouts(self):
        executable = self.inputs['gui']
        library = self.home / 'libfixture.dylib'
        library.write_bytes(macos.MACH_MAGICS[0])
        def fake(*args):
            if args[:2] == ('otool', '-L') and Path(args[2]).resolve() == executable.resolve():
                return str(executable) + ':\n\t' + str(library) + ' (compatibility version 1.0.0, current version 1.0.0)'
            return self.fake_run(*args)
        for binary_dir, library_dir, relative in [
                ('bin', 'lib/wooting-signals', '@loader_path/../lib/wooting-signals/libfixture.dylib'),
                ('Contents/MacOS', 'Contents/Frameworks', '@loader_path/../Frameworks/libfixture.dylib')]:
            stage = self.home / binary_dir.replace('/', '-')
            with patch.object(macos.release, 'run', side_effect=fake):
                origins = macos.release.bundle_dependencies(stage, 'Darwin', library_dir, sign=False)(
                    [(executable, stage / binary_dir / 'underglow-gui')])
            self.assertIn(library_dir + '/libfixture.dylib', origins)
            self.assertTrue(any(relative in c for c in self.calls))


@unittest.skipUnless(platform.system() == 'Darwin' and shutil.which('clang'), 'native Darwin fixture')
class NativeFixtureTests(unittest.TestCase):
    def test_native_relocation_signing_and_atomic_publish(self):
        with tempfile.TemporaryDirectory(prefix='wooting native fixture ') as temporary:
            home = Path(temporary)
            lib = home / 'libfixture.dylib'
            source = home / 'fixture.c'
            source.write_text('int fixture(void) { return 0; }\n')
            subprocess.run(['/usr/bin/clang', '-dynamiclib', '-mmacosx-version-min=11.0',
                            str(source), '-o', str(lib)], check=True, capture_output=True)
            exe = home / 'fixture'
            source.write_text('extern int fixture(void); int main(void) { return fixture(); }\n')
            subprocess.run(['/usr/bin/clang', '-mmacosx-version-min=11.0', str(source), str(lib),
                            '-Wl,-headerpad_max_install_names', '-o', str(exe)], check=True, capture_output=True)
            stage = home / 'stage'
            origins = macos.release.bundle_dependencies(stage, 'Darwin', 'Contents/Frameworks')(
                [(exe, stage / 'Contents/MacOS/fixture')])
            self.assertIn('Contents/Frameworks/libfixture.dylib', origins)
            imports = macos.release.mac_dependencies(stage / 'Contents/MacOS/fixture')
            self.assertIn('@loader_path/../Frameworks/libfixture.dylib', imports)
            macos.validate_native(exe, platform.machine(), '15.0')
            subprocess.run(['/usr/bin/codesign', '--verify', '--strict',
                            str(stage / 'Contents/MacOS/fixture')], check=True, capture_output=True)
            output = home / 'output'
            macos.publish_directory(stage, output)
            self.assertTrue(output.is_dir())
            stage.mkdir()
            with self.assertRaises(OSError):
                macos.publish_directory(stage, output)
            self.assertTrue(stage.exists())
            icon = home / 'fixture.icns'
            macos.create_icon(icon)
            subprocess.run(['/usr/bin/iconutil', '-c', 'iconset', str(icon)], check=True, capture_output=True)
            self.assertTrue((home / 'fixture.iconset/icon_512x512.png').is_file())
            notices = home / 'notices'
            notices.mkdir()
            for name in ('NOTICES.md', 'MPL-2.0.txt'):
                (notices / name).write_text('Synthetic fixture only, not product license review')
            names = ['underglow-gui', 'underglow', 'underglow-service',
                     'libwooting-rgb-sdk.dylib', 'libwooting_analog_sdk_dist.dylib', 'libfixture.dylib']
            (notices / 'native-licenses.json').write_text(json.dumps({name: 'NOTICES.md' for name in names}))
            flags = ['--binary', str(exe), '--gui', str(exe), '--service', str(exe),
                     '--rgb-sdk', str(lib), '--analog-sdk', str(lib), '--notices', str(notices),
                     '--output', str(home / 'product'), '--version', '0.0.1',
                     '--identifier', 'org.example.fixture', '--minimum-os', '15.0',
                     '--architecture', platform.machine(), '--local-test']
            macos.build(macos.parser().parse_args(flags))
            self.assertTrue(next((home / 'product').glob('*.dmg')).stat().st_size > 0)
            subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict',
                            str(home / 'product' / macos.APP)], check=True, capture_output=True)


if __name__ == '__main__':
    unittest.main()
