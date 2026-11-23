"""Release failures must be caught before assets reach GitHub."""
import base64
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import plistlib
import shutil
import tempfile
import subprocess
import sys
import tarfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('release', ROOT / 'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

VERSION_FILES = ['package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock',
                 'src-tauri/tauri.conf.json', 'src-tauri/tauri.windows.conf.json',
                 'src-tauri/tauri.macos.conf.json', 'src-tauri/Info.plist']
MATERIALS = ['LICENSE', 'NOTICE', 'scripts/build-media.sh', 'scripts/verify-media.py',
             'src-tauri/binaries/FFmpeg-LICENSE.txt', 'src-tauri/binaries/NOTICE.txt',
             'src-tauri/binaries/THIRD-PARTY-NOTICES.txt', '.tmp/dependency-sources.tar.gz']


def copy_version_files(root):
    for name in VERSION_FILES:
        if (ROOT / name).is_file():
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)


def committed_key_id():
    public = base64.b64decode(release.tauri_config(ROOT)['plugins']['updater']['pubkey']).decode().split('\n')
    return base64.b64decode(public[1])[2:10]


def signature(version, key_id=None, comment=None):
    """A minisign-format updater signature, structurally like `tauri signer sign` output."""
    key_id = committed_key_id() if key_id is None else key_id
    trusted = comment if comment is not None else f'timestamp:1700000000\tfile:fixture\tversion:{version}'
    text = ('untrusted comment: signature from tauri secret key\n'
            + base64.b64encode(b'ED' + key_id + bytes(64)).decode() + '\n'
            + f'trusted comment: {trusted}\n' + base64.b64encode(bytes(64)).decode() + '\n')
    return base64.b64encode(text.encode()).decode()


def add_bytes(archive, name, data, mode=0o644):
    info = tarfile.TarInfo(name)
    info.size, info.mode = len(data), mode
    archive.addfile(info, io.BytesIO(data))


def add_dir(archive, name):
    info = tarfile.TarInfo(name)
    info.type, info.mode = tarfile.DIRTYPE, 0o755
    archive.addfile(info)


def app_archive(root, path, version, plist=None, extra=None, media=None, top='x-talk.app'):
    """A minimal updater archive shaped like `tar -czf x-talk.app.tar.gz -C bundle/macos x-talk.app`."""
    config = release.tauri_config(root, 'macos')
    info = {'CFBundleIdentifier': config['identifier'], 'CFBundleShortVersionString': version,
            'CFBundleVersion': version, 'CFBundleExecutable': 'x-talk', 'CFBundlePackageType': 'APPL',
            'LSMinimumSystemVersion': release.MACOS_MINIMUM,
            'NSMicrophoneUsageDescription': 'Dictation', 'NSAudioCaptureUsageDescription': 'Meetings'}
    info.update(plist or {})
    media = {'ffmpeg': b'ffmpeg fixture', 'ffprobe': b'ffprobe fixture'} if media is None else media
    with tarfile.open(path, 'w:gz') as archive:
        for directory in ['', '/Contents', '/Contents/MacOS', '/Contents/Resources',
                          '/Contents/Resources/binaries', '/Contents/_CodeSignature']:
            add_dir(archive, top + directory)
        add_bytes(archive, f'{top}/Contents/Info.plist', plistlib.dumps(info))
        add_bytes(archive, f'{top}/Contents/MacOS/x-talk', b'\xcf\xfa\xed\xfe app', 0o755)
        add_bytes(archive, f'{top}/Contents/_CodeSignature/CodeResources', b'seal')
        for target in config['bundle']['resources'].values():
            if not target.startswith('binaries/ff'):
                add_bytes(archive, f'{top}/Contents/Resources/{target}', b'notice')
        for name, data in media.items():
            add_bytes(archive, f'{top}/Contents/Resources/binaries/{name}', data, 0o755)
        for name, data in (extra or {}).items():
            add_bytes(archive, name, data)
    return path


def disk_image(path):
    path.write_bytes(b'image data' + b'koly' + bytes(508))
    return path


class ReleaseFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        copy_version_files(self.root)
        self.version = release.version(self.root)
        self.tag = f'v{self.version}'

    def materials(self):
        """Committed release inputs, a media lock pinning both platforms' fixture tools, and a git HEAD."""
        self.source = self.root / 'ffmpeg.tar.xz'
        self.source.write_bytes(b'fixture source')
        for name in MATERIALS:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b'fixture')
        for name in ['ffmpeg.exe', 'ffprobe.exe']:
            (self.root / 'src-tauri/binaries' / name).write_bytes(name.encode())
        digest = lambda data: hashlib.sha256(data).hexdigest()
        media = {'version': '8.0.1', 'sourceSha256': release.sha256(self.source), 'platforms': {
            'windows-x86_64': {'binaries': {n: digest(n.encode()) for n in ['ffmpeg.exe', 'ffprobe.exe']}},
            'darwin-aarch64': {'binaries': {'ffmpeg': digest(b'ffmpeg fixture'),
                                            'ffprobe': digest(b'ffprobe fixture')}}}}
        (self.root / 'scripts/media.lock.json').write_text(json.dumps(media))
        (self.root / 'docs').mkdir(exist_ok=True)
        (self.root / 'docs/RELEASE_NOTES.md').write_text('Friendly release notes.', encoding='utf-8')
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        subprocess.run(['git', 'add', 'package.json'], cwd=self.root, check=True)
        subprocess.run(['git', '-c', 'user.name=Release Test', '-c', 'user.email=test@example.invalid',
                        '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], cwd=self.root, check=True)
        self.installer = self.root / f'x-talk_{self.version}_x64-setup.exe'
        self.installer.write_bytes(b'MZfixture')
        dmg_name, archive_name = release.macos_names(self.version)
        self.dmg = disk_image(self.root / dmg_name)
        self.archive = app_archive(self.root, self.root / archive_name, self.version)

    def sign(self, path, version=None):
        Path(str(path) + '.sig').write_text(signature(version or self.version), encoding='utf-8')

    def platform_sets(self):
        self.materials()
        self.sign(self.installer)
        self.sign(self.archive)
        windows, macos = self.root / 'windows', self.root / 'macos'
        release.assemble(self.root, self.installer, self.source, windows, self.tag, updater=True)
        release.package_macos(self.root, self.dmg, self.archive, macos, self.tag, updater=True)
        return windows, macos


class VersionTests(ReleaseFixture):
    def test_versions_and_matching_tag(self):
        self.assertEqual(release.version(self.root, self.tag), self.version)

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

    def test_platform_config_cannot_carry_another_version(self):
        for platform in release.CONFIG_PLATFORMS:
            with self.subTest(platform=platform):
                path = self.root / f'src-tauri/tauri.{platform}.conf.json'
                original = path.read_text(encoding='utf-8') if path.exists() else None
                config = json.loads(original or '{}')
                config['version'] = '0.0.1'
                path.write_text(json.dumps(config))
                with self.assertRaisesRegex(ValueError, 'disagree'):
                    release.version(self.root)
                config['version'] = self.version
                path.write_text(json.dumps(config))
                self.assertEqual(release.version(self.root), self.version)
                config.pop('version')
                config.setdefault('bundle', {}).setdefault('macOS', {})['bundleVersion'] = '1'
                path.write_text(json.dumps(config))
                with self.assertRaisesRegex(ValueError, 'disagree'):
                    release.version(self.root)
                if original is None:
                    path.unlink()
                else:
                    path.write_text(original, encoding='utf-8')

    def test_info_plist_cannot_pin_the_bundle_version(self):
        path = self.root / 'src-tauri/Info.plist'
        path.write_bytes(plistlib.dumps({'NSMicrophoneUsageDescription': 'x', 'CFBundleVersion': '1.0.0'}))
        with self.assertRaisesRegex(ValueError, 'Info.plist'):
            release.version(self.root)


class ConfigSplitTests(unittest.TestCase):
    def test_merge_patch_follows_rfc_7396(self):
        base = {'a': {'b': 1, 'c': [1, 2]}, 'd': 1, 'e': {'f': 1}}
        patch = {'a': {'c': [3], 'g': None, 'h': {'i': 1}}, 'd': None, 'e': 2}
        self.assertEqual(release.merge_patch(base, patch), {'a': {'b': 1, 'c': [3], 'h': {'i': 1}}, 'e': 2})
        self.assertEqual(base['a']['c'], [1, 2], 'the base document must not be modified')

    def test_platform_views_bundle_the_right_media_and_targets(self):
        windows = release.tauri_config(ROOT, 'windows')
        macos = release.tauri_config(ROOT, 'macos')
        self.assertEqual(windows['bundle']['targets'], ['nsis'])
        self.assertIn('binaries/ffmpeg.exe', windows['bundle']['resources'])
        self.assertNotIn('binaries/ffmpeg', windows['bundle']['resources'])
        self.assertTrue({'app', 'dmg'} <= set(macos['bundle']['targets']))
        self.assertNotIn('nsis', macos['bundle']['targets'])
        self.assertEqual(macos['bundle']['resources']['binaries/ffmpeg'], 'binaries/ffmpeg')
        self.assertEqual(macos['bundle']['resources']['binaries/ffprobe'], 'binaries/ffprobe')
        self.assertNotIn('binaries/ffmpeg.exe', macos['bundle']['resources'])
        self.assertEqual(macos['bundle']['macOS']['minimumSystemVersion'], release.MACOS_MINIMUM)
        self.assertEqual(macos['bundle']['macOS']['signingIdentity'], '-')
        for config in (windows, macos):
            # Shared license material ships on both platforms.
            for target in ['LICENSE', 'NOTICE', 'binaries/NOTICE.txt', 'binaries/FFmpeg-LICENSE.txt',
                           'binaries/THIRD-PARTY-NOTICES.txt']:
                self.assertIn(target, config['bundle']['resources'].values())

    def test_ci_builds_never_need_the_updater_key(self):
        # createUpdaterArtifacts makes `tauri build` require TAURI_SIGNING_PRIVATE_KEY; CI signs separately.
        for platform in release.CONFIG_PLATFORMS:
            with self.subTest(platform=platform):
                self.assertFalse(release.tauri_config(ROOT, platform)['bundle'].get('createUpdaterArtifacts', False))
        self.assertEqual(release.updater_pubkey(ROOT, 'windows'), release.updater_pubkey(ROOT, 'macos'))


class SignatureTests(unittest.TestCase):
    def setUp(self):
        self.pubkey = release.tauri_config(ROOT)['plugins']['updater']['pubkey']

    def test_bound_signature_with_committed_key_passes(self):
        release.check_signature(signature('1.8.42'), self.pubkey, '1.8.42')

    def test_unbound_wrong_key_and_malformed_signatures_fail(self):
        cases = [
            ('not bound', signature('1.8.41')),
            ('not bound', signature('1.8.4')),
            ('not bound', signature('1.8.42', comment='timestamp:1\tfile:x')),
            ('key does not match', signature('1.8.42', key_id=bytes(8))),
            ('Malformed', 'not base64 !'),
            ('Malformed', base64.b64encode(b'untrusted comment: x\nshort').decode()),
        ]
        for index, (message, text) in enumerate(cases):
            with self.subTest(case=index), self.assertRaisesRegex(ValueError, message):
                release.check_signature(text, self.pubkey, '1.8.42')


class WindowsPackagingTests(ReleaseFixture):
    def test_wrong_installer_cannot_be_packaged(self):
        with self.assertRaisesRegex(ValueError, 'Expected installer'):
            release.assemble(self.root, self.root / 'old.exe', self.root / 'source', self.root / 'out', self.tag)

    def test_corrupt_installer_is_rejected(self):
        installer = self.root / f'x-talk_{self.version}_x64-setup.exe'
        installer.write_bytes(b'not a PE file')
        with self.assertRaisesRegex(ValueError, 'Windows executable'):
            release.assemble(self.root, installer, self.root / 'source', self.root / 'out', self.tag)

    def test_source_checksum_failure_prevents_output(self):
        installer = self.root / f'x-talk_{self.version}_x64-setup.exe'
        installer.write_bytes(b'MZfixture')
        source = self.root / 'source'
        source.write_bytes(b'wrong source')
        (self.root / 'scripts').mkdir()
        shutil.copyfile(ROOT / 'scripts/media.lock.json', self.root / 'scripts/media.lock.json')
        with self.assertRaisesRegex(ValueError, 'source checksum'):
            release.assemble(self.root, installer, source, self.root / 'out', self.tag)
        self.assertFalse((self.root / 'out').exists())

    def test_complete_asset_set_has_verified_checksums_and_excludes_local_secrets(self):
        self.materials()
        (self.root / '.env').write_text('PRIVATE_TEST_MARKER=must-not-be-archived')
        output = self.root / 'out'
        release.assemble(self.root, self.installer, self.source, output, self.tag)
        self.assertEqual(release.verified_files(output, 'Windows'),
                         release.shared_names(self.version, {'version': '8.0.1'}) | {self.installer.name})
        with tarfile.open(output / f'x-talk_{self.version}_source.tar.gz') as archive:
            self.assertFalse(any('.env' in n for n in archive.getnames()))
            self.assertIn(f'x-talk-{self.version}/package.json', archive.getnames())
        self.assertFalse((output / 'latest.json').exists(), 'the feed is written only for the combined release')
        self.sign(self.installer)
        signed = self.root / 'signed'
        release.assemble(self.root, self.installer, self.source, signed, self.tag, updater=True)
        self.assertIn(self.installer.name + '.sig', release.verified_files(signed, 'Windows'))
        Path(str(self.installer) + '.sig').unlink()
        with self.assertRaisesRegex(ValueError, 'signature'):
            release.assemble(self.root, self.installer, self.source, self.root / 'unsigned', self.tag, updater=True)
        self.assertFalse((self.root / 'unsigned').exists())
        with self.assertRaisesRegex(ValueError, 'empty'):
            release.assemble(self.root, self.installer, self.source, output, self.tag)

    def test_unrecorded_media_checksums_block_packaging(self):
        self.materials()
        lock = json.loads((self.root / 'scripts/media.lock.json').read_text())
        lock['platforms']['windows-x86_64']['binaries'] = {}
        (self.root / 'scripts/media.lock.json').write_text(json.dumps(lock))
        with self.assertRaisesRegex(ValueError, 'No recorded media checksums'):
            release.assemble(self.root, self.installer, self.source, self.root / 'out', self.tag)


class MacPackagingTests(ReleaseFixture):
    def setUp(self):
        super().setUp()
        self.materials()

    def package(self, archive=None, dmg=None, updater=False, output='mac'):
        return release.package_macos(self.root, dmg or self.dmg, archive or self.archive,
                                     self.root / output, self.tag, updater)

    def test_valid_bundle_is_packaged_with_checksums(self):
        self.package()
        self.assertEqual(release.verified_files(self.root / 'mac', 'macOS'), {self.dmg.name, self.archive.name})
        self.sign(self.archive)
        self.package(updater=True, output='signed')
        self.assertIn(self.archive.name + '.sig', release.verified_files(self.root / 'signed', 'macOS'))

    def test_tauri_default_names_are_rejected(self):
        plain = self.root / 'x-talk.app.tar.gz'
        shutil.copyfile(self.archive, plain)
        with self.assertRaisesRegex(ValueError, 'Expected updater archive'):
            self.package(archive=plain)
        x64 = self.root / f'x-talk_{self.version}_x64.dmg'
        shutil.copyfile(self.dmg, x64)
        with self.assertRaisesRegex(ValueError, 'Expected disk image'):
            self.package(dmg=x64)

    def test_invalid_disk_image_is_rejected(self):
        self.dmg.write_bytes(b'not a disk image' * 64)
        with self.assertRaisesRegex(ValueError, 'UDIF'):
            self.package()

    def test_archive_must_unpack_as_one_app_bundle(self):
        cases = {
            'outside': {'extra': {'other/file': b'x'}},
            'metadata': {'extra': {'x-talk.app/Contents/._Info.plist': b'x'}},
            'Media checksum': {'media': {'ffmpeg': b'homebrew ffmpeg', 'ffprobe': b'ffprobe fixture'}},
            'CFBundleShortVersionString': {'plist': {'CFBundleShortVersionString': '0.0.1'}},
            'LSMinimumSystemVersion': {'plist': {'LSMinimumSystemVersion': '11.0'}},
            'NSAudioCaptureUsageDescription': {'plist': {'NSAudioCaptureUsageDescription': ''}},
        }
        for index, (message, options) in enumerate(cases.items()):
            with self.subTest(message=message):
                (self.root / f'case{index}').mkdir()
                archive = app_archive(self.root, self.root / f'case{index}' / self.archive.name, self.version,
                                      **options)
                with self.assertRaisesRegex(ValueError, message):
                    self.package(archive=archive, output=f'out{index}')
                self.assertFalse((self.root / f'out{index}').exists())
        (self.root / 'nested').mkdir()
        dotted = self.root / 'nested' / self.archive.name
        with tarfile.open(dotted, 'w:gz') as archive:
            add_dir(archive, './x-talk.app')
        with self.assertRaisesRegex(ValueError, 'outside'):
            self.package(archive=dotted)

    def test_signed_package_requires_signature(self):
        with self.assertRaisesRegex(ValueError, 'signature'):
            self.package(updater=True)


class PublishSetTests(ReleaseFixture):
    def test_two_platform_release_has_one_feed_and_complete_checksums(self):
        windows, macos = self.platform_sets()
        output = self.root / 'release'
        release.publish_set(self.root, self.tag, windows, macos, output)
        feed = json.loads((output / 'latest.json').read_text(encoding='utf-8'))
        self.assertEqual(feed['version'], self.version)
        self.assertEqual(feed['notes'], 'Friendly release notes.')
        self.assertEqual(set(feed['platforms']), {'windows-x86_64', 'darwin-aarch64'})
        base = f'https://github.com/alexander-north/x-talk/releases/download/{self.tag}'
        self.assertEqual(feed['platforms']['windows-x86_64'],
                         {'url': f'{base}/{self.installer.name}',
                          'signature': (self.root / (self.installer.name + '.sig')).read_text().strip()})
        self.assertEqual(feed['platforms']['darwin-aarch64'],
                         {'url': f'{base}/{self.archive.name}',
                          'signature': (self.root / (self.archive.name + '.sig')).read_text().strip()})
        for platform in feed['platforms'].values():
            self.assertTrue((output / platform['url'].rsplit('/', 1)[1]).is_file())
        # Every published file, including the feed, is covered exactly once.
        listed = release.verified_files(output, 'release')
        self.assertEqual(listed, {p.name for p in output.iterdir()} - {'SHA256SUMS.txt'})
        self.assertEqual(listed, release.verified_files(windows, 'Windows') | release.verified_files(macos, 'macOS')
                         | {'latest.json'})
        lines = (output / 'SHA256SUMS.txt').read_text(encoding='utf-8').splitlines()
        self.assertEqual(len(lines), len(listed))
        with self.assertRaisesRegex(ValueError, 'empty'):
            release.publish_set(self.root, self.tag, windows, macos, output)

    def test_missing_platform_publishes_nothing(self):
        windows, macos = self.platform_sets()
        output = self.root / 'release'
        with self.assertRaisesRegex(ValueError, 'Missing macOS'):
            release.publish_set(self.root, self.tag, windows, self.root / 'absent', output)
        with self.assertRaisesRegex(ValueError, 'Missing Windows'):
            release.publish_set(self.root, self.tag, self.root / 'absent', macos, output)
        (macos / (self.archive.name + '.sig')).unlink()
        release.write_checksums(macos)
        with self.assertRaisesRegex(ValueError, r'Incomplete macOS.*\.sig'):
            release.publish_set(self.root, self.tag, windows, macos, output)
        self.assertFalse(output.exists())

    def test_unsigned_platform_set_cannot_publish(self):
        self.materials()
        windows, macos = self.root / 'windows', self.root / 'macos'
        release.assemble(self.root, self.installer, self.source, windows, self.tag)
        self.sign(self.archive)
        release.package_macos(self.root, self.dmg, self.archive, macos, self.tag, updater=True)
        with self.assertRaisesRegex(ValueError, r'Incomplete Windows.*setup\.exe\.sig'):
            release.publish_set(self.root, self.tag, windows, macos, self.root / 'release')

    def test_tampered_extra_or_misbound_assets_are_rejected(self):
        windows, macos = self.platform_sets()
        output = self.root / 'release'
        dmg = macos / self.dmg.name
        original = dmg.read_bytes()
        dmg.write_bytes(original + b'tampered')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            release.publish_set(self.root, self.tag, windows, macos, output)
        dmg.write_bytes(original)
        (macos / 'notes.txt').write_text('stray')
        with self.assertRaisesRegex(ValueError, 'does not match its files'):
            release.publish_set(self.root, self.tag, windows, macos, output)
        release.write_checksums(macos)
        with self.assertRaisesRegex(ValueError, r"unexpected \['notes.txt'\]"):
            release.publish_set(self.root, self.tag, windows, macos, output)
        (macos / 'notes.txt').unlink()
        (macos / (self.archive.name + '.sig')).write_text(signature('0.0.1'))
        release.write_checksums(macos)
        with self.assertRaisesRegex(ValueError, 'not bound'):
            release.publish_set(self.root, self.tag, windows, macos, output)
        self.assertFalse(output.exists())

    def test_assets_from_another_version_are_rejected(self):
        windows, macos = self.platform_sets()
        package = json.loads((self.root / 'package.json').read_text())
        with self.assertRaisesRegex(ValueError, 'Expected tag'):
            release.publish_set(self.root, 'v0.0.1', windows, macos, self.root / 'release')
        self.assertEqual(package['version'], self.version)


class CommandLineTests(unittest.TestCase):
    def test_version_check_and_signature_commands(self):
        run = lambda *args: subprocess.run([sys.executable, str(ROOT / 'scripts/release.py'), *args],
                                           capture_output=True, text=True)
        self.assertEqual(run().returncode, 0)
        self.assertNotEqual(run('--tag', 'v0.0.0').returncode, 0)
        version = release.version()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'fixture.sig'
            path.write_text(signature(version))
            for platform in release.CONFIG_PLATFORMS:
                result = run('check-signature', '--platform', platform, '--version', version, str(path))
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotEqual(run('check-signature', '--platform', 'macos', '--version', '9.9.9', str(path)).returncode, 0)


if __name__ == '__main__':
    unittest.main()
