"""Verify pinned media build inputs. --record is used by the reproducible source build."""
import hashlib,json,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parent.parent
lock_path=root/'scripts/media.lock.json'
lock=json.loads(lock_path.read_text())
files={name:hashlib.sha256((root/'src-tauri/binaries'/name).read_bytes()).hexdigest() for name in ['ffmpeg.exe','ffprobe.exe']}
if '--record' in sys.argv:
 lock['binaries']=files
 lock_path.write_text(json.dumps(lock,indent=2)+'\n')
 notice=f"FFmpeg {lock['version']} · audio-only static LGPL 2.1 or later build\nSource: {lock['source']}\nSource SHA-256: {lock['sourceSha256']}\nBuild recipe: scripts/build-media.sh in the x-talk source repository.\nNo GPL/nonfree components or external codec libraries.\nCorresponding source is the unchanged official archive; configure flags are in the build script.\nThese separate executables communicate through files and pipes.\nLicense: FFmpeg-LICENSE.txt\n\n"+'\n'.join(f'{k}: {v}' for k,v in files.items())+'\n'
 (root/'src-tauri/binaries/NOTICE.txt').write_text(notice)
elif lock.get('binaries')!=files: raise SystemExit('Bundled media checksum mismatch. Rebuild from the pinned source.')
print('Media binary checksums verified.')
