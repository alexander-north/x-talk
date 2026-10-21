import { browser, $, $$, expect } from '@wdio/globals';
import { readFileSync, mkdirSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
async function command(name, args={}) { return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args); }
describe('Windows desktop application', () => {
 after(async()=>{const s=await command('state');await command('save_settings',{settings:{...s.settings,theme:'system',keep_audio:100,transform:false}});await $('button[aria-label="Transcripts"]').click();mkdirSync('.tmp',{recursive:true});await browser.saveScreenshot('.tmp/acceptance-ui.png');});
 it('loads bundled UI and authoritative native state', async () => {
  await expect($('button[aria-label="Recorder"] img')).toHaveAttribute('alt','x-talk');
  const s=await command('state');
  if(s.settings.keep_audio!==100 || s.settings.transform!==false) throw new Error('Unexpected initial preferences');
  if(s.models.filter(m=>m.stage==='transcription').length<4) throw new Error('Missing transcription adapters');
  const expectedSystem=readFileSync(new URL('../../prompts/conservative-cleanup.system.md',import.meta.url),'utf8');
  if(s.transformation.system!==expectedSystem) throw new Error('Cleanup seed changed');
  if(!(await command('history',{offset:0})).length)await expect($('[aria-label="Transcript output"]')).toHaveText('Your words will appear here after recording.');
 });
 it('shows provider capabilities and all independent sound controls', async () => {
  await $('button[aria-label="Settings"]').click();await $('button=Providers').click();
  await expect((await $$('select'))[0]).toExist();await $('button=API Keys').click();await expect($('input[type=password]')).not.toExist();
  await $('button=Delivery and Sounds').click();
  if((await $$('input[type=checkbox]')).length!==7) throw new Error('Expected two delivery and five sound controls');
  await $('button=Transformation').click();
  await expect((await $$('textarea.prompt'))[0]).toHaveValue(expect.stringContaining('conservative speech-to-text transcript cleaner'));
 });
 it('persists preferences and rejects stale revision updates', async () => {
  const s=await command('state');const settings={...s.settings,theme:'night',keep_audio:1};
  const saved=await command('save_settings',{settings});
  if(saved.theme!=='night'||saved.revision!==settings.revision+1)throw new Error('Preferences did not persist');
  const rejected=await browser.execute(async(settings)=>{try{await window.__TAURI_INTERNALS__.invoke('save_settings',{settings});return false;}catch{return true;}},settings);if(!rejected)throw new Error('Stale settings accepted');
  await browser.waitUntil(async()=>await browser.execute(()=>document.querySelector('.app').dataset.theme==='night'));
  await expect($('.app')).toHaveAttribute('data-theme','night');
 });
 it('writes Unicode and multiline clipboard text through the Rust boundary', async () => {
  const text='x-talk ✓ — 日本語\nLine two: do not change 20,000.';
  await command('copy_text',{id:null,kind:null,text});
  const output=execFileSync('powershell.exe',['-NoProfile','-Command',"[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Get-Clipboard -Raw)))"],{encoding:'utf8'}).trim();
  if(Buffer.from(output,'base64').toString('utf8').replace(/\r\n/g,'\n').trimEnd()!==text)throw new Error('Clipboard mismatch');
 });
 it('activates the same instance on a second launch',async()=>{
  const application=process.env.XTALK_APP_PATH || fileURLToPath(new URL('../../src-tauri/target/debug/x-talk.exe',import.meta.url));
  const escaped=application.replaceAll("'","''");
  execFileSync('powershell.exe',['-NoProfile','-Command',`$p=Start-Process '${escaped}' -PassThru; if(!$p.WaitForExit(10000)){throw 'Second instance did not exit'}`]);
  if(!(await command('state')).settings)throw new Error('Existing instance became unavailable');
 });
 if(process.env.XTALK_E2E_LIVE==='1') it('imports through the native dialog, saves output and plays/exports original audio', async () => {
  await $('button[aria-label="Recorder"]').click();
  const current=await command('state');
  const existing=new Set((await command('history',{offset:0})).map(e=>e.id));
  await command('save_settings',{settings:{...current.settings,transform:true}});
  await $('button=Import audio').click();
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('../../scripts/native-dialog.ps1',import.meta.url)),'-Path',process.env.XTALK_E2E_AUDIO],{encoding:'utf8'});
  await browser.waitUntil(async()=>{const list=await command('history',{offset:0});return list.some(e=>!existing.has(e.id)&&!!e.transformed);},{timeout:120000,timeoutMsg:'Import pipeline did not finish'});
  const entry=(await command('history',{offset:0})).find(e=>!existing.has(e.id)&&e.transformed);
  if(!entry.audio||!entry.transcript||!entry.transformed)throw new Error('Import did not preserve complete stage results');
  const s=await command('state');const job=s.jobs.find(j=>j.entry_id===entry.id);
  if(job.delivery!=='manual_only')throw new Error('Import copied automatically');
  await command('play_audio',{id:entry.id});await browser.pause(500);
  if((await command('state')).playing!==entry.id)throw new Error('Original playback did not stay active');
  await command('stop_playback');
  const destination=fileURLToPath(new URL('../../.tmp/e2e-export.wav',import.meta.url));
  mkdirSync('.tmp',{recursive:true});rmSync(destination,{force:true});
  await browser.execute(id=>{window.__e2eExport=null;window.__TAURI_INTERNALS__.invoke('export_audio',{id}).then(()=>window.__e2eExport='done').catch(()=>window.__e2eExport='failed');},entry.id);
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('../../scripts/native-dialog.ps1',import.meta.url)),'-Path',destination],{encoding:'utf8'});
  await browser.waitUntil(async()=>await browser.execute(()=>window.__e2eExport==='done'),{timeout:15000,timeoutMsg:'Original export failed'});
  if(!readFileSync(destination).equals(readFileSync(process.env.XTALK_E2E_AUDIO)))throw new Error('Export changed the original audio');
  await expect($('.banner.error')).not.toExist();
  mkdirSync('.tmp',{recursive:true});
  await browser.saveScreenshot('.tmp/import-history.png');
  await command('delete_entry',{id:entry.id,revision:entry.revision,confirmed:true});
 });
 if(process.env.XTALK_E2E_CAPTURE==='1') it('opens the Windows microphone and cancels without creating history', async () => {
  const before=await command('history',{offset:0});
  await command('capture_action',{action:'start'});
  const active=await command('state');if(!active.capture.active)throw new Error('Capture did not become ready');
  await browser.pause(1000);await command('capture_action',{action:'cancel'});
  const after=await command('history',{offset:0});if(after.length!==before.length)throw new Error('Canceled capture created history');
 });
 if(process.env.XTALK_E2E_SHORTCUTS==='1') it('handles global shortcuts with another application focused',async()=>{
  const script=fileURLToPath(new URL('../../scripts/global-hotkey.ps1',import.meta.url));
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',script,'-Action','toggle']);
  await browser.waitUntil(async()=>(await command('state')).capture.active,{timeout:5000,timeoutMsg:'Toggle shortcut did not start capture'});
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',script,'-Action','cancel']);
  await browser.waitUntil(async()=>!(await command('state')).capture.active,{timeout:5000,timeoutMsg:'Cancel shortcut did not stop capture'});
 });

});
