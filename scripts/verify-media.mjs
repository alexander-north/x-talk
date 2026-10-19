import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
const lock = JSON.parse(readFileSync(new URL('./media.lock.json', import.meta.url), 'utf8'));
for (const [name, expected] of Object.entries(lock.binaries)) {
 const file = readFileSync(new URL(`../src-tauri/binaries/${name}`, import.meta.url));
 if (createHash('sha256').update(file).digest('hex') !== expected) throw new Error(`Media checksum mismatch: ${name}`);
}
console.log('Bundled media checksums verified.');
