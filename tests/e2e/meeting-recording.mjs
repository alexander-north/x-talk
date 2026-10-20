// Opt-in real hardware acceptance; use only an isolated provider build.
import {browser,$,expect} from '@wdio/globals';
const command=(name,args={})=>browser.execute((n,a)=>window.__TAURI_INTERNALS__.invoke(n,a),name,args);
const suite=process.env.XTALK_E2E_CAPTURE==='1'?describe:describe.skip;
suite('Meeting native recording',()=>{
 before(async()=>{if(!(await command('state')).test_providers)throw Error('Requires isolated providers');});
 afterEach(async()=>{if((await command('meeting_capture_status')).id)await command('meeting_stop',{discard:true});});
 async function start(){
  await command('meeting_start',{options:{microphone:true,system:true,microphone_device:null,output_device:null}});
  await command('meeting_mute',{microphone:true,muted:true});
  await command('meeting_mute',{microphone:false,muted:true});
 }
 it('shows the native-timed notice, blocks conflicting dictation, and saves on normal Stop',async()=>{
  await $('button[aria-label="Meetings"]').click();
  await start();
  const active=await command('meeting_capture_status');
  if(!active.active)throw Error('Meeting did not start');
  await expect($('.responsibility')).toHaveText("You are responsible for letting everyone know they're being recorded.");
  let rejected=false;try{await command('capture_action',{action:'start'});}catch{rejected=true;}
  if(!rejected||!(await command('meeting_capture_status')).active)throw Error('Dictation interfered with meeting capture');
  await browser.waitUntil(async()=>(await command('meeting_capture_status')).elapsed>=15.2,{timeout:20000,interval:250});
  await expect($('.responsibility')).not.toExist();
  await $('button=Stop & save').click();
  await browser.waitUntil(async()=>!(await command('meeting_capture_status')).id,{timeout:15000});
  await browser.waitUntil(async()=>{const view=await command('meeting_get',{id:active.id});return view.meeting.transcription_status==='complete';},{timeout:30000});
  const saved=await command('meeting_get',{id:active.id});
  if(!saved.meeting.audio_available||saved.meeting.duration_seconds<15||!saved.meeting.brief.content)throw Error('Stop failed to admit a complete meeting');
 });
 it('discards without creating a meeting or starting provider work',async()=>{
  const before=(await command('meetings_list')).length;
  await start();
  await browser.pause(500);
  await command('meeting_stop',{discard:true});
  if((await command('meetings_list')).length!==before)throw Error('Discard saved a meeting');
  if((await command('meeting_capture_status')).id)throw Error('Discard left capture active');
 });
});
