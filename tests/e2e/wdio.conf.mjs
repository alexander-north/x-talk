import { spawn } from 'node:child_process';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
let driver;let updateServer;
const isolated = process.env.XTALK_E2E_MEETING_RECOVERY==='1' || process.env.XTALK_E2E_UPDATES==='1' || process.env.XTALK_E2E_STOP==='1' || process.env.XTALK_E2E_OFFLINE==='1';
export const config = {
 hostname: '127.0.0.1', port: 4444, logLevel: 'warn', maxInstances: 1,
 specs: process.env.XTALK_E2E_MEETING_RECOVERY==='1'?['./meeting-recovery.mjs']:process.env.XTALK_E2E_UPDATES==='1'?['./updates.mjs']:process.env.XTALK_E2E_OFFLINE==='1'?['./offline.mjs','./glass.mjs','./shortcuts.mjs','./meetings.mjs']:process.env.XTALK_E2E_STOP==='1'?['./smoke.mjs','./meeting-recording.mjs','./recording.mjs','./realtime.mjs','./diagnostics.mjs','./sounds.mjs','./progress.mjs','./interface.mjs','./shortcuts.mjs','./quit-idle.mjs','./quit-stop.mjs','./quit-discard.mjs','./quit-realtime-stop.mjs','./quit-realtime-discard.mjs']:['./smoke.mjs'], framework: 'mocha', reporters: ['spec'],
 capabilities: [{ 'tauri:options': { application: path.resolve(process.env.XTALK_APP_PATH || 'src-tauri/target/debug/x-talk.exe') } }],
 mochaOpts: { timeout: 120000 },
 beforeSession() {
  const data=path.join(tmpdir(), 'x-talk', '.tmp', `e2e-${Date.now()}`);
  if(isolated) {
   const audio=path.join(data,'audio');mkdirSync(audio,{recursive:true});
   const identifier=randomUUID();
   writeFileSync(path.join(audio,`${identifier}.admission`),'{incomplete');
   writeFileSync(path.join(audio,`${identifier}.import.part`),'preserved interrupted import');
   const capture=randomUUID();
   writeFileSync(path.join(audio,`${capture}.journal`),'{incomplete');
   writeFileSync(path.join(audio,`${capture}.wav.part`),'preserved interrupted capture');
  }
  if(process.env.XTALK_E2E_MEETING_RECOVERY==='1') {
   const id=randomUUID(),directory=path.join(data,'meetings','audio',id);
   mkdirSync(directory,{recursive:true});mkdirSync('.tmp',{recursive:true});
   writeFileSync(path.join(directory,'microphone.f32'),Buffer.alloc(16000*4*5));
   writeFileSync(path.join(directory,'capture.json'),JSON.stringify({id,created:new Date().toISOString(),tracks:[{filename:'microphone.f32',rate:16000,channels:1,microphone:true}],elapsed:5,ready:true,discarded:false,warning:'Synthetic recovery warning'}));
   writeFileSync('.tmp/meeting-recovery-profile.json',JSON.stringify({id,directory}));
  }
  if(process.env.XTALK_E2E_UPDATES==='1') {
   mkdirSync('.tmp',{recursive:true});writeFileSync('.tmp/update-mode','unavailable');
   updateServer=spawn(process.execPath,['tests/e2e/update-server.mjs','.tmp/update-mode'],{stdio:'inherit'});
  }
  driver = spawn(path.join(homedir(), '.cargo', 'bin', 'tauri-driver.exe'),
   ['--native-driver', path.join(homedir(), '.cargo', 'bin', 'msedgedriver.exe')], {
   env: { ...process.env, XTALK_TEST_PROVIDERS: isolated?'1':'0', XTALK_TEST_DATA_DIR: data, XTALK_TEST_UPDATE_URL: process.env.XTALK_E2E_UPDATES==='1'?'http://127.0.0.1:18979/latest.json':'' }, stdio: ['ignore','inherit','inherit'],
  });
  driver.on('error', e => { throw e; });
 },
 async before() {
  const { browser }=await import('@wdio/globals');
  await browser.waitUntil(()=>browser.execute(()=>typeof window.__TAURI_INTERNALS__?.invoke==='function'),
   {timeout:15000,timeoutMsg:'Native bridge did not initialize'});
  if(isolated) {
   const s=await browser.execute(()=>window.__TAURI_INTERNALS__.invoke('state'));
   if(s.test_providers!==true)throw new Error('Refusing to run Stop tests against a non-test application');
  }
 },
 afterSession() { driver?.kill();updateServer?.kill(); },
};
