import { describe, it, expect, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
import { invoke } from '@tauri-apps/api/core';
import { api, message, duration } from './ipc';
import type { Entry } from './types';
describe('validated command contract', () => {
 it('sends revision tokens for retries and metadata changes', async () => {
  const entry = { id: 'entry-id', revision: 7 } as Entry;
  await api.retry(entry, 'transform');
  expect(invoke).toHaveBeenLastCalledWith('retry', { id: 'entry-id', revision: 7, kind: 'transform' });
  await api.edit(entry, 'Title', 'Subtitle');
  expect(invoke).toHaveBeenLastCalledWith('edit_metadata', { id: 'entry-id', revision: 7, title: 'Title', subtitle: 'Subtitle' });
 });
 it('requires confirmed deletion and selects explicit saved output', async () => {
  await api.delete({ id: 'id', revision: 3 } as Entry);
  expect(invoke).toHaveBeenLastCalledWith('delete_entry', { id: 'id', revision: 3, confirmed: true });
  await api.copy('id', 'transcript');
  expect(invoke).toHaveBeenLastCalledWith('copy_text', { id: 'id', kind: 'transcript', text: null });
 });
 it('keeps test output separate from history', async () => {
  await api.copyTest('literal {{input}} <b>text</b>');
  expect(invoke).toHaveBeenLastCalledWith('copy_text', { id: null, kind: null, text: 'literal {{input}} <b>text</b>' });
 });
 it('presents duration boundaries and structured errors', () => {
  expect(duration(3600)).toBe('60:00'); expect(duration(61)).toBe('1:01');
  expect(message({ stage: 'transcribing', message: 'Add a key.' })).toBe('Add a key.');
 });
});

describe('recording workspace contracts',()=>{
 it('sends the full-library query and revision for transcript edits',async()=>{
  const query={search:'a phrase',sort:'created',descending:true,offset:30};
  await api.recordings(query);expect(invoke).toHaveBeenLastCalledWith('recordings',{query});
  await api.editRecording({id:'id',revision:8} as Entry,'Title','Subtitle','Corrected words');
  expect(invoke).toHaveBeenLastCalledWith('edit_recording',{id:'id',revision:8,title:'Title',subtitle:'Subtitle',transcript:'Corrected words'});
 });
 it('keeps seek position and pause state explicit and uses a fixed repository command',async()=>{
  await api.play('id',2.5,true);expect(invoke).toHaveBeenLastCalledWith('play_audio',{id:'id',startAt:2.5,paused:true});
  await api.pause('id',false);expect(invoke).toHaveBeenLastCalledWith('pause_playback',{id:'id',paused:false});
  await api.repository();expect(invoke).toHaveBeenLastCalledWith('open_repository');
 });
});

it('formats measured stage times and hides missing legacy measurements',async()=>{
 const {elapsedTime}=await import('./ipc');
 expect(elapsedTime(500)).toBe('0.5 seconds');expect(elapsedTime(1100)).toBe('1.1 seconds');expect(elapsedTime(0)).toBe('0.0 seconds');
 expect(elapsedTime(null)).toBe('');expect(elapsedTime(undefined)).toBe('');expect(elapsedTime(-1)).toBe('');expect(elapsedTime(NaN)).toBe('');
});

it('saves and tests one transformation without preset identity or collection operations',async()=>{
 const transformation={revision:4,provider:'openai',model:'gpt-6-luna',system:'Clean',user:'{{input}}',reasoning:'none',temperature:null};
 await api.transformation(transformation);expect(invoke).toHaveBeenLastCalledWith('save_transformation',{transformation});
 await api.test(transformation,'Raw text');expect(invoke).toHaveBeenLastCalledWith('test_transformation',{transformation,input:'Raw text'});
 expect(api).not.toHaveProperty('preset');expect(api).not.toHaveProperty('deletePreset');
});

it('requires explicit full-library confirmation without applying the current search or page',async()=>{
 await api.clearRecordings();expect(invoke).toHaveBeenLastCalledWith('clear_recordings',{confirmed:true});
});
