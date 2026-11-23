"""Stamp an immutable CI release commit without changing main's development version."""
import argparse
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
# Tauri overlays the platform file on tauri.conf.json; either may carry the application version.
PLATFORM_CONFIGS = ['src-tauri/tauri.windows.conf.json', 'src-tauri/tauri.macos.conf.json']
TAURI_CONFIGS = ['src-tauri/tauri.conf.json', *PLATFORM_CONFIGS]
VERSION_FILES = ['package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock',
                 *TAURI_CONFIGS]


def stamp(root, number):
    if not 1 <= number <= 65535:
        raise ValueError('Release run number must fit the Windows patch version (1–65535)')
    package = json.loads((root / 'package.json').read_text(encoding='utf-8'))
    major, minor, _ = package['version'].split('.')
    version = f'{major}.{minor}.{number}'
    for name in ['package.json', 'package-lock.json', *TAURI_CONFIGS]:
        path = root / name
        if name in PLATFORM_CONFIGS and not path.is_file():
            continue
        data = json.loads(path.read_text(encoding='utf-8'))
        if name in PLATFORM_CONFIGS and 'version' not in data:
            # Platform files inherit the base version; leave them byte-for-byte unchanged.
            continue
        data['version'] = version
        if name == 'package-lock.json':
            data['packages']['']['version'] = version
        path.write_text(json.dumps(data, indent=2) + '\n', encoding='utf-8')
    for name, pattern in [('src-tauri/Cargo.toml', r'(\[package\]\nname = "x-talk"\nversion = ")[^"]+'),
                          ('src-tauri/Cargo.lock', r'(\[\[package\]\]\nname = "x-talk"\nversion = ")[^"]+')]:
        path = root / name
        text, count = re.subn(pattern, lambda m: m[1] + version, path.read_text(encoding='utf-8'))
        if count != 1:
            raise ValueError(f'Expected one application version in {name}')
        path.write_text(text, encoding='utf-8')
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('number', type=int)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    version = stamp(ROOT, args.number)
    subprocess.run(['git', 'add', *[name for name in VERSION_FILES if (ROOT / name).is_file()]],
                   cwd=ROOT, check=True)
    subprocess.run(['git', '-c', 'user.name=github-actions[bot]', '-c',
                    'user.email=41898282+github-actions[bot]@users.noreply.github.com',
                    'commit', '-m', f'Release x-talk {version}'], cwd=ROOT, check=True)
    sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    # A run retry reuses its existing tag; never replace a previously built release.
    tag = f'v{version}'
    existing = subprocess.check_output(['git', 'ls-remote', 'origin', f'refs/tags/{tag}'], cwd=ROOT, text=True).strip()
    if existing:
        sha = existing.split()[0]
    else:
        subprocess.run(['git', 'push', 'origin', f'{sha}:refs/tags/{tag}'], cwd=ROOT, check=True)
    with args.output.open('a', encoding='utf-8') as output:
        output.write(f'commit={sha}\ntag={tag}\n')


if __name__ == '__main__':
    main()
