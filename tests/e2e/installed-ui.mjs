// Non-recording release checks: preserve the owner's settings, recordings and clipboard.
import {browser,$,$$,expect} from '@wdio/globals';
import {readFileSync,readdirSync} from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
async function command(name,args={}){return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);}
let pid;
describe('Installed release workspace',()=>{
 before(async()=>{
  const state=await command('state');pid=state.diagnostics.pid;
  const version=JSON.parse(readFileSync(new URL('../../package.json',import.meta.url),'utf8')).version;
  if(state.diagnostics.version!==version||state.test_providers===true)throw new Error('Expected the matching production release');
  if(state.capture.active)throw new Error('Do not attach this suite to an active recording');
 });
 it('opens the bundled recorder with the supplied icon and available output cards',async()=>{
  await expect($('button[aria-label="Recorder"] img')).toHaveAttribute('alt','x-talk');
  // A previously saved compact geometry may intentionally hide outputs.
  const compact=await browser.execute(()=>document.querySelector('.app').classList.contains('compact'));
  if(!compact){
   await expect($('[aria-label="Transcript output"]')).toExist();const state=await command('state');const entries=await command('history',{offset:0});
   if(state.settings.transform&&entries[0]?.transformed)await expect($('.transformed-card.card-front')).toExist();else await expect($('.transformed-card')).not.toExist();
  }
  await expect($('button[aria-label="Cancel recording"]')).not.toExist();
  await expect($('button[aria-label="Start recording"]')).toExist();await expect($('header')).not.toExist();await expect($('footer')).not.toExist();
 });
 it('opens the persisted recordings table and executes its native full-library query',async()=>{
  await $('button[aria-label="Recordings"]').click();await expect($('.recordings-table')).toExist();
  const result=await command('recordings',{query:{search:'',sort:'created',descending:true,offset:0}});
  if(!Array.isArray(result.entries)||typeof result.total!=='number')throw new Error('Recordings query failed');
  await expect($('input[aria-label="Search recordings"]')).toExist();await expect($('summary')).toHaveText('Columns');
 });
 it('opens existing settings, diagnostic version/path, and all five sound previews',async()=>{
  await $('button[aria-label="Settings"]').click();await $('button=Delivery and Sounds').click();if((await $$('button[title="Preview sound"]')).length!==5)throw new Error('Expected five cue previews');
  await $('button=API Keys').click();await expect($('input[type=password]')).not.toExist();
  const current=await command('state');if(!current.transformation||'presets' in current)throw Error('Expected one transformation configuration');
  await $('button=Transformation').click();await expect($('.transformation-details')).toExist();await expect($('.preset-sidebar')).not.toExist();
  const matches=await browser.execute(system=>document.querySelector('textarea.prompt').value===system.replace(/\r\n/g,'\n'),current.transformation.system);if(!matches)throw Error('Active transformation was not preserved');
  await $('button=General').click();await expect($('#diagnostic-logging')).toExist();await expect($('button=Open logs folder')).toExist();
  const state=await command('state');if(!state.diagnostics.directory.toLowerCase().endsWith('\\x-talk\\logs')||state.diagnostics.error)throw new Error('Diagnostic folder/write status failed');
 });
 it('plays the five approved embedded cues without changing saved toggles',async()=>{
  const state=await command('state');
  for(const event of ['started','stopped','canceled','transcribed','transformed']){
   await command('preview_sound',{event});await browser.pause(550);
  }
  if(JSON.stringify((await command('state')).settings.sounds)!==JSON.stringify(state.settings.sounds))throw Error('Preview changed saved sound toggles');
  if(state.diagnostics.enabled){
   const log=()=>readdirSync(state.diagnostics.directory).filter(n=>n.endsWith('.log')&&n.includes(state.diagnostics.session.slice(0,8))).map(n=>readFileSync(path.join(state.diagnostics.directory,n),'utf8')).join('');
   await browser.waitUntil(()=>log().split('\n').filter(l=>l.includes('cue_started')&&l.includes('sound_set="v6"')).length>=5,{timeout:5000});
   if(log().includes('cue_playback_failed'))throw Error('Installed Windows playback failed');
  }
 });
 it('quits the installed release through the UI without hanging',async()=>{
  await browser.execute(()=>setTimeout(()=>window.__TAURI_INTERNALS__.invoke('quit',{discardCapture:false}),200));
  execFileSync('powershell.exe',['-NoProfile','-Command',`$p=Get-Process -Id ${pid} -ErrorAction SilentlyContinue; if($p -and !$p.WaitForExit(15000)){throw 'Installed app did not quit'}; exit 0`],{timeout:20000});
 });
});
