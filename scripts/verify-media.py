"""Verify the bundled FFmpeg tools against media.lock.json.

The lock pins one official source archive and, per platform, the checksums of the tools built from it
by scripts/build-media.sh. --record is used by that build to pin a deliberately rebuilt platform; it
also regenerates src-tauri/binaries/NOTICE.txt, which ships next to the tools on every platform.
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
HEADER = """FFmpeg {version} · audio-only static LGPL 2.1 or later build
Source: {source}
Source SHA-256: {sha}
Build recipe: scripts/build-media.sh in the x-talk source repository.
No GPL/nonfree components or external codec libraries.
Corresponding source is the unchanged official archive; configure flags are in the build script.
These separate executables communicate through files and pipes.
License: FFmpeg-LICENSE.txt
"""


def host_platform():
    """Platform whose tools this machine builds or bundles. Linux/WSL builds the Windows tools."""
    if host.system() == 'Darwin':
        if host.machine() != 'arm64':
            raise SystemExit('macOS media is built and bundled for Apple Silicon only')
        return 'darwin-aarch64'
    return 'windows-x86_64'


def render_notice(lock):
    parts = [HEADER.format(version=lock['version'], source=lock['source'], sha=lock['sourceSha256'])]
    for name, names in PLATFORMS.items():
        entry = lock['platforms'][name]
        parts.append(f"\n{entry['description']}:\n")
        binaries = entry.get('binaries') or {}
        if not binaries:
            parts.append('Checksums not yet recorded; no release contains these tools.\n')
        for binary in names:
            if binary in binaries:
                parts.append(f'{binary}: {binaries[binary]}\n')
    return ''.join(parts)


def checksums(root, platform):
    return {name: hashlib.sha256((root / 'src-tauri/binaries' / name).read_bytes()).hexdigest()
            for name in PLATFORMS[platform]}


def verify(root, platform):
    lock = json.loads((root / 'scripts/media.lock.json').read_text(encoding='utf-8'))
    expected = lock['platforms'][platform].get('binaries') or {}
    if set(expected) != set(PLATFORMS[platform]):
        if all((root / 'src-tauri/binaries' / name).is_file() for name in PLATFORMS[platform]):
            for name, digest in checksums(root, platform).items():
                print(f'{name}: built {digest}')
        raise SystemExit(f'No recorded media checksums for {platform}. Build on the pinned toolchain and '
                         'record them (see docs/RELEASING.md).')
    actual = checksums(root, platform)
    if actual != expected:
        for name in PLATFORMS[platform]:
            print(f'{name}: expected {expected[name]}, built {actual[name]}')
        raise SystemExit('Bundled media checksum mismatch. Rebuild from the pinned source.')
    notice = (root / 'src-tauri/binaries/NOTICE.txt').read_text(encoding='utf-8')
    if notice != render_notice(lock):
        raise SystemExit('src-tauri/binaries/NOTICE.txt is out of date with media.lock.json')


def record(root, platform):
    path = root / 'scripts/media.lock.json'
    lock = json.loads(path.read_text(encoding='utf-8'))
    lock['platforms'][platform]['binaries'] = checksums(root, platform)
    path.write_text(json.dumps(lock, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    (root / 'src-tauri/binaries/NOTICE.txt').write_text(render_notice(lock), encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=sorted(PLATFORMS))
    action = parser.add_mutually_exclusive_group()
    action.add_argument('--verify', action='store_true', help='Check the built tools (default)')
    action.add_argument('--record', action='store_true', help='Pin the built tools')
    args = parser.parse_args()
    platform = args.platform or os.environ.get('XTALK_MEDIA_PLATFORM') or host_platform()
    if platform not in PLATFORMS:
        raise SystemExit(f'Unknown media platform: {platform}')
    if args.record:
        record(ROOT, platform)
        print(f'Recorded media checksums for {platform}.')
    else:
        verify(ROOT, platform)
        print(f'Media binary checksums verified for {platform}.')


if __name__ == '__main__':
    main()
