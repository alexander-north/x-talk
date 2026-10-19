"""Release failures must be caught before assets reach GitHub."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import subprocess
import tarfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('release', ROOT / 'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in ['package.json', 'package-lock.json', 'src-tauri/Cargo.toml',
                     'src-tauri/Cargo.lock', 'src-tauri/tauri.conf.json']:
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest)
        self.version = release.version(self.root)

    def test_versions_and_matching_tag(self):
        self.assertEqual(release.version(self.root, f'v{self.version}'), self.version)

    def test_tag_cannot_publish_different_version(self):
        for tag in ['main', 'v0.0.0', 'v1.4.1; echo unsafe', '../v1.4.1']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.version(self.root, tag)

    def test_stale_lockfile_is_rejected(self):
        path = self.root / 'package-lock.json'
        lock = json.loads(path.read_text())
        lock['packages']['']['version'] = '0.0.0'
        path.write_text(json.dumps(lock))
        with self.assertRaisesRegex(ValueError, 'disagree'):
            release.version(self.root)

    def test_wrong_installer_cannot_be_packaged(self):
        with self.assertRaisesRegex(ValueError, 'Expected installer'):
            release.assemble(self.root, self.root / 'old.exe', self.root / 'source',
                             self.root / 'out', f'v{self.version}')

    def test_corrupt_installer_is_rejected(self):
        installer = self.root / f'x-talk_{self.version}_x64-setup.exe'
        installer.write_bytes(b'not a PE file')
        with self.assertRaisesRegex(ValueError, 'Windows executable'):
            release.assemble(self.root, installer, self.root / 'source',
                             self.root / 'out', f'v{self.version}')

    def test_source_checksum_failure_prevents_output(self):
        installer = self.root / f'x-talk_{self.version}_x64-setup.exe'
        installer.write_bytes(b'MZfixture')
        source = self.root / 'source'
        source.write_bytes(b'wrong source')
        (self.root / 'scripts').mkdir()
        shutil.copyfile(ROOT / 'scripts/media.lock.json', self.root / 'scripts/media.lock.json')
        with self.assertRaisesRegex(ValueError, 'source checksum'):
            release.assemble(self.root, installer, source, self.root / 'out', f'v{self.version}')
        self.assertFalse((self.root / 'out').exists())

    def test_complete_asset_set_has_verified_checksums_and_excludes_local_secrets(self):
        installer = self.root / f'x-talk_{self.version}_x64-setup.exe'
        installer.write_bytes(b'MZfixture')
        source = self.root / 'ffmpeg.tar.xz'
        source.write_bytes(b'fixture source')
        materials = ['LICENSE', 'NOTICE', 'scripts/build-media.sh', 'scripts/verify-media.py',
                     'src-tauri/binaries/FFmpeg-LICENSE.txt', 'src-tauri/binaries/NOTICE.txt',
                     'src-tauri/binaries/THIRD-PARTY-NOTICES.txt', '.tmp/dependency-sources.tar.gz']
        for name in materials:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b'fixture')
        media = {'version': '8.0.1', 'sourceSha256': release.sha256(source), 'binaries': {}}
        (self.root / 'scripts/media.lock.json').write_text(json.dumps(media))
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        subprocess.run(['git', 'add', 'package.json'], cwd=self.root, check=True)
        subprocess.run(['git', '-c', 'user.name=Release Test', '-c', 'user.email=test@example.invalid',
                        '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], cwd=self.root, check=True)
        (self.root / '.env').write_text('PRIVATE_TEST_MARKER=must-not-be-archived')
        output = self.root / 'out'
        release.assemble(self.root, installer, source, output, f'v{self.version}')
        checksums = (output / 'SHA256SUMS.txt').read_text().splitlines()
        self.assertEqual(len(checksums), 5)
        for line in checksums:
            digest, name = line.split('  ', 1)
            self.assertEqual(digest, release.sha256(output / name))
        with tarfile.open(output / f'x-talk_{self.version}_source.tar.gz') as archive:
            self.assertFalse(any('.env' in n for n in archive.getnames()))
            self.assertIn(f'x-talk-{self.version}/package.json', archive.getnames())
        # Signed assets must bind the immutable tag URL and participate in checksums.
        (self.root / 'docs').mkdir()
        (self.root / 'docs/RELEASE_NOTES.md').write_text('Friendly release notes.', encoding='utf-8')
        Path(str(installer) + '.sig').write_text('signature fixture', encoding='utf-8')
        signed = self.root / 'signed'
        release.assemble(self.root, installer, source, signed, f'v{self.version}', updater=True)
        feed = json.loads((signed / 'latest.json').read_text(encoding='utf-8'))
        self.assertEqual(feed['version'], self.version)
        platform = feed['platforms']['windows-x86_64']
        self.assertEqual(platform['signature'], 'signature fixture')
        self.assertEqual(platform['url'], f'https://github.com/alexander-north/x-talk/releases/download/v{self.version}/{installer.name}')
        for line in (signed / 'SHA256SUMS.txt').read_text(encoding='utf-8').splitlines():
            digest, name = line.split('  ', 1)
            self.assertEqual(digest, release.sha256(signed / name))
        Path(str(installer) + '.sig').unlink()
        with self.assertRaisesRegex(ValueError, 'signature'):
            release.assemble(self.root, installer, source, self.root / 'missing-signature', f'v{self.version}', updater=True)
        with self.assertRaisesRegex(ValueError, 'empty'):
            release.assemble(self.root, installer, source, output, f'v{self.version}')


if __name__ == '__main__':
    unittest.main()
