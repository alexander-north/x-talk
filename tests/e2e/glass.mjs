import {browser,$,expect} from '@wdio/globals';
import {mkdirSync,writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
const command=(name,args={})=>browser.execute((n,a)=>window.__TAURI_INTERNALS__.invoke(n,a),name,args);
const rejected=async(name,args={})=>browser.execute(async(n,a)=>{try{await window.__TAURI_INTERNALS__.invoke(n,a);return false;}catch{return true;}},name,args);
const phase=()=>$('.glass-orb').getAttribute('data-phase');
const source=path.resolve('.tmp/glass/synthetic.wav');
async function importRecording(complete=true){
 const previous=new Set((await command('history',{offset:0})).map(e=>e.id));
 await $('button=Import audio').click();
 execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('../../scripts/native-dialog.ps1',import.meta.url)),'-Path',source],{timeout:40000});
 let entry;
 await browser.waitUntil(async()=>{entry=(await command('history',{offset:0})).find(e=>!previous.has(e.id));return !!entry;});
 if(complete){
  await browser.waitUntil(async()=>!(await command('state')).operation_busy,{timeout:15000});
  entry=await command('entry',{id:entry.id});
  if(!entry.transcript||!entry.transformed)throw Error('Independent recording fixture did not complete');
  await browser.waitUntil(async()=>await phase()==='idle',{timeout:7000});
 }
 return entry;
}
describe('Warm glass recorder and serial processing',()=>{
 before(async()=>{
  const s=await command('state');if(!s.test_providers)throw Error('Isolated backend required');
  await command('save_settings',{settings:{...s.settings,theme:'day',transform:true,copy_raw:false,copy_transformed:false,sounds:{started:false,stopped:false,canceled:false,transcribed:false,transformed:false}}});
  mkdirSync('.tmp/glass',{recursive:true});
  const wav=Buffer.alloc(44+24000*2);wav.write('RIFF');wav.writeUInt32LE(wav.length-8,4);wav.write('WAVEfmt ',8);wav.writeUInt32LE(16,16);wav.writeUInt16LE(1,20);wav.writeUInt16LE(1,22);wav.writeUInt32LE(24000,24);wav.writeUInt32LE(48000,28);wav.writeUInt16LE(2,32);wav.writeUInt16LE(16,34);wav.write('data',36);wav.writeUInt32LE(wav.length-44,40);
  for(let i=0;i<24000;i++)wav.writeInt16LE(Math.round(4000*Math.sin(2*Math.PI*440*i/24000)),44+i*2);
  writeFileSync(source,wav);
 });
 afterEach(async()=>{
  if((await command('state')).operation_busy){
   await $('button[aria-label="Cancel processing"]').click();
   await browser.waitUntil(async()=>!(await command('state')).operation_busy);
  }
 });
 it('renders the idle glass control and respects the motion preference',async()=>{
  await browser.waitUntil(async()=>await phase()==='idle');
  await browser.waitUntil(()=>browser.execute(()=>!document.hidden),{timeout:5000,timeoutMsg:'Recorder window remained hidden'});
  const environment=await browser.execute(()=>{
   const canvas=document.querySelector('.glass-orb canvas');
   return {reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,visibility:document.visibilityState,width:canvas.width,height:canvas.height,userAgent:navigator.userAgent};
  });
  writeFileSync('.tmp/glass/environment.json',JSON.stringify(environment,null,2));
  await browser.waitUntil(()=>browser.execute(()=>{
   const c=document.querySelector('.glass-orb canvas');
   return c.width>0&&c.height>0&&c.getContext('2d').getImageData(0,0,c.width,c.height).data.some((v,i)=>i%4===3&&v>0);
  }),{timeout:5000,timeoutMsg:'Glass canvas remained blank'});
  const firstFrame=await browser.execute(()=>document.querySelector('.glass-orb canvas').toDataURL());
  if(environment.reduced){
   await browser.pause(350);
   const nextFrame=await browser.execute(()=>document.querySelector('.glass-orb canvas').toDataURL());
   if(nextFrame!==firstFrame)throw Error('Reduced-motion idle animation should remain still');
  }else{
   await browser.waitUntil(async()=>await browser.execute(()=>document.querySelector('.glass-orb canvas').toDataURL())!==firstFrame,{timeout:5000,timeoutMsg:'Idle animation did not advance with motion enabled'});
  }
  await browser.saveScreenshot('.tmp/glass/idle.png');
 });
 it('imports into a locked transcription state and preserves audio on cancellation',async()=>{
  const entry=await importRecording(false);
  await expect($('.glass-orb')).toBeDisabled();
  if(!await rejected('capture_action',{action:'start'}))throw Error('Recording was admitted during processing');
  if(!await rejected('import_audio'))throw Error('Second import was admitted');
  if(!await rejected('retry',{id:entry.id,revision:entry.revision,kind:'transcribe'}))throw Error('Retry was admitted during processing');
  await $('button[aria-label="Cancel processing"]').click();
  await browser.waitUntil(async()=>!(await command('state')).operation_busy);
  if(!(await command('entry',{id:entry.id})).audio)throw Error('Cancel lost saved original');
 });
 it('shows the iris then transformation, and cancellation preserves the completed transcript',async()=>{
  let entry=await importRecording();
  await command('retry',{id:entry.id,revision:entry.revision,kind:'transcribe'});
  await browser.waitUntil(async()=>await phase()==='transcribing');
  await browser.pause(450);await browser.saveScreenshot('.tmp/glass/transcribing.png');
  await browser.waitUntil(async()=>!(await command('state')).operation_busy);
  entry=await command('entry',{id:entry.id});
  if(!entry.transcript)throw Error('Transcript did not complete');
  await command('retry',{id:entry.id,revision:entry.revision,kind:'transform'});
  await browser.waitUntil(async()=>await phase()==='transforming');
  await browser.pause(600);await browser.saveScreenshot('.tmp/glass/transforming.png');
  await $('button[aria-label="Cancel processing"]').click();
  await browser.waitUntil(async()=>!(await command('state')).operation_busy);
  const saved=await command('entry',{id:entry.id});
  if(saved.transcript!==entry.transcript||saved.transformed!==entry.transformed||!saved.audio)throw Error('Canceled transformation changed completed work');
  await expect($('.glass-orb')).toHaveAttribute('data-phase','idle');
 });
 it('celebrates completion without holding the operation lock, then settles to idle',async()=>{
  const entry=await importRecording();
  await command('retry',{id:entry.id,revision:entry.revision,kind:'transform'});
  await browser.waitUntil(async()=>await phase()==='complete',{timeout:12000});
  if((await command('state')).operation_busy)throw Error('Celebration blocked recording');
  await expect($('button[aria-label="Start recording"]')).toBeEnabled();
  await expect($('[aria-label="Transformed output"]')).toHaveText('Acceptance transformed output.');
  await browser.pause(650);
  await browser.saveScreenshot('.tmp/glass/complete.png');
  await browser.waitUntil(async()=>await phase()==='idle',{timeout:7000});
 });
 it('copies whole cards while displaying only the final fitting lines',async()=>{
  let entry=await importRecording();
  const text=Array.from({length:80},(_,i)=>`Sentence ${i+1}: café and a thoughtful idea.\n`).join('')+'Final words 👩🏽‍💻.';
  await command('edit_recording',{id:entry.id,revision:entry.revision,title:entry.title,subtitle:entry.subtitle??'',transcript:text});
  await browser.waitUntil(()=>browser.execute(()=>document.querySelector('.home-outputs').getAnimations({subtree:true}).every(a=>a.playState!=='running')));
  const rear=$('button[aria-label="Show transcript"]');
  if(await rear.isExisting()){
   const point=await browser.execute(()=>{const r=document.querySelector('button[aria-label="Show transcript"]').getBoundingClientRect();return {x:Math.round(r.left+70),y:Math.round(r.top+18)};});
   await browser.action('pointer').move({...point,origin:'viewport'}).down().up().perform();
  }
  try{
   for(const width of [1100,800,460]){
    await browser.setWindowSize(width,800);
    await browser.waitUntil(()=>browser.execute(()=>{
     const n=document.querySelector('.transcript-card .home-output');
     return n.textContent.startsWith('… ')&&n.textContent.endsWith('Final words 👩🏽‍💻.')&&n.scrollHeight<=n.clientHeight+1&&n.scrollWidth<=n.clientWidth+1;
    }),{timeoutMsg:'Card must keep final text visible without scrollbars'});
   }
   const cursor=await browser.execute(()=>getComputedStyle(document.querySelector('button[aria-label="Copy transcript"]')).cursor);
   if(cursor!=='pointer')throw Error('Copy card is missing pointer affordance');
   await $('button[aria-label="Copy transcript"]').click();
   const clipboard=()=>Buffer.from(execFileSync('powershell.exe',['-NoProfile','-Command',"[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Get-Clipboard -Raw)))"],{encoding:'utf8'}).trim(),'base64').toString('utf8').replace(/\r\n/g,'\n').trimEnd();
   if(clipboard()!==text)throw Error('Card copied truncated text instead of the complete transcript');
   await command('copy_text',{id:null,kind:null,text:'sentinel'});
   await browser.execute(()=>document.querySelector('button[aria-label="Copy transcript"]').focus());
   await browser.keys('Space');
   await browser.waitUntil(async()=>clipboard()===text,{timeoutMsg:'Keyboard card activation did not copy full text'});
   await browser.saveScreenshot('.tmp/glass/tail-copy.png');
  }finally{await browser.setWindowSize(1100,800);}
 });
 it('keeps cards readable in dark mode and controls within compact bounds',async()=>{
  await importRecording();
  const s=await command('state');await command('save_settings',{settings:{...s.settings,theme:'night'}});
  await browser.waitUntil(async()=>await $('.app').getAttribute('data-theme')==='night');
  await browser.saveScreenshot('.tmp/glass/dark.png');
  await $('button[aria-label="Compact view"]').click();
  await expect($('.app')).toHaveElementClass('compact');
  const fits=await browser.execute(()=>{const r=document.querySelector('.glass-orb').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight&&r.left>=0&&r.right<=innerWidth;});
  if(!fits)throw Error('Compact control exceeds viewport');
  await browser.saveScreenshot('.tmp/glass/compact.png');
  await $('button[aria-label="Expand workspace"]').click();
  await expect($('.home-outputs')).toBeDisplayed();
 });
});
