// Native media/storage/clipboard with deterministic providers and no microphone.
import {browser,$,expect} from '@wdio/globals';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {mkdirSync,statSync,existsSync,writeFileSync,truncateSync,unlinkSync} from 'node:fs';
// Wrap valid domain objects with an error field so WebDriver does not treat them as protocol errors.
const command=async(name,args={})=>(await browser.execute(async(n,a)=>({result:await window.__TAURI_INTERNALS__.invoke(n,a)}),name,args)).result;
const clipboard=()=>JSON.parse(execFileSync('powershell.exe',['-NoProfile','-Command','Get-Clipboard -Raw'],{encoding:'utf8'}));
let id,view;
describe('Meetings native acceptance',()=>{
 it('imports video through the picker with an explicit first-track notice',async()=>{
  const state=await command('state');if(!state.test_providers)throw Error('Requires isolated test providers');
  await $('button[aria-label="Meetings"]').click();
  await $('button=Import recording').click();
  await $('.drop-zone').click();
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('../../scripts/native-dialog.ps1',import.meta.url)),'-Path',path.resolve('src-tauri/tests/fixtures/meeting-two-tracks.mkv')],{timeout:40000});
  await expect($('.import-preview')).toHaveText(expect.stringContaining('Only the first audio track'));
  await $('input[aria-label="Import meeting title"]').setValue('Project review');
  mkdirSync('.tmp',{recursive:true});await browser.saveScreenshot('.tmp/meetings-import-dialog.png');
  await $('button=Create Transcript and Brief').click();
  await expect($('.meeting-card')).toBeDisplayed();
  await browser.waitUntil(async()=>{const rows=await command('meetings_list');id=rows[0]?.id;if(!id)return false;view=await command('meeting_get',{id});return view.meeting.transcription_status==='complete';},{timeout:60000});
  if(view.meeting.source!=='video'||!view.meeting.audio_available)throw Error('Video import lost audio provenance');
  await $('button=Open recording ↗').click();
  await expect($('.meeting-summary')).toHaveText('The speakers agreed to send an updated proposal tomorrow.');
 });
 it('copies full JSON from the brief, even when the transcript is filtered',async()=>{
  await $('button=Copy meeting JSON').click();
  const first=clipboard();
  if(first.format!=='x-talk.meeting'||first.transcript.turns.length!==2||!first.brief.content)throw Error('Incomplete JSON package');
  if(JSON.stringify(first).includes('fixture.invalid'))throw Error('Provider URL leaked into JSON');
  await $('button[role="tab"]=Transcript').click();
  await $('input[aria-label="Search transcript"]').setValue('will send');
  await $('button=Copy meeting JSON').click();
  if(JSON.stringify(first)!==JSON.stringify(clipboard()))throw Error('Transcript filter changed exported JSON');
 });
 it('persists corrections, marks the brief stale, and supports chained review operations',async()=>{
  view=await command('meeting_get',{id});
  view=await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'correct',turn:'seg_1',correction:{text:'Send the revised proposal.',speaker:'A'}}});
  if(!view.meeting.brief.stale)throw Error('Correction failed to stale the brief');
  view=await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'name',speaker:'B',name:'Taylor'}});
  view=await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'merge',from:'A',into:'B'}});
  if(view.meeting.transcript.speakers.length!==1||view.meeting.transcript.speakers[0].name!=='Taylor')throw Error('Speaker merge/name projection failed');
  view=await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'undo_merge'}});
  if(view.meeting.transcript.speakers.length!==2)throw Error('Merge undo failed');
  await command('meeting_copy',{id});
  const saved=clipboard();
  if(saved.transcript.turns[0].original_text===saved.transcript.turns[0].text)throw Error('Original evidence was replaced');
 });
 it('keeps meetings independent from dictation Clear All and refreshes a stale brief',async()=>{
  await command('clear_recordings',{confirmed:true});
  if((await command('meetings_list')).length!==1)throw Error('Dictation clear removed meetings');
  view=await command('meeting_get',{id});
  await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'retry',transcription:false}});
  await browser.waitUntil(async()=>{view=await command('meeting_get',{id});return view.meeting.transcription_status==='complete'&&!view.meeting.brief.stale;},{timeout:30000});
  await $('button[role="tab"]=Meeting brief').click();
  mkdirSync('.tmp',{recursive:true});await browser.saveScreenshot('.tmp/meetings-review.png');
 });
 it('downloads only audio and plays from a transcript timestamp',async()=>{
  await $('button[role="tab"]=Details').click();
  mkdirSync('.tmp',{recursive:true});
  const destination=path.resolve(`.tmp/meeting-export-${Date.now()}.m4a`);
  await $('button=Download audio').click();
  execFileSync('powershell.exe',['-NoProfile','-ExecutionPolicy','Bypass','-File',fileURLToPath(new URL('../../scripts/native-dialog.ps1',import.meta.url)),'-Path',destination],{timeout:40000});
  await browser.waitUntil(async()=>existsSync(destination)&&statSync(destination).size>0,{timeout:15000});
  const probe=JSON.parse(execFileSync(path.resolve('src-tauri/binaries/ffprobe.exe'),['-v','error','-show_entries','stream=codec_type','-of','json',destination],{encoding:'utf8'}));
  if(probe.streams.some(stream=>stream.codec_type==='video'))throw Error('Downloaded video instead of audio');
  await command('meeting_play',{id,start:1});
  const playback=await command('playback_state');
  if(playback?.id!==`meeting:${id}`)throw Error('Meeting playback was not admitted');
  await command('stop_playback');
 });
 it('replaces a transcript atomically and preserves a manual title',async()=>{
  view=await command('meeting_get',{id});
  const previous=view.meeting.transcript.generation;
  view=await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'metadata',title:'Meeting acceptance review',call_at:view.meeting.call_at}});
  view=await command('meeting_edit',{id,revision:view.meeting.revision,edit:{kind:'retry',transcription:true}});
  if(view.meeting.transcript.generation!==previous)throw Error('Retry cleared the old transcript before success');
  await browser.waitUntil(async()=>{view=await command('meeting_get',{id});return view.meeting.transcription_status==='complete'&&view.meeting.transcript.generation!==previous;},{timeout:60000});
  if(view.meeting.title!=='Meeting acceptance review')throw Error('Generated brief replaced manual title');
  if(view.meeting.transcript.turns[0].text!==view.meeting.transcript.turns[0].original_text)throw Error('Replacement retained stale correction');
  await command('meeting_copy',{id});
  const exported=clipboard();
  if(exported.brief.actual_model!=='fixture-brief')throw Error('Missing actual brief model provenance');
  await $('button=All meetings').click();
  await $('button=Copy meeting JSON').click();
  if(JSON.stringify(exported)!==JSON.stringify(clipboard()))throw Error('Library copy differs from detail copy');
 });

 it('moves Meetings above the divider and keeps its settings independent',async()=>{
  const nav=await browser.execute(()=>Array.from(document.querySelector('.nav-rail').children).slice(0,5).map(el=>el.getAttribute('aria-label')||el.className));
  if(nav.join('|')!=='Recorder|Meetings|rail-divider|Recordings|Settings')throw Error('Navigation order is incorrect');
  await $('button[aria-label="Settings"]').click();
  await $('nav[aria-label="Settings sections"]').$('button=Meetings').click();
  await expect($('select[aria-label="Meeting transcription provider"]')).toHaveValue('assemblyai');
  const old=(await command('state')).settings.keep_audio;
  await $('input[aria-label="Meeting audio retention"]').setValue('7');
  await $('h2.settings-section-title').click();
  await browser.waitUntil(async()=>await command('meeting_retention')===7,{timeout:5000});
  if((await command('state')).settings.keep_audio!==old)throw Error('Meeting retention changed dictation');
  await $('button[aria-label="Meetings"]').click();
  await $('button=Record a meeting').click();
  await expect($('.setup-grid')).toBeDisplayed();
  await $('button=Close recording setup').click();
  await expect($('.setup-grid')).not.toExist();
 });
 it('admits a background import immediately and preserves it across navigation',async()=>{
  const back=$('button=All meetings');if(await back.isExisting())await back.click();
  await expect($('.meeting-library')).toBeDisplayed();
  mkdirSync('.tmp',{recursive:true});const file=path.resolve('.tmp/background-meeting.wav');
  const bytes=16000*2*1800,header=Buffer.alloc(44);
  header.write('RIFF',0);header.writeUInt32LE(bytes+36,4);header.write('WAVEfmt ',8);header.writeUInt32LE(16,16);header.writeUInt16LE(1,20);header.writeUInt16LE(1,22);header.writeUInt32LE(16000,24);header.writeUInt32LE(32000,28);header.writeUInt16LE(2,32);header.writeUInt16LE(16,34);header.write('data',36);header.writeUInt32LE(bytes,40);
  writeFileSync(file,header);truncateSync(file,bytes+44);
  const preview=await command('meeting_preview',{path:file});
  const start=Date.now();const imported=await command('meeting_import',{token:preview.token,title:'Background import',callAt:'2026-10-01T12:00:00Z'});
  if(Date.now()-start>2000)throw Error('Import waited for media preparation');
  const initial=await command('meeting_get',{id:imported});
  if(initial.meeting.transcription_status!=='importing')throw Error('No durable importing state');
  await $('button[aria-label="Recorder"]').click();await $('button[aria-label="Meetings"]').click();
  await expect($('.meeting-library')).toHaveText(expect.stringContaining('Background import'));
  await browser.waitUntil(async()=>(await command('meeting_get',{id:imported})).meeting.transcription_status==='complete',{timeout:60000});
  const saved=(await command('meeting_get',{id:imported})).meeting;
  if(saved.call_at!=='2026-10-01T12:00:00Z'||saved.title!=='Background import')throw Error('Import metadata was lost');
  unlinkSync(file);
  await browser.waitUntil(async()=>await browser.execute(()=>Array.from(document.querySelectorAll('.meeting-card')).find(card=>card.querySelector('.meeting-title')?.textContent==='Background import')?.querySelector('.badge')?.textContent==='Ready'),{timeout:5000});
  await browser.saveScreenshot('.tmp/meetings-cards.png');
 });
 it('keeps failed imports visible with a useful error',async()=>{
  const file=path.resolve('.tmp/missing-meeting.wav');
  const bytes=Buffer.alloc(44+32000);bytes.write('RIFF',0);bytes.writeUInt32LE(bytes.length-8,4);bytes.write('WAVEfmt ',8);bytes.writeUInt32LE(16,16);bytes.writeUInt16LE(1,20);bytes.writeUInt16LE(1,22);bytes.writeUInt32LE(16000,24);bytes.writeUInt32LE(32000,28);bytes.writeUInt16LE(2,32);bytes.writeUInt16LE(16,34);bytes.write('data',36);bytes.writeUInt32LE(32000,40);writeFileSync(file,bytes);
  const preview=await command('meeting_preview',{path:file});unlinkSync(file);
  const imported=await command('meeting_import',{token:preview.token,title:'Interrupted source'});
  await browser.waitUntil(async()=>(await command('meeting_get',{id:imported})).meeting.transcription_status==='failed',{timeout:15000});
  const failed=await command('meeting_get',{id:imported});
  if(!failed.error||failed.meeting.audio_available)throw Error('Import failure was hidden');
  await expect($('.meeting-library')).toHaveText(expect.stringContaining('Interrupted source'));
 });

});
