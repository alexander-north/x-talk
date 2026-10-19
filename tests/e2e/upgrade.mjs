// Only for disposable hosted runner profiles. Never run against an owner's installation.
import {browser} from '@wdio/globals';
import {readFileSync} from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
const command=(name,args={})=>browser.execute((n,a)=>window.__TAURI_INTERNALS__.invoke(n,a),name,args);
describe('Installed upgrade persistence',()=>{
 before(()=>{if(process.env.GITHUB_ACTIONS!=='true'||!['seed','verify'].includes(process.env.XTALK_UPGRADE_PHASE))throw Error('Upgrade checks require an explicitly selected disposable CI profile');});
 it('preserves preferences, transformation, and credentials across installer replacement',async()=>{
  const state=await command('state');
  if(state.test_providers)throw Error('Must exercise a production installer');
  const expected=process.env.XTALK_EXPECTED_VERSION||JSON.parse(readFileSync(new URL('../../package.json',import.meta.url),'utf8')).version;
  if(state.diagnostics.version!==expected)throw Error('Installed app version does not match the source');
  const installed=path.dirname(process.env.XTALK_APP_PATH);
  if(!readFileSync(path.join(installed,'LICENSE'),'utf8').includes('Apache License')||!readFileSync(path.join(installed,'NOTICE'),'utf8').includes('Alexander North'))throw Error('Installer omitted application license notices');
  if(process.env.XTALK_UPGRADE_PHASE==='seed'){
   if(Object.values(state.credentials).some(Boolean))throw Error('Refusing a profile with existing credentials');
   await command('save_settings',{settings:{...state.settings,theme:'night',keep_audio:37}});
   await command('save_transformation',{transformation:{...state.transformation,system:'Installer upgrade acceptance marker.'}});
   await command('credential',{provider:'openai',action:'set',key:'not-a-real-key-upgrade-fixture'});
  }else{
   if(state.settings.theme!=='night'||state.settings.keep_audio!==37||state.transformation.system!=='Installer upgrade acceptance marker.'||!state.credentials.openai)throw Error('Installer replacement lost persisted data');
   const entry=await command('entry',{id:'00000000-0000-4000-8000-000000000141'});
   if(entry.transcript!=='Saved raw text before upgrade.'||entry.transformed!=='Saved transformed text before upgrade.'||!entry.audio||readFileSync(entry.audio).length!==48044)throw Error('Installer replacement lost history or audio');
   if(process.env.XTALK_UPGRADE_CLEANUP==='1')await command('credential',{provider:'openai',action:'delete',key:null});
  }
 });
 after(async()=>{
  const pid=(await command('state')).diagnostics.pid;
  await browser.execute(()=>setTimeout(()=>window.__TAURI_INTERNALS__.invoke('quit',{discardCapture:false}),200));
  execFileSync('powershell.exe',['-NoProfile','-Command',`$p=Get-Process -Id ${pid} -ErrorAction SilentlyContinue; if($p -and !$p.WaitForExit(15000)){throw 'App did not quit'}; exit 0`],{timeout:20000});
 });
});
