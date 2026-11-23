#!/usr/bin/env bash
# Build the bundled FFmpeg tools from the pinned official source archive (see media.lock.json).
#   Linux/WSL:            MinGW-w64 cross-build of ffmpeg.exe/ffprobe.exe (windows-x86_64).
#   macOS, Apple Silicon: native build of ffmpeg/ffprobe (darwin-aarch64), macOS 14.2 or later.
# Both use the same audio-only LGPL component set. The result is checked against the lock;
# pass --record only when deliberately pinning a new build (it rewrites the lock and NOTICE.txt).
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
mode=${1:---verify}
case "$mode" in
  --verify|--record) ;;
  *) echo "usage: scripts/build-media.sh [--verify|--record]" >&2; exit 2 ;;
esac
case "$(uname -s)" in
  Linux) platform=windows-x86_64 ;;
  Darwin) platform=darwin-aarch64 ;;
  *) echo "Unsupported build host $(uname -s): use Linux/WSL for Windows media or an Apple Silicon Mac for macOS media" >&2; exit 1 ;;
esac
work="$root/.tmp/media-build"
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
export SOURCE_DATE_EPOCH=1763596800

# One component set for every platform: audio decoding, MP4/MOV, Matroska/WebM and Ogg demuxing
# (video files are only read for their audio stream), AAC/PCM encoding and the filters the app uses.
# No video decoders, network protocols, external libraries, GPL or nonfree code.
# Argument order is part of the embedded configuration string and therefore of the pinned checksums.
# shellcheck disable=SC2054 # commas belong to FFmpeg's component lists
components=(--disable-everything --disable-autodetect --disable-x86asm --disable-doc --disable-debug --disable-network --disable-avdevice --disable-swscale --disable-gpl --disable-nonfree --enable-small --enable-static --disable-shared --enable-ffmpeg --enable-ffprobe --enable-protocol=file,pipe --enable-decoder=aac,mp3,mp3float,flac,opus,vorbis,alac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,pcm_f64le,pcm_u8,pcm_s16be,pcm_s24be,pcm_s32be --enable-parser=aac,mpegaudio,flac,opus,vorbis --enable-demuxer=wav,mp3,mov,aac,flac,matroska,ogg,pcm_f32le --enable-encoder=aac,pcm_s16le,pcm_f32le --enable-muxer=wav,ipod,null,pcm_f32le --enable-filter=aresample,aformat,anull,atrim,asetpts,silenceremove,pan,abuffer,abuffersink,amix,alimiter,apad)

if [[ $platform == windows-x86_64 ]]; then
  # Unchanged Windows recipe: identical arguments produce the pinned ffmpeg.exe/ffprobe.exe.
  if [[ ! -d "$work/ffmpeg-8.0.1" ]]; then tar -xJf "$archive" -C "$work"; fi
  cd "$work/ffmpeg-8.0.1"
  ./configure --fatal-warnings --target-os=mingw32 --arch=x86_64 --cross-prefix=x86_64-w64-mingw32- --enable-cross-compile --pkg-config=pkg-config "${components[@]}" '--extra-ldflags=-static -Wl,--no-insert-timestamp'
  make -j"${XTALK_BUILD_JOBS:-4}"
  x86_64-w64-mingw32-strip ffmpeg.exe ffprobe.exe
  cp ffmpeg.exe ffprobe.exe "$root/src-tauri/binaries/"
