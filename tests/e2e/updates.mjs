import {browser,$,expect} from '@wdio/globals';
import {writeFileSync,readFileSync} from 'node:fs';
import {execFileSync} from 'node:child_process';
// Wrap status objects: WebDriver treats a top-level `error` field as a protocol error.
const command=async(name,args={})=>(await browser.execute(async(n,a)=>({data:await window.__TAURI_INTERNALS__.invoke(n,a)}),name,args)).data;
const mode=value=>writeFileSync('.tmp/update-mode',value);
describe('Native signed updater',()=>{
 before(async()=>{const s=await command('state');if(!s.test_providers)throw Error('Requires isolated acceptance version');});
 it('preserves the automatic-check preference and recovers from unavailable and malformed feeds',async()=>{
  await $('button[aria-label="Settings"]').click();
  const checkbox=$('section[aria-label="Application updates"] input');await checkbox.click();
  await browser.waitUntil(async()=>!(await command('state')).settings.check_updates);
  for(const value of ['unavailable','malformed']){
   mode(value);await $('button=Check for updates').click();
   await browser.waitUntil(async()=>(await command('update_status')).phase==='error');
   await expect($('.error-text')).toBeDisplayed();
  }
  mode('current');await $('button=Check for updates').click();
  await browser.waitUntil(async()=>(await command('update_status')).phase==='current');
 });
 it('shows the offered version and notes, and requires explicit confirmation',async()=>{
  mode('available');await $('button=Check for updates').click();
  await browser.waitUntil(async()=>(await command('update_status')).phase==='available');
  const result=await browser.execute(async()=>{try{await window.__TAURI_INTERNALS__.invoke('install_update',{confirmed:false});return false;}catch{return true;}});
  if(!result)throw Error('Install skipped confirmation');
  await $('button=Update and restart').click();await expect($('dialog')).toBeDisplayed();await $('button=Not now').click();
  if((await command('update_status')).phase!=='available')throw Error('Cancel started installation');
 });
 it('refuses installation while a transformation test is running',async()=>{
  const transformation=(await command('state')).transformation;
  const result=await browser.execute(async transformation=>{
   const invoke=window.__TAURI_INTERNALS__.invoke;
   const test=invoke('test_transformation',{transformation,input:'Synthetic update concurrency test.'});
   await new Promise(resolve=>setTimeout(resolve,100));
   let rejected=false;try{await invoke('install_update',{confirmed:true});}catch(e){rejected=/processing|transformation/.test(e.message||String(e));}
   await test;return rejected;
  },transformation);
  if(!result)throw Error('Updater did not protect the active transformation test');
 });
 it('rejects corrupted or unsigned bytes without exiting or losing settings',async()=>{
  for(const value of ['tampered','wrong-version']){
   mode(value);await command('check_update');
   const result=await browser.execute(async()=>{try{await window.__TAURI_INTERNALS__.invoke('install_update',{confirmed:true});return false;}catch{return true;}});
   if(!result)throw Error('Invalid installer or mismatched signed version was accepted');
   if((await command('update_status')).phase!=='available')throw Error('Failed install did not allow retry');
  }
  const s=await command('state');if(s.settings.check_updates)throw Error('Failed update lost preference');
  await command('save_settings',{settings:{...s.settings,theme:'night'}});
 });
 it('hands a verified installer to NSIS and restarts the production app',async function(){
  if(process.env.XTALK_UPDATE_INSTALL!=='1'){this.skip();return;}
  if(process.env.GITHUB_ACTIONS!=='true')throw Error('Installer handoff requires disposable hosted CI');
  mode('available');await command('check_update');
  const pid=(await command('state')).diagnostics.pid;
  // The successful Windows handoff terminates the original process.
  await browser.execute(()=>setTimeout(()=>window.__TAURI_INTERNALS__.invoke('install_update',{confirmed:true}),300));
  const version=process.env.XTALK_EXPECTED_VERSION||JSON.parse(readFileSync('package.json','utf8')).version;
  execFileSync('powershell.exe',['-NoProfile','-Command',`
   $old=Get-Process -Id ${pid} -ErrorAction SilentlyContinue
   if($old -and !$old.WaitForExit(90000)){throw 'Updater did not exit'}
   $deadline=(Get-Date).AddSeconds(90)
   do {
    $restarted=Get-Process x-talk -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq "$env:LOCALAPPDATA\\x-talk\\x-talk.exe" }
    if($restarted){break};Start-Sleep -Milliseconds 500
   } while((Get-Date) -lt $deadline)
   if(!$restarted){throw 'Updated application did not restart'}
   if((Get-Item $restarted.Path).VersionInfo.ProductVersion -notlike '${version}*'){throw 'Restarted wrong version'}
   $restarted | Stop-Process
   exit 0
  `],{timeout:190000});
 });
});
