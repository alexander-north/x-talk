import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
// Same platform choice as verify-media.py: Linux/WSL builds and checks the Windows tools.
const tools = { 'windows-x86_64': ['ffmpeg.exe', 'ffprobe.exe'], 'darwin-aarch64': ['ffmpeg', 'ffprobe'] };
let platform = process.env.XTALK_MEDIA_PLATFORM || 'windows-x86_64';
if (!process.env.XTALK_MEDIA_PLATFORM && process.platform === 'darwin') {
 if (process.arch !== 'arm64') throw new Error('macOS media is built and bundled for Apple Silicon only');
 platform = 'darwin-aarch64';
}
if (!tools[platform]) throw new Error(`Unknown media platform: ${platform}`);
const lock = JSON.parse(readFileSync(new URL('./media.lock.json', import.meta.url), 'utf8'));
const expected = lock.platforms[platform].binaries ?? {};
for (const name of tools[platform]) {
 if (!expected[name]) throw new Error(`No recorded media checksum for ${platform}: ${name}. See docs/RELEASING.md.`);
 const file = readFileSync(new URL(`../src-tauri/binaries/${name}`, import.meta.url));
 if (createHash('sha256').update(file).digest('hex') !== expected[name]) throw new Error(`Media checksum mismatch: ${name}`);
}
console.log(`Bundled media checksums verified for ${platform}.`);
