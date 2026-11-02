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
