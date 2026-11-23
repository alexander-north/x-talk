import { existsSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
// Same rules as verify-media.py: Linux/WSL builds and checks the Windows tools. A platform pinned in
// media.lock.json must match its recorded checksums; macOS tools must be exactly the ones the last
// scripts/build-media.sh run built from the pinned source with the current recipe.
const tools = { 'windows-x86_64': ['ffmpeg.exe', 'ffprobe.exe'], 'darwin-aarch64': ['ffmpeg', 'ffprobe'] };
let platform = process.env.XTALK_MEDIA_PLATFORM || 'windows-x86_64';
if (!process.env.XTALK_MEDIA_PLATFORM && process.platform === 'darwin') {
 if (process.arch !== 'arm64') throw new Error('macOS media is built and bundled for Apple Silicon only');
 platform = 'darwin-aarch64';
}
if (!tools[platform]) throw new Error(`Unknown media platform: ${platform}`);
const file = (path) => new URL(`../${path}`, import.meta.url);
const sha256 = (path) => createHash('sha256').update(readFileSync(file(path))).digest('hex');
const lock = JSON.parse(readFileSync(file('scripts/media.lock.json'), 'utf8'));
let expected = lock.platforms[platform].binaries;
if (!expected) {
 const manifestPath = 'src-tauri/binaries/media-build.json';
 if (!existsSync(file(manifestPath)) || !tools[platform].every((name) => existsSync(file(`src-tauri/binaries/${name}`)))) {
  throw new Error(`No media build for ${platform}: run scripts/build-media.sh first (see README).`);
 }
 const manifest = JSON.parse(readFileSync(file(manifestPath), 'utf8'));
 if (manifest.platform !== platform || manifest.sourceSha256 !== lock.sourceSha256 || manifest.version !== lock.version) {
  throw new Error('The media tools were built from another FFmpeg source; run scripts/build-media.sh again.');
 }
 if (manifest.recipeSha256 !== sha256('scripts/build-media.sh')) {
  throw new Error('The media tools were built with another version of scripts/build-media.sh; run it again.');
 }
 expected = manifest.binaries ?? {};
}
for (const name of tools[platform]) {
 if (!expected[name]) throw new Error(`No recorded media checksum for ${platform}: ${name}. See docs/RELEASING.md.`);
 if (sha256(`src-tauri/binaries/${name}`) !== expected[name]) throw new Error(`Media checksum mismatch: ${name}. Rebuild with scripts/build-media.sh.`);
}
console.log(`Bundled media checksums verified for ${platform}.`);
