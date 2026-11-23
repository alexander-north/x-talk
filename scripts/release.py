"""Validate versions and assemble reviewed release assets; never publishes anything.

  release.py [--tag TAG]                  check that every version file agrees (and matches TAG)
  release.py windows --tag TAG --installer EXE --ffmpeg-source ARCHIVE --output DIR [--updater]
  release.py macos --tag TAG --dmg DMG --app-archive TAR_GZ --output DIR [--updater]
  release.py publish-set --tag TAG --windows DIR --macos DIR --output DIR
  release.py check-signature --platform {windows,macos} --version VERSION SIGNATURE

The windows and macos commands package and validate one platform's CI output. publish-set combines
both platforms into the one release that is published: it fails unless both are complete, so a
release (and its update feed) always covers every platform.
"""
import argparse
import base64
import binascii
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import plistlib
import posixpath
import re
import shutil
import subprocess
import sys
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent
DOWNLOAD_URL = 'https://github.com/alexander-north/x-talk/releases/download'
CONFIG_PLATFORMS = ('windows', 'macos')
MACOS_MINIMUM = '14.2'
UPDATER_TARGETS = {'windows': 'windows-x86_64', 'macos': 'darwin-aarch64'}
MEDIA_PLATFORMS = {'windows': 'windows-x86_64', 'macos': 'darwin-aarch64'}
LICENSE_MATERIALS = ['LICENSE', 'NOTICE', 'scripts/build-media.sh', 'scripts/media.lock.json',
                     'scripts/verify-media.py', 'src-tauri/binaries/FFmpeg-LICENSE.txt',
                     'src-tauri/binaries/NOTICE.txt', 'src-tauri/binaries/THIRD-PARTY-NOTICES.txt']
# Keys Tauri derives from tauri.conf.json; Info.plist must not pin them to a stale release.
PLIST_VERSION_KEYS = ('CFBundleShortVersionString', 'CFBundleVersion')


def merge_patch(target, patch):
    """RFC 7396 JSON merge patch, as Tauri applies tauri.<platform>.conf.json over tauri.conf.json."""
    if not isinstance(patch, dict):
        return patch
    result = dict(target) if isinstance(target, dict) else {}
    for key, value in patch.items():
        if value is None:
            result.pop(key, None)
        else:
            result[key] = merge_patch(result.get(key), value)
    return result


def tauri_config(root=ROOT, platform=None):
    """The configuration Tauri uses on a platform: the base file overlaid with its platform file."""
    config = json.loads((root / 'src-tauri/tauri.conf.json').read_text(encoding='utf-8'))
    if platform is not None:
        overlay = root / f'src-tauri/tauri.{platform}.conf.json'
        if overlay.is_file():
            config = merge_patch(config, json.loads(overlay.read_text(encoding='utf-8')))
    return config


def version(root=ROOT, tag=None):
    package = json.loads((root / 'package.json').read_text(encoding='utf-8'))
    lock = json.loads((root / 'package-lock.json').read_text(encoding='utf-8'))
    cargo = tomllib.loads((root / 'src-tauri/Cargo.toml').read_text(encoding='utf-8'))
    cargo_lock = tomllib.loads((root / 'src-tauri/Cargo.lock').read_text(encoding='utf-8'))
    versions = {'package.json': package['version'], 'package-lock.json': lock['version'],
                'package-lock.json packages[""]': lock['packages']['']['version'],
                'src-tauri/Cargo.toml': cargo['package']['version']}
    for platform in CONFIG_PLATFORMS:
        config = tauri_config(root, platform)
        versions[f'tauri config ({platform})'] = config.get('version')
        # CFBundleVersion defaults to the version; a fixed override would never be stamped.
        bundle_version = config.get('bundle', {}).get('macOS', {}).get('bundleVersion')
        if bundle_version is not None:
            versions[f'bundle.macOS.bundleVersion ({platform})'] = bundle_version
    applications = [p['version'] for p in cargo_lock['package'] if p['name'] == 'x-talk']
    if len(applications) != 1:
        raise ValueError('Expected one x-talk package in src-tauri/Cargo.lock')
    versions['src-tauri/Cargo.lock'] = applications[0]
    if len(set(versions.values())) != 1:
        raise ValueError(f'Package versions disagree: {versions}')
    plist = root / 'src-tauri/Info.plist'
    if plist.is_file() and any(key in plistlib.loads(plist.read_bytes()) for key in PLIST_VERSION_KEYS):
        raise ValueError('src-tauri/Info.plist must not set the bundle version; Tauri takes it from tauri.conf.json')
    current = package['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+', current):
        raise ValueError('Release version must be major.minor.patch')
    if tag is not None and tag != f'v{current}':
        raise ValueError(f'Expected tag v{current}, got {tag!r}')
    if package.get('license') != 'Apache-2.0' or cargo['package'].get('license') != 'Apache-2.0':
        raise ValueError('Missing application license metadata')
    return current


