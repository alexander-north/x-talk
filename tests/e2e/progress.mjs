import {browser,$,expect} from '@wdio/globals';
import {readdirSync,readFileSync,mkdirSync} from 'node:fs';
import path from 'node:path';
let diagnostics;
function reads(name){return readdirSync(diagnostics.directory).filter(n=>n.endsWith('.log')&&n.includes(diagnostics.session.slice(0,8))).flatMap(n=>readFileSync(path.join(diagnostics.directory,n),'utf8').split('\n')).filter(l=>l.includes('acceptance_'+name+'_read')).length;}
async function command(name,args={}){return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);}
describe('Lightweight recording progress',()=>{
 it('updates capture and final text without reloading devices, keys, or history pages',async()=>{
  const s=await command('state');if(!s.test_providers)throw Error('Requires isolated test providers');
  await command('save_settings',{settings:{...s.settings,theme:'day',transform:true,copy_raw:false,copy_transformed:false,sounds:{started:false,stopped:false,canceled:false,transcribed:false,transformed:false}}});
  await expect($('.app')).toHaveAttribute('data-theme','day');await browser.pause(500);
  diagnostics=s.diagnostics;
  const metadataBefore=reads('metadata');
  if(metadataBefore===0)throw Error('Acceptance metadata instrumentation is unavailable');
  const listener=await browser.execute(()=>{
   window.__spectrumFrames=[];
   return window.__TAURI_INTERNALS__.invoke('plugin:event|listen',{event:'capture-spectrum',target:{kind:'Any'},handler:window.__TAURI_INTERNALS__.transformCallback(e=>window.__spectrumFrames.push(e.payload))});
  });
  await $('button[aria-label="Start recording"]').click();
  await expect($('button[aria-label="Stop recording"]')).toBeDisplayed();await browser.pause(400);
  await browser.waitUntil(()=>browser.execute(()=>window.__spectrumFrames.length>=3),{timeout:3000,timeoutMsg:'Native spectrum events did not arrive'});
  const valid=await browser.execute(()=>window.__spectrumFrames.every(b=>b.length===48&&b.every(v=>Number.isFinite(v)&&v>=0&&v<=1)));
  if(!valid)throw Error('Native frequency spectrum exceeded its bounded payload');
  await expect($('.glass-orb')).toHaveAttribute('data-phase','recording');
  const clear=await browser.execute(()=>{
   const square=document.querySelector('.stop-glyph').getBoundingClientRect();
   const timer=document.querySelector('.record-timer').getBoundingClientRect();
   return timer.top>=square.top&&timer.bottom<=square.bottom&&timer.left>=square.left&&timer.right<=square.right&&!document.querySelector('.record-instruction');
  });
  if(!clear)throw Error('Timer must fit inside the Stop square without an external status label');
  mkdirSync('.tmp/glass',{recursive:true});await browser.saveScreenshot('.tmp/glass/recording.png');
  await $('button[aria-label="Stop recording"]').click();
  await browser.execute(async id=>{window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener('capture-spectrum',id);await window.__TAURI_INTERNALS__.invoke('plugin:event|unlisten',{event:'capture-spectrum',eventId:id});},listener);
  await browser.waitUntil(async()=>{
   const p=await command('progress');return p.latest?.transformed==='Acceptance transformed output.';
  },{timeout:10000});
  await expect($('[aria-label="Transformed output"]')).toHaveText('Acceptance transformed output.');
  await browser.pause(200);
  if(reads('metadata')!==metadataBefore)throw Error('Hot path requested heavyweight metadata');
  if(reads('progress')===0)throw Error('Progress instrumentation is unavailable');
  const p=await command('progress');
  for(const name of ['settings','devices','credentials','models','transformation'])if(name in p)throw Error('Progress exposed unnecessary metadata: '+name);
  const state=await command('state');
  await browser.waitUntil(()=>reads('metadata')>metadataBefore,{timeout:5000});
  const configurationBefore=reads('metadata');
  await command('save_settings',{settings:{...state.settings,theme:'night'}});
  await expect($('.app')).toHaveAttribute('data-theme','night');
  await browser.waitUntil(()=>reads('metadata')>configurationBefore,{timeout:5000,timeoutMsg:'Configuration change did not reload metadata'});
 });
 it('measures progress and metadata response latency separately',async()=>{
  const result=await browser.execute(async()=>{
   const samples={state:[],progress:[]};
   for(let i=0;i<12;i++)for(const name of ['state','progress']){
    const start=performance.now();await window.__TAURI_INTERNALS__.invoke(name);samples[name].push(performance.now()-start);
   }
   return Object.fromEntries(Object.entries(samples).map(([name,values])=>[name,values.sort((a,b)=>a-b)[6]]));
  });
  console.log('Native IPC median milliseconds:',JSON.stringify(result));
 });
});
