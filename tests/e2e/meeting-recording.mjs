// Opt-in real hardware acceptance; use only an isolated provider build.
import {browser,$,expect} from '@wdio/globals';
// Wrap valid domain objects with an error field so WebDriver does not treat them as protocol errors.
const command=async(name,args={})=>(await browser.execute(async(n,a)=>({result:await window.__TAURI_INTERNALS__.invoke(n,a)}),name,args)).result;
const suite=process.env.XTALK_E2E_CAPTURE==='1'?describe:describe.skip;
suite('Meeting native recording',()=>{
 before(async()=>{if(!(await command('state')).test_providers)throw Error('Requires isolated providers');});
 afterEach(async()=>{if((await command('meeting_capture_status')).id)await command('meeting_stop',{discard:true});});
 async function start(){
  await $('button[aria-label="Start meeting recording"]').click();
  await browser.waitUntil(async()=>(await command('meeting_capture_status')).active);
  await $('button=Mute microphone').click();
  await $('button=Mute system audio').waitForEnabled();
  await $('button=Mute system audio').click();
  await browser.waitUntil(async()=>{const s=await command('meeting_capture_status');return s.microphone_muted&&s.system_muted;});
 }
 it('shows the native-timed notice, blocks conflicting dictation, and saves on normal Stop',async()=>{
  await $('button[aria-label="Recorder"]').click();
  await $('.recorder-modes').$('button=Meeting').click();
  await start();
  const active=await command('meeting_capture_status');
  if(!active.active)throw Error('Meeting did not start');
  await expect($('.responsibility')).toHaveText("You are responsible for letting everyone know they're being recorded.");
  let rejected=false;try{await command('capture_action',{action:'start'});}catch{rejected=true;}
  if(!rejected||!(await command('meeting_capture_status')).active)throw Error('Dictation interfered with meeting capture');
  await browser.waitUntil(async()=>(await command('meeting_capture_status')).elapsed>=15.2,{timeout:20000,interval:250});
  await expect($('.responsibility')).not.toExist();
  await $('button[aria-label="Settings"]').click();
  await expect($('button=Stop & save')).toBeDisplayed();
  await $('button[aria-label="Recorder"]').click();
  await expect($('.recorder-modes').$('button=Dictation')).toBeDisabled();
  await $('button[aria-label="Compact view"]').click();
  await expect($('button=Unmute mic')).toBeDisplayed();
  await expect($('button=Unmute system')).toBeDisplayed();
  const fits=await browser.execute(()=>{const r=document.querySelector('button[aria-label="Stop meeting recording"]').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight;});
  if(!fits)throw Error('Compact meeting Stop is outside the window');
  await browser.saveScreenshot('.tmp/meeting-recorder-compact.png');
  await $('button[aria-label="Stop meeting recording"]').click();
  await browser.waitUntil(async()=>!(await command('meeting_capture_status')).id,{timeout:15000});
  await browser.waitUntil(async()=>{const view=await command('meeting_get',{id:active.id});return view.meeting.transcription_status==='complete';},{timeout:30000});
  const saved=await command('meeting_get',{id:active.id});
  if(!saved.meeting.audio_available||saved.meeting.duration_seconds<15||!saved.meeting.brief.content)throw Error('Stop failed to admit a complete meeting');
 });
 it('discards without creating a meeting or starting provider work',async()=>{
  await $('button[aria-label="Expand workspace"]').click();
  const before=(await command('meetings_list')).length;
  await start();
  await browser.pause(500);
  await $('button[aria-label="Cancel meeting recording"]').click();
  await $('button=Discard recording').click();
  await browser.waitUntil(async()=>!(await command('meeting_capture_status')).id);
  if((await command('meetings_list')).length!==before)throw Error('Discard saved a meeting');
  if((await command('meeting_capture_status')).id)throw Error('Discard left capture active');
 });
});
