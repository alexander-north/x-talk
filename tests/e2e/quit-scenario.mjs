import { browser, $, expect } from '@wdio/globals';
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
async function command(name,args={}){return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);}
export function quitScenario(mode,realtime=false){
 describe(`Windows quit (${mode}${realtime?', real-time':''})`,()=>{
  it('exits without blocking the UI thread and flushes shutdown diagnostics',async()=>{
   const state=await command('state');
   if(!state.test_providers)throw new Error('Quit regression requires isolated test providers');
   const folder=state.diagnostics.directory;
   const audio=path.join(folder,'..','audio');
   const existing=new Set(readdirSync(audio));
   if(mode!=='idle'){
    const sounds={started:false,stopped:false,canceled:false,transcribed:false,transformed:false};
    await command('save_settings',{settings:{...state.settings,sounds,...(realtime?{transcription_mode:'realtime',transcription:{provider:'elevenlabs',model:'scribe_v2',prompt:'',temperature:null}}:{})}});
    await command('capture_action',{action:'start'});await browser.pause(1000);
    if(realtime)await browser.waitUntil(async()=>!!(await command('progress')).capture.live?.text);
   }
   const current=await command('state');await command('save_settings',{settings:{...current.settings,close_to_tray:false}});
   execFileSync('powershell.exe',['-NoProfile','-Command',`Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class NativeClose { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l); }'; $p=Get-Process -Id ${state.diagnostics.pid}; [NativeClose]::PostMessage($p.MainWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null`]);
   await expect($('dialog')).toBeDisplayed();if(mode==='idle')await expect($('dialog')).not.toHaveText(expect.stringContaining('Pending processing'));else await expect($('dialog')).toHaveText(expect.stringContaining('Choose whether to save or discard the active recording'));
   await browser.execute(discard=>{setTimeout(()=>document.querySelector(discard?'dialog button.danger':'dialog button.primary').click(),200);},mode==='discard');
   // Do not query the renderer once shutdown begins. Check the actual native process.
   execFileSync('powershell.exe',['-NoProfile','-Command',`$p=Get-Process -Id ${state.diagnostics.pid} -ErrorAction SilentlyContinue; if($p -and !$p.WaitForExit(15000)){throw 'Quit did not terminate the process'}; exit 0`],{timeout:20000});
   const text=readdirSync(folder).filter(n=>n.endsWith('.log')).map(n=>readFileSync(path.join(folder,n),'utf8')).join('');
   for(const event of ['shutdown_requested','shutdown_playback_stop_completed','shutdown_capture_stop_completed','shutdown_ready_to_exit']){
    if(!text.includes(event))throw new Error(`Missing flushed shutdown event: ${event}`);
   }
   const created=readdirSync(audio).filter(n=>!existing.has(n));
   if(mode==='stop'){
    if(created.length!==1||!created[0].endsWith('.wav')||statSync(path.join(audio,created[0])).size<=44)throw new Error('Stop and quit did not preserve the original');
    if(!text.includes('capture_admission_completed'))throw new Error('Stop and quit did not save the entry/job');
   }else if(created.length)throw new Error('Quit left unexpected capture files');
   if(text.includes('Acceptance raw transcript.')||text.includes('acceptance-placeholder'))throw new Error('Private content leaked into logs');
  });
 });
}
