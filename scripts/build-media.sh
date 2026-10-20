#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
work="$PWD/.tmp/media-build"
mkdir -p "$work" src-tauri/binaries
archive="$work/ffmpeg-8.0.1.tar.xz"
if [[ ! -f "$archive" ]]; then curl --fail --location --silent --show-error https://ffmpeg.org/releases/ffmpeg-8.0.1.tar.xz -o "$archive"; fi
# Official release archive, pinned by content (see media.lock.json).
python3 - "$archive" <<'PY'
import hashlib,json,sys
from pathlib import Path
lock=json.loads(Path('scripts/media.lock.json').read_text())
if hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest()!=lock['sourceSha256']: raise SystemExit('FFmpeg source checksum mismatch')
PY
if [[ ! -d "$work/ffmpeg-8.0.1" ]]; then tar -xJf "$archive" -C "$work"; fi
cd "$work/ffmpeg-8.0.1"
export SOURCE_DATE_EPOCH=1763596800
./configure --fatal-warnings --target-os=mingw32 --arch=x86_64 --cross-prefix=x86_64-w64-mingw32- --enable-cross-compile --pkg-config=pkg-config --disable-everything --disable-autodetect --disable-x86asm --disable-doc --disable-debug --disable-network --disable-avdevice --disable-swscale --disable-gpl --disable-nonfree --enable-small --enable-static --disable-shared --enable-ffmpeg --enable-ffprobe --enable-protocol=file,pipe --enable-decoder=aac,mp3,mp3float,flac,opus,vorbis,alac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,pcm_f64le,pcm_u8,pcm_s16be,pcm_s24be,pcm_s32be --enable-parser=aac,mpegaudio,flac,opus,vorbis --enable-demuxer=wav,mp3,mov,aac,flac,matroska,ogg,pcm_f32le --enable-encoder=aac,pcm_s16le,pcm_f32le --enable-muxer=wav,ipod,null,pcm_f32le --enable-filter=aresample,aformat,anull,atrim,asetpts,silenceremove,pan,abuffer,abuffersink,amix,alimiter,apad --extra-ldflags='-static -Wl,--no-insert-timestamp'
make -j"${XTALK_BUILD_JOBS:-4}"
x86_64-w64-mingw32-strip ffmpeg.exe ffprobe.exe
cp ffmpeg.exe ffprobe.exe "$OLDPWD/src-tauri/binaries/"
cp COPYING.LGPLv2.1 "$OLDPWD/src-tauri/binaries/FFmpeg-LICENSE.txt"
cp ffbuild/config.log "$OLDPWD/.tmp/ffmpeg-config.log"
cd "$OLDPWD"
python3 scripts/verify-media.py "${1:---verify}"
