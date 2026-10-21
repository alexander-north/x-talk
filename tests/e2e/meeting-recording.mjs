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
  await expect($('meter[aria-label="Microphone level"]')).toBeDisplayed();
  await expect($('meter[aria-label="Microphone level"]')).toBeDisplayed();
  await expect($('meter[aria-label="System audio level"]')).toBeDisplayed();
  const before=(await command('meetings_list')).length;
  if((await command('meeting_capture_status')).id)throw Error('Preview started a recording');
  await $('button[aria-label="Settings"]').click();
  await $('button[aria-label="Recorder"]').click();
  await expect($('meter[aria-label="Microphone level"]')).toBeDisplayed();
  if((await command('meetings_list')).length!==before)throw Error('Preview admitted a meeting');
  await start();
  const cursor=await browser.execute(()=>getComputedStyle(document.querySelector('.recorder-modes button:disabled')).cursor);
  if(cursor!=='not-allowed')throw Error('Disabled control is missing forbidden cursor');
  await expect($('select[aria-label="Meeting microphone"]')).toBeEnabled();
  await expect($('select[aria-label="Meeting system output"]')).toBeEnabled();
  for(const label of ['Meeting microphone','Meeting system output']){
   await browser.execute(label=>document.querySelector(`select[aria-label="${label}"]`).dispatchEvent(new Event('change',{bubbles:true})),label);
   await browser.waitUntil(async()=>await $(`select[aria-label="${label}"]`).isEnabled());
  }
  const active=await command('meeting_capture_status');
  if(!active.active)throw Error('Meeting did not start');
  await expect($('.responsibility')).toHaveText("You are responsible for letting everyone know they're being recorded.");
  let rejected=false;try{await command('capture_action',{action:'start'});}catch{rejected=true;}
  if(!rejected||!(await command('meeting_capture_status')).active)throw Error('Dictation interfered with meeting capture');
  for(const height of [460,650]){
   await browser.setWindowSize(520,height);await browser.pause(100);
   const fits=await browser.execute(()=>{const main=document.querySelector('.meeting-recorder'),orb=main.querySelector('.record-orbit').getBoundingClientRect(),sources=main.querySelector('.meeting-sources').getBoundingClientRect();return {scroll:main.scrollHeight,height:main.clientHeight,orbTop:orb.top,orbBottom:orb.bottom,sourceTop:sources.top,noticeBottom:main.querySelector('.responsibility')?.getBoundingClientRect().bottom??0,top:main.getBoundingClientRect().top};});
   if(fits.scroll>fits.height+1||fits.orbBottom>fits.sourceTop||fits.noticeBottom>fits.sourceTop||fits.orbTop<fits.top){await browser.saveScreenshot('.tmp/active-meeting-overflow.png');throw Error(`Active meeting controls overflow at ${height}: ${JSON.stringify(fits)}`);}
  }
  await browser.setWindowSize(1100,800);
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
  await expect($('button=Open meeting')).toBeDisplayed();
  await expect($('.meeting-sources')).not.toExist();
  await $('button=Open meeting').click();
  await expect($('.meetings-page.detail-view')).toBeDisplayed();
  await $('button[aria-label="Recorder"]').click();
  await $('button[aria-label="Compact view"]').click();
  const saved=await command('meeting_get',{id:active.id});
  if(!saved.meeting.audio_available||saved.meeting.duration_seconds<15||!saved.meeting.brief.content)throw Error('Stop failed to admit a complete meeting');
 });
 it('discards without creating a meeting or starting provider work',async()=>{
  await $('button[aria-label="Expand workspace"]').click();
  await $('button=New recording').click();
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
