// Generated interrupted capture only: no microphone and no real providers.
import {browser,$,expect} from '@wdio/globals';
import {readFileSync,existsSync} from 'node:fs';
import path from 'node:path';
// Wrap valid domain objects with an error field so WebDriver does not treat them as protocol errors.
const command=async(name,args={})=>(await browser.execute(async(n,a)=>({result:await window.__TAURI_INTERNALS__.invoke(n,a)}),name,args)).result;
describe('Meeting crash recovery',()=>{
 it('archives recovered audio durably and waits for explicit transcription',async()=>{
  const state=await command('state');if(!state.test_providers)throw Error('Requires isolated providers');
  const fixture=JSON.parse(readFileSync('.tmp/meeting-recovery-profile.json','utf8'));
  let view;
  await browser.waitUntil(async()=>{
   const rows=await command('meetings_list');if(!rows.some(row=>row.id===fixture.id))return false;
   view=await command('meeting_get',{id:fixture.id});return view.meeting.transcription_status==='recovered';
  },{timeout:20000});
  if(view.meeting.transcript||view.meeting.duration_seconds!==5||!view.meeting.audio_available)throw Error('Incorrect recovered meeting');
  if(view.meeting.capture_warning!=='Synthetic recovery warning')throw Error('Capture warning was lost');
  await browser.waitUntil(async()=>!existsSync(path.join(fixture.directory,'microphone.f32')),{timeout:5000});
  for(const name of ['prepared.m4a','microphone.m4a','admission.json'])if(!existsSync(path.join(fixture.directory,name)))throw Error(`Missing durable recovery asset ${name}`);
  await browser.pause(11000); // Exercise at least one periodic worker pass.
  view=await command('meeting_get',{id:fixture.id});
  if(view.meeting.transcription_status!=='recovered'||view.meeting.transcript)throw Error('Recovery automatically submitted audio');
  await $('button[aria-label="Meetings"]').click();
  await $('button=Recovered meeting').click();
  await expect($('.meetings-page')).toHaveText(expect.stringContaining('Synthetic recovery warning'));
  await command('meeting_edit',{id:fixture.id,revision:view.meeting.revision,edit:{kind:'retry',transcription:true}});
  await browser.waitUntil(async()=>{
   view=await command('meeting_get',{id:fixture.id});return view.meeting.transcription_status==='complete';
  },{timeout:30000});
  if(!view.meeting.transcript||!view.meeting.brief.content)throw Error('Explicit recovery processing did not complete');
 });
});
