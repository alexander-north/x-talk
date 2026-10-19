"""Collect attribution from the exact locked Rust and production JavaScript dependencies."""
import json,subprocess,tarfile
from pathlib import Path
root=Path(__file__).resolve().parent.parent
metadata=json.loads(subprocess.check_output(['cargo','metadata','--manifest-path',str(root/'src-tauri/Cargo.toml'),'--format-version','1','--locked','--features','desktop'],stderr=subprocess.DEVNULL))
parts=['x-talk third-party dependency notices\nGenerated from Cargo.lock and package-lock.json.\n\nFFmpeg is a separate executable; see FFmpeg-LICENSE.txt and NOTICE.txt.\n']
for package in sorted(metadata['packages'],key=lambda p:(p['name'],p['version'])):
 if package['name']=='x-talk':continue
 directory=Path(package['manifest_path']).parent
 files=[p for p in directory.iterdir() if p.is_file() and (p.name.upper().startswith(('LICENSE','COPYING','COPYRIGHT','UNLICENCE','UNLICENSE')))]
 parts.append(f"\n{'='*72}\nRust: {package['name']} {package['version']}\nLicense: {package.get('license')}\nSource: {package.get('repository') or package.get('source')}\n")
 for file in files:parts.append(file.name+'\n'+file.read_text(encoding='utf-8',errors='replace'))
lock=json.loads((root/'package-lock.json').read_text(encoding='utf-8'))
for name,package in sorted(lock['packages'].items()):
 if not name or package.get('dev',False):continue
 directory=root/name
 parts.append(f"\n{'='*72}\nJavaScript: {name} {package['version']}\nLicense: {package.get('license')}\nSource: {package.get('resolved')}\n")
 for file in directory.glob('*'):
  if file.is_file() and file.name.upper().startswith(('LICENSE','COPYING','COPYRIGHT','UNLICENSE')):parts.append(file.name+'\n'+file.read_text(encoding='utf-8',errors='replace'))
path=root/'src-tauri/binaries/THIRD-PARTY-NOTICES.txt';path.write_text('\n'.join(parts),encoding='utf-8');print('Third-party notices generated.')

# Include exact unmodified MPL crate sources even when only used by build tooling.
# Other dual-licensed dependencies can be used under their permissive alternatives.
source_archive=root/'.tmp/dependency-sources.tar.gz'
source_archive.parent.mkdir(parents=True,exist_ok=True)
with tarfile.open(source_archive,'w:gz') as archive:
 for package in metadata['packages']:
  if package.get('license')=='MPL-2.0':
   archive.add(Path(package['manifest_path']).parent,arcname=f"{package['name']}-{package['version']}")
print('Corresponding MPL dependency sources packaged.')
