import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'packaging'))
import macos


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
        for flag, name in [('binary', 'wooting-signals'), ('gui', 'wooting-gui'),
                           ('service', 'wooting-service'), ('rgb-sdk', 'libwooting-rgb-sdk.dylib'),
                           ('analog-sdk', 'libwooting_analog_sdk_dist.dylib')]:
            path = self.home / name
            path.write_bytes(macos.MACH_MAGICS[0] + b'synthetic fixture')
            self.inputs[flag] = path
        self.mapping = {path.name: 'NOTICES.md' for path in self.inputs.values()}
        (self.notices / 'native-licenses.json').write_text(json.dumps(self.mapping))
        flags = [item for flag, path in self.inputs.items() for item in ('--' + flag, str(path))]
        self.args = macos.parser().parse_args(flags + [
            '--notices', str(self.notices), '--output', str(self.home / 'result'),
            '--version', '1.2.3', '--identifier', 'org.example.wooting-signals',
            '--minimum-os', '13.0', '--architecture', 'arm64', '--verified-inputs'])
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
            Path(args[-1]).write_bytes(b'fixture disk image')
        if tool == 'xcrun' and args[1] == 'notarytool':
            return '{"status":"Accepted"}'
        return ''

    def test_self_contained_layout_metadata_and_no_host_actions(self):
        macos.build(self.args)
        app = self.args.output / macos.APP
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleExecutable'], 'wooting-gui')
        self.assertEqual(info['CFBundleShortVersionString'], '1.2.3')
        self.assertEqual(info['LSMinimumSystemVersion'], '13.0')
        self.assertEqual(info['LSApplicationCategoryType'], 'public.app-category.utilities')
        self.assertIn('Key input is not recorded', info['NSInputMonitoringUsageDescription'])
        self.assertEqual(info['CFBundleIconFile'], 'AppIcon')
        self.assertEqual((app / 'Contents/Resources/AppIcon.icns').read_bytes()[:4], b'icns')
        self.assertEqual(sorted(p.name for p in (app / 'Contents/MacOS').iterdir()),
                         ['wooting-gui', 'wooting-service', 'wooting-signals'])
        self.assertEqual(len(list((app / 'Contents/Frameworks').iterdir())), 2)
        provenance = json.loads((self.args.output / 'provenance.json').read_text())
        self.assertEqual(provenance['distribution'], 'ad-hoc-local-test-only')
        self.assertEqual(len(provenance['native_inputs']), 5)
        self.assertIn('Contents/MacOS/wooting-service', provenance['native_inputs'])
        dmg = next(self.args.output.glob('*.dmg'))
        self.assertEqual((self.args.output / 'SHA256SUMS').read_text(), macos.digest(dmg) + '  ' + dmg.name + '\n')
        self.assertFalse(any('attach' in c or 'launchctl' in c or 'notarytool' in c for c in self.calls))
        self.assertFalse(list(self.home.glob('.wooting-macos-*')))

    def test_missing_service_license_mapping_rejected(self):
        del self.mapping['wooting-service']
        (self.notices / 'native-licenses.json').write_text(json.dumps(self.mapping))
        with self.assertRaisesRegex(RuntimeError, 'missing native license mapping: wooting-service'):
            macos.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_all_notice_symlinks_rejected_including_unused(self):
        (self.notices / 'unused').symlink_to('/etc/passwd')
        with self.assertRaisesRegex(RuntimeError, 'regular files'):
            macos.build(self.args)

    def test_notice_escape_mapping_rejected(self):
        self.mapping['wooting-service'] = '../wooting-service'
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
        macos.build(self.args)
        provenance = json.loads((self.args.output / 'provenance.json').read_text())
        self.assertEqual(provenance['license_review'], 'pending-not-for-distribution')
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
            if args[:2] == ('otool', '-L') and Path(args[-1]).name == 'wooting-gui':
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
                    [(executable, stage / binary_dir / 'wooting-gui')])
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
            names = ['wooting-gui', 'wooting-signals', 'wooting-service',
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
