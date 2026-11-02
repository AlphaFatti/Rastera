"""Transactional installation guards; no real applications/preferences are touched."""
import importlib.util
import plistlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('rastera_packager', Path(__file__).with_name('package_app.py'))
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class InstallationTests(unittest.TestCase):
    def bundle(self, path, identity, marker):
        (path/'Contents').mkdir(parents=True)
        (path/'Contents/Info.plist').write_bytes(plistlib.dumps(dict(CFBundleIdentifier=identity)))
        (path/'marker').write_text(marker)
        return path

    def test_verification_failure_restores_previous_copy(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root)
            old=self.bundle(root/'Rastera.app',packager.BUNDLE_ID,'old')
            staged=self.bundle(root/'incoming.app',packager.BUNDLE_ID,'new')
            with patch.object(packager,'verify',side_effect=RuntimeError('invalid signature')):
                with self.assertRaises(RuntimeError):
                    packager.replace(staged,old)
            self.assertEqual((old/'marker').read_text(),'old')
            self.assertEqual((staged/'marker').read_text(),'new')

    def test_unrelated_bundle_is_preserved(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root)
            old=self.bundle(root/'Rastera.app','someone.else','unrelated')
            staged=self.bundle(root/'incoming.app',packager.BUNDLE_ID,'new')
            with self.assertRaises(RuntimeError):
                packager.replace(staged,old)
            self.assertEqual((old/'marker').read_text(),'unrelated')
            self.assertTrue(staged.exists())

    def test_success_replaces_only_the_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root)
            old=self.bundle(root/'Rastera.app',packager.BUNDLE_ID,'old')
            staged=self.bundle(root/'incoming.app',packager.BUNDLE_ID,'new')
            project=root/'show.rastera.json';project.write_text('untouched')
            with patch.object(packager,'verify'):
                packager.replace(staged,old)
            self.assertEqual((old/'marker').read_text(),'new')
            self.assertEqual(project.read_text(),'untouched')
            self.assertEqual(list(root.glob('.Rastera.previous-*')),[])


class DistributionTests(unittest.TestCase):
    def test_dirty_distribution_is_rejected(self):
        with patch.object(packager.subprocess,'check_output',side_effect=['a'*40+'\n',' M README.md\n']):
            with self.assertRaisesRegex(RuntimeError,'clean committed'):
                packager.source_metadata(require_clean=True)

    def test_no_credentials_means_explicit_unnotarized_preview(self):
        with patch.dict(packager.os.environ,{},clear=True), patch.object(packager.subprocess,'check_output',return_value='0 valid identities found\n'):
            self.assertEqual(packager.distribution_signing('auto'),('-',None,'ad-hoc-unnotarized'))
            with self.assertRaises(RuntimeError):
                packager.distribution_signing('developer-id')

    def test_distribution_credentials_are_separate_from_local_signing(self):
        identity='A'*40
        with patch.dict(packager.os.environ,{'RASTERA_SIGN_IDENTITY':identity},clear=True), patch.object(packager.subprocess,'check_output',return_value=f'1) {identity} "Developer ID Application: Test"\n'):
            self.assertEqual(packager.distribution_signing('auto'),('-',None,'ad-hoc-unnotarized'))
        with patch.dict(packager.os.environ,{'RASTERA_DISTRIBUTION_IDENTITY':identity,'RASTERA_NOTARY_PROFILE':'test-profile'},clear=True), patch.object(packager.subprocess,'check_output',return_value=f'1) {identity} "Developer ID Application: Test"\n'):
            self.assertEqual(packager.distribution_signing('auto'),(identity,'test-profile','developer-id-notarized'))
            self.assertEqual(packager.distribution_signing('adhoc'),('-',None,'ad-hoc-unnotarized'))

    def test_payload_excludes_profiles_and_unexpected_files(self):
        with tempfile.TemporaryDirectory() as root:
            root=Path(root)
            (root/'Applications').symlink_to('/Applications')
            (root/'Tester Guide.txt').write_text('guide')
            resources=root/'Rastera.app/Contents/Resources';resources.mkdir(parents=True)
            (resources/'build-info.json').write_text('{}')
            packager.validate_payload(root)
            (resources/'recovery.json').write_text('private')
            with self.assertRaisesRegex(RuntimeError,'Unexpected app payload'):
                packager.validate_payload(root)
            (resources/'recovery.json').unlink()
            (root/'personal.rastera.json').write_text('private')
            with self.assertRaisesRegex(RuntimeError,'Unexpected DMG payload'):
                packager.validate_payload(root)

    def test_readable_checksums_match_final_bytes(self):
        with tempfile.TemporaryDirectory() as root:
            path=Path(root)/'package.dmg';path.write_bytes(b'abc')
            self.assertEqual(packager.checksums([path]),'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  package.dmg\n')

    def test_actual_macos_deployment_target(self):
        with patch.object(packager.subprocess,'check_output',return_value='platform MACOS\n minos 14.0\n sdk 15.5\n'):
            self.assertEqual(packager.normalized_version(packager.deployment_target(Path('app'))),(14,))
        with patch.object(packager.subprocess,'check_output',return_value='platform IOS\n minos 14.0\n'):
            with self.assertRaises(RuntimeError):
                packager.deployment_target(Path('app'))