def sha256(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def windows_installer_name(current):
    return f'x-talk_{current}_x64-setup.exe'


def macos_names(current):
    return f'x-talk_{current}_aarch64.dmg', f'x-talk_{current}_aarch64.app.tar.gz'


def shared_names(current, media):
    return {f'x-talk_{current}_source.tar.gz', f'x-talk_{current}_dependency-sources.tar.gz',
            f'x-talk_{current}_licenses-and-build.tar.gz', f"ffmpeg-{media['version']}.tar.xz"}


def media_lock(root):
    return json.loads((root / 'scripts/media.lock.json').read_text(encoding='utf-8'))


def media_binaries(media, platform):
    binaries = media['platforms'][MEDIA_PLATFORMS[platform]].get('binaries') or {}
    if not binaries:
        raise ValueError(f'No recorded media checksums for {MEDIA_PLATFORMS[platform]}')
    return binaries


def updater_pubkey(root, platform):
    return tauri_config(root, platform)['plugins']['updater']['pubkey']


def check_signature(signature, pubkey, current):
    """Structural check that a Tauri updater signature uses the committed key and is bound to `current`.

    The client verifies the cryptographic signature; this catches a wrong key or a missing
    `--app-version` (rejected by requireSignedVersion) before anything is published.
    """
    def decode(text):
        try:
            return base64.b64decode(text.strip(), validate=True).decode('utf-8').split('\n')
        except (binascii.Error, UnicodeDecodeError, ValueError) as error:
            raise ValueError('Malformed updater signature') from error

    def key_id(line):
        try:
            return base64.b64decode(line.strip(), validate=True)[2:10]
        except (binascii.Error, ValueError) as error:
            raise ValueError('Malformed updater signature') from error

    lines, public = decode(signature), decode(pubkey)
    if len(lines) < 4 or not lines[0].startswith('untrusted comment:') or not lines[2].startswith('trusted comment: '):
        raise ValueError('Malformed updater signature')
    if len(public) < 2 or key_id(lines[1]) != key_id(public[1]) or len(key_id(lines[1])) != 8:
        raise ValueError('Updater signature key does not match the committed public key')
    if f'version:{current}' not in lines[2][len('trusted comment: '):].split('\t'):
        raise ValueError(f'Updater signature is not bound to version {current}')


def read_signature(path):
    if not path.is_file() or not path.read_text(encoding='utf-8').strip():
        raise ValueError(f'Signed release requires an updater signature: {path.name}')
    return path.read_text(encoding='utf-8').strip()


def write_checksums(output):
    files = sorted(p for p in output.iterdir() if p.is_file() and p.name != 'SHA256SUMS.txt')
    (output / 'SHA256SUMS.txt').write_text(''.join(f'{sha256(p)}  {p.name}\n' for p in files), encoding='utf-8')


def verified_files(directory, label):
    """Names in a platform asset directory, after checking its SHA256SUMS.txt covers exactly them."""
    sums = directory / 'SHA256SUMS.txt'
    if not directory.is_dir() or not sums.is_file():
        raise ValueError(f'Missing {label} release assets')
    listed = {}
    for line in sums.read_text(encoding='utf-8').splitlines():
        digest, separator, name = line.partition('  ')
        if not separator or '/' in name or '\\' in name or name in listed:
            raise ValueError(f'Malformed {label} checksum list')
        listed[name] = digest
    present = {p.name for p in directory.iterdir() if p.name != 'SHA256SUMS.txt'}
    if set(listed) != present or not all((directory / n).is_file() for n in present):
        raise ValueError(f'{label} checksum list does not match its files')
    for name, digest in listed.items():
        if sha256(directory / name) != digest:
            raise ValueError(f'{label} asset checksum mismatch: {name}')
    return present


def require_empty(output):
    if output.exists() and any(output.iterdir()):
        raise ValueError('Output directory must be empty; refusing stale release assets')


def check_installer(installer):
    with installer.open('rb') as handle:
        if handle.read(2) != b'MZ':
            raise ValueError('Installer is not a Windows executable')


def check_dmg(dmg):
    # UDIF disk images end with a 512-byte "koly" trailer.
    with dmg.open('rb') as handle:
        handle.seek(0, 2)
        if handle.tell() < 512:
            raise ValueError('Disk image is not a macOS UDIF image')
        handle.seek(-512, 2)
        if handle.read(4) != b'koly':
            raise ValueError('Disk image is not a macOS UDIF image')


def check_app_archive(root, archive, current):
    """Validate the macOS updater archive the way tauri-plugin-updater will unpack it.

    The plugin drops the first path component of every entry and installs the rest as the app
    bundle, so every entry must live under one `<productName>.app` directory.
    """
    config = tauri_config(root, 'macos')
    top = f"{config['productName']}.app"
    media = media_binaries(media_lock(root), 'macos')
    with tarfile.open(archive, 'r:gz') as bundle:
        members = {}
        for member in bundle.getmembers():
            name = member.name.rstrip('/')
            parts = name.split('/')
            if name.startswith('/') or parts[0] != top or any(p in ('', '.', '..') for p in parts):
                raise ValueError(f'Updater archive entry outside {top}: {member.name}')
            if parts[-1].startswith('._'):
                raise ValueError(f'Updater archive contains macOS metadata files: {member.name}')
            if not (member.isfile() or member.isdir() or member.issym()):
                raise ValueError(f'Unsupported updater archive entry: {member.name}')
            if member.issym():
                resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), member.linkname))
                if member.linkname.startswith('/') or not resolved.startswith(top + '/'):
                    raise ValueError(f'Updater archive link leaves the bundle: {member.name}')
            members[name] = member

        def data(name):
            member = members.get(name)
            if member is None or not member.isfile():
                raise ValueError(f'Updater archive is missing {name}')
            return bundle.extractfile(member).read()

        info = plistlib.loads(data(f'{top}/Contents/Info.plist'))
        expected = {'CFBundleIdentifier': config['identifier'], 'CFBundleShortVersionString': current,
                    'LSMinimumSystemVersion': MACOS_MINIMUM}
        for key, value in expected.items():
            if info.get(key) != value:
                raise ValueError(f'Info.plist {key} is {info.get(key)!r}, expected {value!r}')
        for key in ('NSMicrophoneUsageDescription', 'NSAudioCaptureUsageDescription'):
            if not str(info.get(key, '')).strip():
                raise ValueError(f'Info.plist is missing {key}')
        executable = f"{top}/Contents/MacOS/{info.get('CFBundleExecutable')}"
        if not data(executable) or not members[executable].mode & 0o111:
            raise ValueError('Updater archive app executable is missing or not executable')
        data(f'{top}/Contents/_CodeSignature/CodeResources')
        resources = config['bundle'].get('resources', {})
        if not isinstance(resources, dict):
            raise ValueError('Expected bundle.resources to be a source-to-target map')
        for target in resources.values():
            if not data(f'{top}/Contents/Resources/{target}'):
                raise ValueError(f'Bundled resource is empty: {target}')
        for name, checksum in media.items():
            path = f'{top}/Contents/Resources/binaries/{name}'
            if hashlib.sha256(data(path)).hexdigest() != checksum:
                raise ValueError(f'Media checksum mismatch in app bundle: {name}')
            if not members[path].mode & 0o111:
                raise ValueError(f'Bundled media tool is not executable: {name}')


