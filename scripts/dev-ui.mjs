// Native UI iteration with persistent, isolated data and offline providers (`npm run desktop:ui`).
// Windows keeps using dev-ui.ps1, which can also select the project-local Node runtime; macOS and
// other hosts get the same setup here: separate app identity, data under .tmp/ui-dev/data,
// synthetic providers and no update endpoint.
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const run = (command, args, options) => spawn(command, args, { cwd: root, stdio: 'inherit', ...options });
const finish = (child) => {
 for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
 child.on('exit', (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
};

if (process.platform === 'win32') {
 finish(run('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', join(root, 'scripts/dev-ui.ps1')]));
} else {
 const [major, minor, patch] = process.versions.node.split('.').map(Number);
 if (major < 24 || (major === 24 && (minor < 11 || (minor === 11 && patch < 1)))) {
  throw new Error('Node 24.11.1 or newer is required. See README build prerequisites.');
 }
 const tauri = join(root, 'node_modules/.bin/tauri');
 if (!existsSync(tauri)) throw new Error('Run npm ci first.');
 // The Tauri build needs the media resource files to exist. Tools from scripts/build-media.sh are
 // preferred; empty placeholders (debug builds then use Homebrew's ffmpeg) are allowed with a warning.
 if (!['ffmpeg', 'ffprobe'].every((tool) => existsSync(join(root, 'src-tauri/binaries', tool)))) {
  throw new Error('src-tauri/binaries/ffmpeg and ffprobe are missing. Build them with scripts/build-media.sh or create empty placeholders (see README).');
 }
 if (spawnSync(process.execPath, [join(root, 'scripts/verify-media.mjs')], { cwd: root, stdio: 'inherit' }).status !== 0) {
  console.warn('Using development media tools; desktop:build requires the ones scripts/build-media.sh builds.');
 }
 const data = join(root, '.tmp/ui-dev/data');
 mkdirSync(data, { recursive: true });
 const env = { ...process.env, XTALK_TEST_PROVIDERS: '1', XTALK_TEST_DATA_DIR: data };
 delete env.XTALK_TEST_UPDATE_URL;
 finish(run(tauri, ['dev', '--features', 'desktop,test-providers', '--config', 'src-tauri/tauri.ui-dev.conf.json'], { env }));
}
