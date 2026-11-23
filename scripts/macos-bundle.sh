#!/usr/bin/env bash
# After `npm run desktop:build` on an Apple Silicon Mac: create the updater archive
# (x-talk_<version>_aarch64.app.tar.gz) and smoke-check the .app, the archive and the .dmg without
# launching the GUI. Needs no signing key; the archive is signed separately afterwards.
set -euo pipefail
cd "$(dirname "$0")/.."
fail() { echo "macOS bundle check failed: $*" >&2; exit 1; }
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || fail 'run this on an Apple Silicon Mac'
version=$(python3 -c 'import json; print(json.load(open("package.json", encoding="utf-8"))["version"])')
config=$(python3 -c 'import sys; sys.path.insert(0, "scripts"); import release
c = release.tauri_config(platform="macos"); print(c["identifier"], c["productName"], release.MACOS_MINIMUM)')
read -r identifier product minimum <<<"$config"
bundle=src-tauri/target/release/bundle
app="$bundle/macos/$product.app"
dmg="$bundle/dmg/${product}_${version}_aarch64.dmg"
archive="$bundle/macos/${product}_${version}_aarch64.app.tar.gz"
if [[ ! -d $app || ! -f $dmg ]]; then
  find "$bundle" -maxdepth 2 >&2 || true
  fail "expected $app and $dmg"
fi
scratch=$(mktemp -d)
mounted=''
cleanup() {
  if [[ -n $mounted ]]; then hdiutil detach "$mounted" -quiet || hdiutil detach "$mounted" -force -quiet || true; fi
  rm -rf "$scratch"
}
trap cleanup EXIT

# Info.plist: identity, version and the privacy strings macOS requires before granting access.
plist="$app/Contents/Info.plist"
key() { plutil -extract "$1" raw -o - "$plist" 2>/dev/null || true; }
[[ $(key CFBundleIdentifier) == "$identifier" ]] || fail "CFBundleIdentifier is '$(key CFBundleIdentifier)'"
[[ $(key CFBundleShortVersionString) == "$version" ]] || fail "CFBundleShortVersionString is '$(key CFBundleShortVersionString)'"
[[ $(key LSMinimumSystemVersion) == "$minimum" ]] || fail "LSMinimumSystemVersion is '$(key LSMinimumSystemVersion)'"
[[ $(key CFBundlePackageType) == APPL ]] || fail 'CFBundlePackageType is not APPL'
for name in CFBundleVersion NSMicrophoneUsageDescription NSAudioCaptureUsageDescription; do
  [[ -n $(key "$name") ]] || fail "Info.plist is missing $name"
done

# Mach-O checks: arm64 only, built for the declared minimum macOS.
macho() {
  local file=$1 archs minos
  [[ -x $file ]] || fail "$file is missing or not executable"
  archs=$(lipo -archs "$file")
  [[ $archs == arm64 ]] || fail "$file architectures: $archs"
  minos=$(otool -l "$file" | awk '/LC_BUILD_VERSION/{found=1} found && $1=="minos"{print $2; exit}')
  [[ $minos == "$minimum" ]] || fail "$file minimum macOS is '$minos', expected $minimum"
}
macho "$app/Contents/MacOS/$(key CFBundleExecutable)"

# Ad-hoc signature over the whole bundle (no Developer ID); the hardened runtime needs the
# audio-input entitlement or macOS silently denies the microphone.
codesign --verify --deep --strict --verbose=2 "$app"
details=$(codesign --display --verbose=2 "$app" 2>&1)
grep -q '^Signature=adhoc' <<<"$details" || fail 'bundle is not ad-hoc signed'
if grep -q '^CodeDirectory.*flags=.*runtime' <<<"$details"; then
  codesign --display --entitlements - --xml "$app" 2>/dev/null | grep -q 'com.apple.security.device.audio-input' ||
    fail 'hardened runtime without the com.apple.security.device.audio-input entitlement'
fi

# Bundled media: the tools scripts/build-media.sh built and verified (see its media-build.json),
# runnable from inside the bundle, plus license files.
python3 scripts/verify-media.py --platform darwin-aarch64
resources="$app/Contents/Resources"
for tool in ffmpeg ffprobe; do
  cmp -s "$resources/binaries/$tool" "src-tauri/binaries/$tool" || fail "bundled $tool differs from the verified build"
  macho "$resources/binaries/$tool"
done
for name in LICENSE NOTICE binaries/NOTICE.txt binaries/FFmpeg-LICENSE.txt binaries/THIRD-PARTY-NOTICES.txt; do
  [[ -s $resources/$name ]] || fail "missing bundled $name"
done
python3 - "$scratch/tone.wav" <<'PY'
import math, struct, sys, wave
with wave.open(sys.argv[1], 'wb') as out:
    out.setnchannels(1); out.setsampwidth(2); out.setframerate(48000)
    out.writeframes(b''.join(struct.pack('<h', int(8000 * math.sin(i / 20))) for i in range(48000)))
PY
"$resources/binaries/ffmpeg" -nostdin -hide_banner -loglevel error -i "$scratch/tone.wav" -c:a aac "$scratch/tone.m4a"
codec=$("$resources/binaries/ffprobe" -v error -show_entries stream=codec_name -of csv=p=0 "$scratch/tone.m4a")
[[ $codec == aac ]] || fail "bundled ffmpeg/ffprobe round trip produced '$codec'"

# Updater archive: exactly one top-level <product>.app, no AppleDouble/xattr entries, and it must
# unpack to a bundle whose signature still verifies (tauri-plugin-updater swaps it in place).
rm -f "$archive" "$archive.sig"
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -czf "$archive" -C "$bundle/macos" "$product.app"
mkdir "$scratch/unpacked"
tar -xzf "$archive" -C "$scratch/unpacked"
[[ $(ls -A "$scratch/unpacked") == "$product.app" ]] || fail 'updater archive must contain only the app bundle'
codesign --verify --deep --strict "$scratch/unpacked/$product.app"
diff -r "$app" "$scratch/unpacked/$product.app" >/dev/null || fail 'updater archive does not reproduce the app bundle'

# Disk image: valid, mounts read-only, and carries the same signed app.
hdiutil verify -quiet "$dmg"
mkdir "$scratch/mount"
hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$scratch/mount" "$dmg" >/dev/null
mounted="$scratch/mount"
[[ -d $mounted/$product.app ]] || fail 'disk image does not contain the app'
codesign --verify --deep --strict "$mounted/$product.app"
diff -r "$app" "$mounted/$product.app" >/dev/null || fail 'disk image app differs from the built bundle'
[[ -L $mounted/Applications ]] || echo 'warning: disk image has no Applications shortcut' >&2
hdiutil detach "$mounted" -quiet
mounted=''

echo "macOS bundle checks passed: $app, $dmg"
echo "Updater archive: $archive"
