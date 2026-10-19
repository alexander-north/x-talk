"""Validate versions and assemble reviewed release assets; never publishes anything."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def version(root=ROOT, tag=None):
    package = json.loads((root / 'package.json').read_text())
    lock = json.loads((root / 'package-lock.json').read_text())
    tauri = json.loads((root / 'src-tauri/tauri.conf.json').read_text())
    cargo = tomllib.loads((root / 'src-tauri/Cargo.toml').read_text())
    cargo_lock = tomllib.loads((root / 'src-tauri/Cargo.lock').read_text())
    versions = [package['version'], lock['version'], lock['packages']['']['version'],
                tauri['version'], cargo['package']['version']]
    versions.extend(p['version'] for p in cargo_lock['package'] if p['name'] == 'x-talk')
    if len(versions) != 6 or len(set(versions)) != 1:
        raise ValueError(f'Package versions disagree: {versions}')
    current = versions[0]
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


def assemble(root, installer, source, output, tag, updater=False):
    current = version(root, tag)
    expected = f'x-talk_{current}_x64-setup.exe'
    if installer.name != expected or not installer.is_file():
        raise ValueError(f'Expected installer {expected}')
    with installer.open('rb') as handle:
        if handle.read(2) != b'MZ':
            raise ValueError('Installer is not a Windows executable')
    media = json.loads((root / 'scripts/media.lock.json').read_text())
    if sha256(source) != media['sourceSha256']:
        raise ValueError('FFmpeg source checksum mismatch')
    for name, checksum in media['binaries'].items():
        if sha256(root / 'src-tauri/binaries' / name) != checksum:
            raise ValueError(f'Media checksum mismatch: {name}')
    required = ['LICENSE', 'NOTICE', 'scripts/build-media.sh', 'scripts/media.lock.json',
                'scripts/verify-media.py', 'src-tauri/binaries/FFmpeg-LICENSE.txt',
                'src-tauri/binaries/NOTICE.txt', 'src-tauri/binaries/THIRD-PARTY-NOTICES.txt']
    for name in required:
        if not (root / name).is_file() or (root / name).stat().st_size == 0:
            raise ValueError(f'Missing release material: {name}')
    dependency_sources = root / '.tmp/dependency-sources.tar.gz'
    if not dependency_sources.is_file():
        raise ValueError('Generate dependency notices and sources before packaging')
    if output.exists() and any(output.iterdir()):
        raise ValueError('Output directory must be empty; refusing stale release assets')
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(installer, output / expected)
    shutil.copy2(dependency_sources, output / f'x-talk_{current}_dependency-sources.tar.gz')
    shutil.copy2(source, output / f"ffmpeg-{media['version']}.tar.xz")
    # Preserve recipe paths so the included build script resolves its repository root.
    with tarfile.open(output / f'x-talk_{current}_licenses-and-build.tar.gz', 'w:gz') as archive:
        for name in required:
            archive.add(root / name, arcname=name)
    # Git archives exclude untracked local data, .env, dependencies and build products.
    subprocess.run(['git', 'archive', '--format=tar.gz', f'--prefix=x-talk-{current}/',
                    '-o', str((output / f'x-talk_{current}_source.tar.gz').resolve()), 'HEAD'],
                   cwd=root, check=True)
    if updater:
        signature = Path(str(installer) + '.sig')
        if not signature.is_file() or not signature.read_text(encoding='utf-8').strip():
            raise ValueError('Signed release requires an updater signature')
        shutil.copy2(signature, output / signature.name)
        manifest = {
            'version': current,
            'notes': (root / 'docs/RELEASE_NOTES.md').read_text(encoding='utf-8'),
            'pub_date': datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z'),
            'platforms': {'windows-x86_64': {
                'signature': signature.read_text(encoding='utf-8').strip(),
                'url': f'https://github.com/alexander-north/x-talk/releases/download/{tag}/{expected}'}}
        }
        (output / 'latest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')
    checksums = ''.join(f'{sha256(p)}  {p.name}\n' for p in sorted(output.iterdir()) if p.is_file())
    (output / 'SHA256SUMS.txt').write_text(checksums, encoding='utf-8')
    return current


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag')
    parser.add_argument('--updater', action='store_true', help='Require signed installer and create update feed')
    parser.add_argument('--installer', type=Path)
    parser.add_argument('--ffmpeg-source', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    current = version(tag=args.tag)
    if any([args.installer, args.ffmpeg_source, args.output]):
        if not all([args.installer, args.ffmpeg_source, args.output, args.tag]):
            parser.error('Packaging requires --tag, --installer, --ffmpeg-source and --output')
        # Never package dirty source alongside an executable from another revision.
        status = subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT).decode('utf-8')
        if status.strip():
            raise ValueError('Commit the reviewed changes before assembling release assets:\n' + status)
        assemble(ROOT, args.installer, args.ffmpeg_source, args.output, args.tag, args.updater)
    print(f'Release metadata verified: v{current}')


if __name__ == '__main__':
    main()