def assemble(root, installer, source, output, tag, updater=False):
    """Windows platform assets plus the platform-independent source, license and FFmpeg assets."""
    current = version(root, tag)
    expected = windows_installer_name(current)
    if installer.name != expected or not installer.is_file():
        raise ValueError(f'Expected installer {expected}')
    check_installer(installer)
    media = media_lock(root)
    if sha256(source) != media['sourceSha256']:
        raise ValueError('FFmpeg source checksum mismatch')
    for name, checksum in media_binaries(media, 'windows').items():
        if sha256(root / 'src-tauri/binaries' / name) != checksum:
            raise ValueError(f'Media checksum mismatch: {name}')
    for name in LICENSE_MATERIALS:
        if not (root / name).is_file() or (root / name).stat().st_size == 0:
            raise ValueError(f'Missing release material: {name}')
    dependency_sources = root / '.tmp/dependency-sources.tar.gz'
    if not dependency_sources.is_file():
        raise ValueError('Generate dependency notices and sources before packaging')
    signature = Path(str(installer) + '.sig')
    if updater:
        read_signature(signature)
    require_empty(output)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(installer, output / expected)
    if updater:
        shutil.copy2(signature, output / signature.name)
    shutil.copy2(dependency_sources, output / f'x-talk_{current}_dependency-sources.tar.gz')
    shutil.copy2(source, output / f"ffmpeg-{media['version']}.tar.xz")
    # Preserve recipe paths so the included build script resolves its repository root.
    with tarfile.open(output / f'x-talk_{current}_licenses-and-build.tar.gz', 'w:gz') as archive:
        for name in LICENSE_MATERIALS:
            archive.add(root / name, arcname=name)
    # Git archives exclude untracked local data, .env, dependencies and build products.
    subprocess.run(['git', 'archive', '--format=tar.gz', f'--prefix=x-talk-{current}/',
                    '-o', str((output / f'x-talk_{current}_source.tar.gz').resolve()), 'HEAD'],
                   cwd=root, check=True)
    write_checksums(output)
    return current


