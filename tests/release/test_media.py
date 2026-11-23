"""The media lock pins every platform's FFmpeg tools, and the shipped notice must describe it exactly."""
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

    def test_record_pins_one_platform_and_updates_notice(self):
        for name in ['ffmpeg', 'ffprobe']:
            (self.root / 'src-tauri/binaries' / name).write_bytes(name.encode())
        windows = self.lock()['platforms']['windows-x86_64']
        media.record(self.root, 'darwin-aarch64')
        lock = self.lock()
        self.assertEqual(lock['platforms']['windows-x86_64'], windows)
        self.assertEqual(lock['platforms']['darwin-aarch64']['binaries'],
                         {n: hashlib.sha256(n.encode()).hexdigest() for n in ['ffmpeg', 'ffprobe']})
        notice = (self.root / 'src-tauri/binaries/NOTICE.txt').read_text(encoding='utf-8')
        self.assertIn(f"ffprobe: {hashlib.sha256(b'ffprobe').hexdigest()}", notice)
        self.assertIn('ffmpeg.exe: 15ab01b2', notice)
        media.verify(self.root, 'darwin-aarch64')
        (self.root / 'src-tauri/binaries/ffmpeg').write_bytes(b'rebuilt')
        with self.assertRaisesRegex(SystemExit, 'checksum mismatch'):
            media.verify(self.root, 'darwin-aarch64')

    def test_unrecorded_platform_and_stale_notice_fail_verification(self):
        lock = self.lock()
        lock['platforms']['darwin-aarch64']['binaries'] = {}
        (self.root / 'scripts/media.lock.json').write_text(json.dumps(lock), encoding='utf-8')
        with self.assertRaisesRegex(SystemExit, 'No recorded media checksums'):
            media.verify(self.root, 'darwin-aarch64')
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