else
  # Native arm64 only: Rosetta reports x86_64 and a cross-build would not match the pinned checksums.
  if [[ $(uname -m) != arm64 ]]; then echo "macOS media must be built natively on Apple Silicon (uname -m: $(uname -m))" >&2; exit 1; fi
  # The checksums are tied to one Apple toolchain; select it with DEVELOPER_DIR or xcode-select.
  xcode=$(python3 -c 'import json;print(json.load(open("scripts/media.lock.json"))["platforms"]["darwin-aarch64"]["xcode"])')
  actual=$(xcodebuild -version 2>/dev/null | head -n 1 || true)
  if [[ $actual != "Xcode $xcode" && ${XTALK_MEDIA_ANY_XCODE:-} != 1 ]]; then
    echo "macOS media is pinned to Xcode $xcode, found '${actual:-no Xcode}'. Select it (DEVELOPER_DIR=/Applications/Xcode_$xcode.app/Contents/Developer) or set XTALK_MEDIA_ANY_XCODE=1 for an unpinned experiment." >&2
    exit 1
  fi
  build="$work/darwin-aarch64"
  # Always a clean tree, so no object from an earlier configuration can leak into a pinned build.
  rm -rf "$build"
  mkdir -p "$build/tools"
  # No external libraries: a pkg-config that finds nothing, so Homebrew .pc files can never be used.
  # Referenced by name through PATH, because configure embeds its arguments in the binaries.
  # shellcheck disable=SC2016 # the shim's own "$1"
  printf '#!/bin/sh\n[ "$1" = --version ] && { echo 0.0.0; exit 0; }\nexit 1\n' > "$build/tools/xtalk-no-pkg-config"
  chmod +x "$build/tools/xtalk-no-pkg-config"
  export PATH="$build/tools:$PATH"
  tar -xJf "$archive" -C "$build"
  cd "$build/ffmpeg-8.0.1"
  # Deterministic static archives; the deployment target is also passed explicitly below.
  export ZERO_AR_DATE=1 MACOSX_DEPLOYMENT_TARGET=14.2
  # VideoToolbox/AudioToolbox and other Apple media frameworks stay disabled (as does every
  # autodetected library): the app needs only FFmpeg's own audio decoders, which keeps decoding
  # identical to Windows and the binaries free of media-framework dependencies.
  ./configure --fatal-warnings --arch=aarch64 --cc=clang --pkg-config=xtalk-no-pkg-config "${components[@]}" --disable-videotoolbox --disable-audiotoolbox '--extra-cflags=-mmacosx-version-min=14.2' '--extra-ldflags=-mmacosx-version-min=14.2 -Wl,-dead_strip_dylibs'
  make -j"${XTALK_BUILD_JOBS:-$(sysctl -n hw.ncpu)}"
  strip ffmpeg ffprobe
  for tool in ffmpeg ffprobe; do
    # Stripping invalidates the linker's ad-hoc signature; arm64 macOS refuses unsigned code.
    codesign --force --sign - --identifier "$tool" "$tool"
    codesign --verify --strict "$tool"
    archs=$(lipo -archs "$tool")
    if [[ $archs != arm64 ]]; then echo "$tool: expected arm64 only, found: $archs" >&2; exit 1; fi
    minos=$(otool -l "$tool" | awk '/LC_BUILD_VERSION/{found=1} found && $1=="minos"{print $2; exit}')
    if [[ $minos != 14.2 ]]; then echo "$tool: minimum macOS is '$minos', expected 14.2" >&2; exit 1; fi
    # Self-contained: only libraries that ship with macOS (no Homebrew, /usr/local or @rpath).
    otool -L "$tool"
    foreign=$(otool -L "$tool" | tail -n +2 | awk '{print $1}' | grep -Ev '^(/usr/lib/|/System/Library/)' || true)
    if [[ -n $foreign ]]; then echo "$tool links libraries outside macOS: $foreign" >&2; exit 1; fi
    banner=$(./"$tool" -hide_banner -version)
    echo "${banner%%$'\n'*}"
  done
  cp ffmpeg ffprobe "$root/src-tauri/binaries/"
  chmod 0755 "$root/src-tauri/binaries/ffmpeg" "$root/src-tauri/binaries/ffprobe"
fi
cp COPYING.LGPLv2.1 "$root/src-tauri/binaries/FFmpeg-LICENSE.txt"
cp ffbuild/config.log "$root/.tmp/ffmpeg-config.log"
cd "$root"
python3 scripts/verify-media.py --platform "$platform" "$mode"