def package_macos(root, dmg, app_archive, output, tag, updater=False):
    """macOS platform assets: the disk image, the updater archive and its signature."""
    current = version(root, tag)
    dmg_name, archive_name = macos_names(current)
    if dmg.name != dmg_name or not dmg.is_file():
        raise ValueError(f'Expected disk image {dmg_name}')
    if app_archive.name != archive_name or not app_archive.is_file():
        raise ValueError(f'Expected updater archive {archive_name}')
    check_dmg(dmg)
    check_app_archive(root, app_archive, current)
    signature = Path(str(app_archive) + '.sig')
    if updater:
        read_signature(signature)
    require_empty(output)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(dmg, output / dmg_name)
    shutil.copy2(app_archive, output / archive_name)
    if updater:
        shutil.copy2(signature, output / signature.name)
    write_checksums(output)
    return current


def publish_set(root, tag, windows, macos, output):
    """Combine both platforms into the published asset set with one update feed for both."""
    current = version(root, tag)
    media = media_lock(root)
    installer = windows_installer_name(current)
    dmg_name, archive_name = macos_names(current)
    expected = {
        'Windows': (windows, shared_names(current, media) | {installer, installer + '.sig'}),
        'macOS': (macos, {dmg_name, archive_name, archive_name + '.sig'}),
    }
    for label, (directory, names) in expected.items():
        present = verified_files(directory, label)
        if present != names:
            missing, unexpected = sorted(names - present), sorted(present - names)
            raise ValueError(f'Incomplete {label} release assets; missing {missing}, unexpected {unexpected}')
    check_installer(windows / installer)
    check_dmg(macos / dmg_name)
    check_app_archive(root, macos / archive_name, current)
    if sha256(windows / f"ffmpeg-{media['version']}.tar.xz") != media['sourceSha256']:
        raise ValueError('FFmpeg source checksum mismatch')
    platforms = {}
    for platform, directory, name in [('windows', windows, installer), ('macos', macos, archive_name)]:
        signature = read_signature(directory / f'{name}.sig')
        check_signature(signature, updater_pubkey(root, platform), current)
        platforms[UPDATER_TARGETS[platform]] = {'signature': signature, 'url': f'{DOWNLOAD_URL}/{tag}/{name}'}
    require_empty(output)
    output.mkdir(parents=True, exist_ok=True)
    for directory, names in expected.values():
        for name in sorted(names):
            shutil.copy2(directory / name, output / name)
    manifest = {
        'version': current,
        'notes': (root / 'docs/RELEASE_NOTES.md').read_text(encoding='utf-8'),
        'pub_date': datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z'),
        'platforms': platforms,
    }
    (output / 'latest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    write_checksums(output)
    return current


def require_clean(root):
    # Never package dirty source alongside an executable from another revision.
    status = subprocess.check_output(['git', 'status', '--porcelain'], cwd=root).decode('utf-8')
    if status.strip():
        raise ValueError('Commit the reviewed changes before assembling release assets:\n' + status)


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command')
    windows = commands.add_parser('windows', help='Package Windows and shared release assets')
    windows.add_argument('--installer', type=Path, required=True)
    windows.add_argument('--ffmpeg-source', type=Path, required=True)
    macos = commands.add_parser('macos', help='Package macOS release assets')
    macos.add_argument('--dmg', type=Path, required=True)
    macos.add_argument('--app-archive', type=Path, required=True)
    for command in (windows, macos):
        command.add_argument('--tag', required=True)
        command.add_argument('--output', type=Path, required=True)
        command.add_argument('--updater', action='store_true', help='Require the signed updater artifact')
    combined = commands.add_parser('publish-set', help='Combine both platforms and write latest.json')
    combined.add_argument('--tag', required=True)
    combined.add_argument('--windows', type=Path, required=True)
    combined.add_argument('--macos', type=Path, required=True)
    combined.add_argument('--output', type=Path, required=True)
    signature = commands.add_parser('check-signature', help='Check an updater signature binding')
    signature.add_argument('--platform', choices=CONFIG_PLATFORMS, required=True)
    signature.add_argument('--version', required=True)
    signature.add_argument('signature', type=Path)
    if not argv or argv[0].startswith('-') and argv[0] not in ('-h', '--help'):
        check = argparse.ArgumentParser(description='Check that all version files agree.')
        check.add_argument('--tag')
        args = check.parse_args(argv)
        print(f'Release metadata verified: v{version(tag=args.tag)}')
        return
    args = parser.parse_args(argv)
    if args.command == 'check-signature':
        check_signature(read_signature(args.signature), updater_pubkey(ROOT, args.platform), args.version)
        print(f'Updater signature is bound to version {args.version} and the committed public key')
        return
    require_clean(ROOT)
    if args.command == 'windows':
        current = assemble(ROOT, args.installer, args.ffmpeg_source, args.output, args.tag, args.updater)
    elif args.command == 'macos':
        current = package_macos(ROOT, args.dmg, args.app_archive, args.output, args.tag, args.updater)
    else:
        current = publish_set(ROOT, args.tag, args.windows, args.macos, args.output)
    print(f'Release assets assembled: v{current}')


if __name__ == '__main__':
    main()
