// Hosted-CI coverage using actual native import/storage/UI, with no microphone or paid requests.
import {browser,$,expect} from '@wdio/globals';
import {mkdtempSync,writeFileSync,mkdirSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
const command=(name,args={})=>browser.execute((n,a)=>window.__TAURI_INTERNALS__.invoke(n,a),name,args);
let entry;
describe('Offline native acceptance',()=>{
 it('recovers damaged journals and loads isolated settings without a microphone',async()=>{
  const state=await command('state');
  if(state.test_providers!==true||state.capture.active)throw Error('Expected idle isolated acceptance build');
  if(state.recovery_warnings.length<1)throw Error('Damaged journals were not reported');
  await command('save_settings',{settings:{...state.settings,transcription_mode:'realtime',transcription:{provider:'elevenlabs',model:'scribe_v2',prompt:'',temperature:null},transform:true,copy_raw:false,copy_transformed:false,sounds:{started:false,stopped:false,canceled:false,transcribed:false,transformed:false}}});
  await expect($('button[aria-label="Cancel recording"]')).not.toExist();
 });
 it('imports a generated WAV through the native picker and completes both stages',async()=>{
  const dir=mkdtempSync(path.join(tmpdir(),'xtalk-fixture-'));
  const wav=Buffer.alloc(44+24000*2);
  wav.write('RIFF');wav.writeUInt32LE(wav.length-8,4);wav.write('WAVEfmt ',8);
  wav.writeUInt32LE(16,16);wav.writeUInt16LE(1,20);wav.writeUInt16LE(1,22);
  wav.writeUInt32LE(24000,24);wav.writeUInt32LE(48000,28);wav.writeUInt16LE(2,32);wav.writeUInt16LE(16,34);
  wav.write('data',36);wav.writeUInt32LE(wav.length-44,40);
  for(let i=0;i<24000;i++)wav.writeInt16LE(Math.round(4000*Math.sin(2*Math.PI*440*i/24000)),44+i*2);
  const source=path.join(dir,'synthetic.wav');writeFileSync(source,wav);
  await $('button=Import audio').click();
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('../../scripts/native-dialog.ps1',import.meta.url)),'-Path',source],{timeout:40000});
  await browser.waitUntil(async()=>{entry=(await command('history',{offset:0}))[0];return !!entry?.transformed;},{timeout:15000});
  if(entry.transcript!=='Acceptance raw transcript.'||entry.transformed!=='Acceptance transformed output.')throw Error('Incorrect pipeline output');
  if(entry.transcription_ms===null||entry.transformation_ms<1100)throw Error('Stage timings missing');
  const job=(await command('state')).jobs.find(j=>j.entry_id===entry.id);
  if(job.delivery!=='manual_only')throw Error('Import must not copy automatically');
  if(entry.transcript_model!=='elevenlabs / scribe_v2')throw Error('Import did not use batch mode');
  await expect($('[aria-label="Transformed output"]')).toHaveText(entry.transformed);
  mkdirSync('.tmp',{recursive:true});await browser.saveScreenshot('.tmp/offline-recorder.png');
 });
 it('searches both saved outputs, preserves stale transformations after editing, and confirms Clear All',async()=>{
  await $('button[aria-label="Recordings"]').click();
  for(const search of ['raw transcript','transformed output']){
   const page=await command('recordings',{query:{search,sort:'created',descending:true,offset:0}});
   if(page.total!==1)throw Error('Search failed for '+search);
  }
  await $('button[title="Edit recording"]').click();await $('dialog textarea').setValue('Changed raw words');await $('button=Save changes').click();
  await expect($('.badge.stale')).toHaveText('Out of date');
  await $('input[aria-label="Search recordings"]').setValue('no match');
  await $('button=Clear All').click();await $('button=Cancel').click();
  if((await command('history',{offset:0})).length!==1)throw Error('Canceled confirmation deleted data');
  await $('button=Clear All').click();await $('button=Clear all recordings').click();
  if((await command('history',{offset:0})).length!==0)throw Error('Clear All was restricted by search');
 });
 it('confirms key removal, clears status, and filters unavailable providers',async()=>{
  await $('button[aria-label="Settings"]').click();await $('button=API Keys').click();
  await $('button[aria-label="Remove Anthropic key"]').click();await $('button=Cancel').click();
  if(!(await command('state')).credentials.anthropic)throw Error('Cancel removed key');
  await $('button[aria-label="Remove Anthropic key"]').click();await $('button=Remove key').click();
  await browser.waitUntil(async()=>!(await command('state')).credentials.anthropic);
  await browser.waitUntil(async()=>await browser.execute(()=>document.querySelector('.save-state').textContent.trim()===''),{timeout:4000});
  await $('button=Transformation').click();await expect($('.transformation-details select option[value="anthropic"]')).not.toExist();
 });
 it('persists the real-time setting and returns to batch when changing providers',async()=>{
  await $('button=Providers').click();
  const mode=$('select[aria-label="Recording transcription"]');
  await expect(mode).toHaveValue('realtime');
  await mode.selectByAttribute('value','batch');
  await browser.waitUntil(async()=>(await command('state')).settings.transcription_mode==='batch');
  await mode.selectByAttribute('value','realtime');
  await browser.waitUntil(async()=>(await command('state')).settings.transcription_mode==='realtime');
  await $('.two-col label:first-child select').selectByAttribute('value','openai');
  await browser.waitUntil(async()=>(await command('state')).settings.transcription_mode==='batch');
  await expect(mode.$('option[value="realtime"]')).toBeDisabled();
 });
 it('quits without hanging',async()=>{
  const pid=(await command('state')).diagnostics.pid;
  await browser.execute(()=>setTimeout(()=>window.__TAURI_INTERNALS__.invoke('quit',{discardCapture:false}),200));
  execFileSync('powershell.exe',['-NoProfile','-Command',`$p=Get-Process -Id ${pid} -ErrorAction SilentlyContinue; if($p -and !$p.WaitForExit(15000)){throw 'App did not quit'}; exit 0`],{timeout:20000});
 });
});
