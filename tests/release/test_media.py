"""The media lock pins the source and the Windows tools; macOS tools are verified per build. The shipped notice must match."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('verify_media', ROOT / 'scripts/verify-media.py')
media = importlib.util.module_from_spec(spec)
spec.loader.exec_module(media)


class MediaLockTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        (self.root / 'src-tauri/binaries').mkdir(parents=True)
        shutil.copyfile(ROOT / 'scripts/media.lock.json', self.root / 'scripts/media.lock.json')
        shutil.copyfile(ROOT / 'src-tauri/binaries/NOTICE.txt', self.root / 'src-tauri/binaries/NOTICE.txt')

    def lock(self):
        return json.loads((self.root / 'scripts/media.lock.json').read_text(encoding='utf-8'))

    def test_committed_notice_matches_lock(self):
        lock = json.loads((ROOT / 'scripts/media.lock.json').read_text(encoding='utf-8'))
        notice = (ROOT / 'src-tauri/binaries/NOTICE.txt').read_text(encoding='utf-8')
        self.assertEqual(notice, media.render_notice(lock))
        self.assertEqual(set(lock['platforms']), set(media.PLATFORMS))
        # The Windows tools keep their pinned build; this change must not alter them.
        self.assertEqual(lock['platforms']['windows-x86_64']['binaries'], {
            'ffmpeg.exe': '15ab01b2a95b529bb032841425f96510d85f8da807233a55a4a6f7b460c6d5fa',
            'ffprobe.exe': '93148b7a0537924be062920d278e1b092a91cb0f249c8929c6348bb61b8064e7'})
        self.assertIn(lock['sourceSha256'], notice)
        # macOS tools are not pinned in git: each build is verified and published with its release.
        self.assertNotIn('binaries', lock['platforms']['darwin-aarch64'])
        self.assertIn('_aarch64_ffmpeg-build.json', notice)

    def test_record_pins_windows_and_refuses_per_build_macos(self):
        for name in ['ffmpeg.exe', 'ffprobe.exe']:
            (self.root / 'src-tauri/binaries' / name).write_bytes(name.encode())
        darwin = self.lock()['platforms']['darwin-aarch64']
        media.record(self.root, 'windows-x86_64')
        lock = self.lock()
        self.assertEqual(lock['platforms']['darwin-aarch64'], darwin)
        self.assertEqual(lock['platforms']['windows-x86_64']['binaries'],
                         {n: hashlib.sha256(n.encode()).hexdigest() for n in ['ffmpeg.exe', 'ffprobe.exe']})
        notice = (self.root / 'src-tauri/binaries/NOTICE.txt').read_text(encoding='utf-8')
        self.assertIn(f"ffprobe.exe: {hashlib.sha256(b'ffprobe.exe').hexdigest()}", notice)
        media.verify(self.root, 'windows-x86_64')
        (self.root / 'src-tauri/binaries/ffmpeg.exe').write_bytes(b'rebuilt')
        with self.assertRaisesRegex(SystemExit, 'checksum mismatch'):
            media.verify(self.root, 'windows-x86_64')
        with self.assertRaisesRegex(SystemExit, 'not pinned'):
            media.record(self.root, 'darwin-aarch64')

    def test_macos_tools_must_be_the_ones_the_current_recipe_built(self):
        shutil.copyfile(ROOT / 'scripts/build-media.sh', self.root / 'scripts/build-media.sh')
        with self.assertRaisesRegex(SystemExit, 'No media build'):
            media.verify(self.root, 'darwin-aarch64')
        for name in ['ffmpeg', 'ffprobe']:
            (self.root / 'src-tauri/binaries' / name).write_bytes(name.encode())
        with self.assertRaisesRegex(SystemExit, 'No media build'):
            media.verify(self.root, 'darwin-aarch64')
        media.write_manifest(self.root, 'darwin-aarch64', 'Xcode 26.1 Build 17B55')
        manifest = json.loads((self.root / media.MANIFEST).read_text(encoding='utf-8'))
        self.assertEqual(manifest['binaries'], {n: hashlib.sha256(n.encode()).hexdigest() for n in ['ffmpeg', 'ffprobe']})
        self.assertEqual(manifest['sourceSha256'], self.lock()['sourceSha256'])
        self.assertEqual(manifest['toolchain'], 'Xcode 26.1 Build 17B55')
        media.verify(self.root, 'darwin-aarch64')
        (self.root / 'src-tauri/binaries/ffmpeg').write_bytes(b'homebrew ffmpeg')
        with self.assertRaisesRegex(SystemExit, 'checksum mismatch'):
            media.verify(self.root, 'darwin-aarch64')
        media.write_manifest(self.root, 'darwin-aarch64', 'any')
        media.verify(self.root, 'darwin-aarch64')
        with (self.root / 'scripts/build-media.sh').open('a', encoding='utf-8') as recipe:
            recipe.write('# changed recipe\n')
        with self.assertRaisesRegex(SystemExit, 'another version of scripts/build-media.sh'):
            media.verify(self.root, 'darwin-aarch64')
        media.write_manifest(self.root, 'darwin-aarch64', 'any')
        manifest = json.loads((self.root / media.MANIFEST).read_text(encoding='utf-8'))
        manifest['sourceSha256'] = '0' * 64
        (self.root / media.MANIFEST).write_text(json.dumps(manifest), encoding='utf-8')
        with self.assertRaisesRegex(SystemExit, 'another FFmpeg source'):
            media.verify(self.root, 'darwin-aarch64')
        with self.assertRaisesRegex(SystemExit, 'pinned'):
            media.write_manifest(self.root, 'windows-x86_64', 'any')

    def test_unrecorded_platform_and_stale_notice_fail_verification(self):
        lock = self.lock()
        lock['platforms']['windows-x86_64']['binaries'] = {}
        (self.root / 'scripts/media.lock.json').write_text(json.dumps(lock), encoding='utf-8')
        with self.assertRaisesRegex(SystemExit, 'No recorded media checksums'):
            media.verify(self.root, 'windows-x86_64')
        for name in ['ffmpeg.exe', 'ffprobe.exe']:
            (self.root / 'src-tauri/binaries' / name).write_bytes(name.encode())
        lock['platforms']['windows-x86_64']['binaries'] = media.checksums(self.root, 'windows-x86_64')
        (self.root / 'scripts/media.lock.json').write_text(json.dumps(lock), encoding='utf-8')
        with self.assertRaisesRegex(SystemExit, 'NOTICE.txt is out of date'):
            media.verify(self.root, 'windows-x86_64')
        (self.root / 'src-tauri/binaries/NOTICE.txt').write_text(media.render_notice(lock), encoding='utf-8')
        media.verify(self.root, 'windows-x86_64')

    def test_node_and_python_verifiers_agree_on_platform_tools(self):
        script = (ROOT / 'scripts/verify-media.mjs').read_text(encoding='utf-8')
        for platform, names in media.PLATFORMS.items():
            self.assertIn(f"'{platform}': [{', '.join(repr(n) for n in names)}]", script)


if __name__ == '__main__':
    unittest.main()
