import {browser,$,expect} from '@wdio/globals';
import {execFileSync} from 'node:child_process';
const command=(name,args={})=>browser.execute((n,a)=>window.__TAURI_INTERNALS__.invoke(n,a),name,args);
function clipboard(){
 const encoded=execFileSync('powershell.exe',['-NoProfile','-Command',"[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Get-Clipboard -Raw)))"],{encoding:'utf8'}).trim();
 return Buffer.from(encoded,'base64').toString('utf8').trimEnd();
}
describe('Real-time microphone recording with isolated providers',()=>{
 before(async()=>{
  const state=await command('state');
  if(state.test_providers!==true)throw Error('Real-time acceptance requires isolated fake providers');
  await command('save_settings',{settings:{...state.settings,transcription_mode:'realtime',transcription:{provider:'elevenlabs',model:'scribe_v2',prompt:'',temperature:null},transform:true,copy_raw:true,copy_transformed:true,sounds:{started:false,stopped:false,canceled:false,transcribed:false,transformed:false}}});
 });
 it('previews during recording, saves final text on Stop and transforms once',async()=>{
  const before=await command('history',{offset:0});
  await command('copy_text',{id:null,kind:null,text:'Real-time clipboard sentinel'});
  await command('capture_action',{action:'start'});
  await browser.waitUntil(async()=>(await command('progress')).capture.live?.text==='Acceptance live preview.');
  await expect($('[aria-label="Transcript output"]')).toHaveText('Acceptance live preview.');
  await expect($('[aria-label="Copy transcript"]')).toBeDisabled();
  if(clipboard()!=='Real-time clipboard sentinel')throw Error('Preview changed the clipboard');
  await browser.saveScreenshot('.tmp/realtime-live-preview.png');
  if((await command('history',{offset:0})).length!==before.length)throw Error('Preview was saved as a final transcript');
  await command('capture_action',{action:'stop'});
  await browser.waitUntil(async()=>clipboard()==='Acceptance final real-time transcript.',{timeout:5000,timeoutMsg:'Final raw transcript was not copied before transformation'});
  await browser.waitUntil(async()=>(await command('history',{offset:0}))[0]?.transformed==='Acceptance transformed output.');
  const entries=await command('history',{offset:0});
  if(entries.length!==before.length+1)throw Error('Stop did not admit exactly one recording');
  if(entries[0].transcript!=='Acceptance final real-time transcript.'||entries[0].transcript_model!=='elevenlabs / scribe_v2_realtime'||!entries[0].audio)throw Error('Final transcript or original audio missing');
  if((await command('progress')).capture.live)throw Error('Preview survived completed recording');
  await browser.waitUntil(async()=>clipboard()==='Acceptance transformed output.');
 });
 it('Cancel discards the preview and never admits the recording',async()=>{
  const before=await command('history',{offset:0});
  await command('copy_text',{id:null,kind:null,text:'Cancel clipboard sentinel'});
  await command('capture_action',{action:'start'});
  await browser.waitUntil(async()=>!!(await command('progress')).capture.live?.text);
  await command('capture_action',{action:'cancel'});
  const progress=await command('progress');
  if(progress.capture.active||progress.capture.live)throw Error('Cancel did not clear capture');
  if((await command('history',{offset:0})).length!==before.length)throw Error('Cancel admitted a recording');
  if(clipboard()!=='Cancel clipboard sentinel')throw Error('Canceled recording changed the clipboard');
 });
});
