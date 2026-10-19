import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('prepare', ROOT / 'scripts/prepare-release.py')
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class AutoReleaseTests(unittest.TestCase):
    def test_stamps_all_versions_without_modifying_other_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = ['package.json', 'package-lock.json', 'src-tauri/Cargo.toml',
                     'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json']
            for name in names:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, target)
            old = (root / 'src-tauri/Cargo.lock').read_text(encoding='utf-8')
            version = prepare.stamp(root, 321)
            self.assertTrue(version.endswith('.321'))
            self.assertEqual(json.loads((root / 'package-lock.json').read_text(encoding='utf-8'))['packages']['']['version'], version)
            # Exactly the application version changes in the Rust dependency lock.
            before = old.split('name = "x-talk"\nversion = "')
            after = (root / 'src-tauri/Cargo.lock').read_text(encoding='utf-8').split('name = "x-talk"\nversion = "')
            self.assertEqual(before[0], after[0])
            self.assertEqual(before[1].split('"', 1)[1], after[1].split('"', 1)[1])
            for number in [0, -1, 65536]:
                with self.assertRaises(ValueError):
                    prepare.stamp(root, number)
