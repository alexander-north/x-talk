"""Verify the bundled FFmpeg tools against media.lock.json.

The lock pins one official source archive. A platform entry with `binaries` is pinned: its tools must
reproduce those checksums (Windows, cross-built on Linux, is reproducible and pinned). A platform
without `binaries` (macOS) is verified per build instead: scripts/build-media.sh builds the tools
from the checksum-verified source, checks them, and writes a build manifest
(src-tauri/binaries/media-build.json) with their checksums, the source checksum, the recipe
checksum and the toolchain. Verification then requires the tools to be exactly the ones that build
produced, from the current recipe; release packaging publishes the manifest with each release.

--record pins a deliberately rebuilt pinned platform; it also regenerates
src-tauri/binaries/NOTICE.txt, which ships next to the tools on every platform.
"""
import argparse
import hashlib
import json
import os
import platform as host
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLATFORMS = {
    'windows-x86_64': ('ffmpeg.exe', 'ffprobe.exe'),
    'darwin-aarch64': ('ffmpeg', 'ffprobe'),
}
MANIFEST = 'src-tauri/binaries/media-build.json'
RECIPE = 'scripts/build-media.sh'
HEADER = """FFmpeg {version} · audio-only static LGPL 2.1 or later build
Source: {source}
Source SHA-256: {sha}
Build recipe: scripts/build-media.sh in the x-talk source repository.
No GPL/nonfree components or external codec libraries.
Corresponding source is the unchanged official archive; configure flags are in the build script.
These separate executables communicate through files and pipes.
License: FFmpeg-LICENSE.txt
"""
PER_BUILD = ('Built from the source above for each app build. Each GitHub release publishes the SHA-256\n'
             'checksums of its tools as x-talk_<version>_aarch64_ffmpeg-build.json.\n')


def host_platform():
    """Platform whose tools this machine builds or bundles. Linux/WSL builds the Windows tools."""
    if host.system() == 'Darwin':
        if host.machine() != 'arm64':
            raise SystemExit('macOS media is built and bundled for Apple Silicon only')
        return 'darwin-aarch64'
    return 'windows-x86_64'


def pinned(lock, platform):
    return 'binaries' in lock['platforms'][platform]


def render_notice(lock):
    parts = [HEADER.format(version=lock['version'], source=lock['source'], sha=lock['sourceSha256'])]
    for name, names in PLATFORMS.items():
        entry = lock['platforms'][name]
        parts.append(f"\n{entry['description']}:\n")
        if not pinned(lock, name):
            parts.append(PER_BUILD)
            continue
        binaries = entry.get('binaries') or {}
        if not binaries:
            parts.append('Checksums not yet recorded; no release contains these tools.\n')
        for binary in names:
            if binary in binaries:
                parts.append(f'{binary}: {binaries[binary]}\n')
    return ''.join(parts)


def sha256_file(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checksums(root, platform):
    return {name: sha256_file(root / 'src-tauri/binaries' / name) for name in PLATFORMS[platform]}


def check_manifest(root, lock, platform, manifest):
    """Raise unless `manifest` describes a build of `platform` from the pinned source and current recipe."""
    if not isinstance(manifest, dict) or manifest.get('platform') != platform:
        raise SystemExit(f'Media build manifest is not for {platform}')
    if manifest.get('sourceSha256') != lock['sourceSha256'] or manifest.get('version') != lock['version']:
        raise SystemExit('Media build manifest is for another FFmpeg source; rebuild with scripts/build-media.sh')
    if manifest.get('recipeSha256') != sha256_file(root / RECIPE):
        raise SystemExit('Media tools were built with another version of scripts/build-media.sh; rebuild them')
    binaries = manifest.get('binaries')
    if not isinstance(binaries, dict) or set(binaries) != set(PLATFORMS[platform]) or not all(
            isinstance(d, str) and len(d) == 64 and all(c in '0123456789abcdef' for c in d) for d in binaries.values()):
        raise SystemExit('Malformed media build manifest')
    return binaries


def write_manifest(root, platform, toolchain):
    lock = json.loads((root / 'scripts/media.lock.json').read_text(encoding='utf-8'))
    if pinned(lock, platform):
        raise SystemExit(f'{platform} media is pinned in media.lock.json; use --verify or --record')
    manifest = {'platform': platform, 'version': lock['version'], 'sourceSha256': lock['sourceSha256'],
                'recipeSha256': sha256_file(root / RECIPE), 'toolchain': toolchain,
                'binaries': checksums(root, platform)}
    (root / MANIFEST).write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')


def verify(root, platform):
    lock = json.loads((root / 'scripts/media.lock.json').read_text(encoding='utf-8'))
    missing = [n for n in PLATFORMS[platform] if not (root / 'src-tauri/binaries' / n).is_file()]
    if pinned(lock, platform):
        expected = lock['platforms'][platform].get('binaries') or {}
        if set(expected) != set(PLATFORMS[platform]):
            if not missing:
                for name, digest in checksums(root, platform).items():
                    print(f'{name}: built {digest}')
            raise SystemExit(f'No recorded media checksums for {platform}. Build on the pinned toolchain and '
                             'record them (see docs/RELEASING.md).')
    else:
        path = root / MANIFEST
        if missing or not path.is_file():
            raise SystemExit(f'No media build for {platform}: run scripts/build-media.sh (see README).')
        try:
            manifest = json.loads(path.read_text(encoding='utf-8'))
        except ValueError as error:
            raise SystemExit('Malformed media build manifest') from error
        expected = check_manifest(root, lock, platform, manifest)
    actual = checksums(root, platform)
    if actual != expected:
        for name in PLATFORMS[platform]:
            print(f'{name}: expected {expected[name]}, found {actual[name]}')
        raise SystemExit('Bundled media checksum mismatch. Rebuild with scripts/build-media.sh.')
    notice = (root / 'src-tauri/binaries/NOTICE.txt').read_text(encoding='utf-8')
    if notice != render_notice(lock):
        raise SystemExit('src-tauri/binaries/NOTICE.txt is out of date with media.lock.json')


def record(root, platform):
    path = root / 'scripts/media.lock.json'
    lock = json.loads(path.read_text(encoding='utf-8'))
    if not pinned(lock, platform):
        raise SystemExit(f'{platform} media is verified per build and published with each release, not pinned')
    lock['platforms'][platform]['binaries'] = checksums(root, platform)
    path.write_text(json.dumps(lock, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    (root / 'src-tauri/binaries/NOTICE.txt').write_text(render_notice(lock), encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--platform', choices=sorted(PLATFORMS))
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--verify', action='store_true', help='Check the built tools (default)')
    action.add_argument('--record', action='store_true', help='Pin the built tools (pinned platforms)')
    action.add_argument('--write-manifest', metavar='TOOLCHAIN',
                        help='Describe a fresh per-build platform build (scripts/build-media.sh only)')
    args = parser.parse_args()
    platform = args.platform or os.environ.get('XTALK_MEDIA_PLATFORM') or host_platform()
    if platform not in PLATFORMS:
        raise SystemExit(f'Unknown media platform: {platform}')
    if args.record:
        record(ROOT, platform)
        print(f'Recorded media checksums for {platform}.')
    elif args.write_manifest is not None:
        write_manifest(ROOT, platform, args.write_manifest)
        print(f'Wrote {MANIFEST} for {platform}.')
    else:
        verify(ROOT, platform)
        print(f'Media binary checksums verified for {platform}.')


if __name__ == '__main__':
    main()
