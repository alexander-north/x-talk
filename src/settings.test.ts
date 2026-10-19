import {describe,it,expect} from 'vitest';
import {settingsWriter,nextTheme,textDirty} from './settings';
import type {Settings,Transformation} from './types';
describe('settings persistence',()=>{
 it('serializes rapid changes using fresh revisions without losing independent choices',async()=>{
  let stored={revision:1,theme:'system',topmost:false} as Settings;
  const write=settingsWriter(async()=>({...stored}),async value=>{if(value.revision!==stored.revision)throw Error('stale');stored={...value,revision:value.revision+1};return stored;},async()=>{});
  await Promise.all([write(s=>({...s,theme:'night'})),write(s=>({...s,topmost:true}))]);
  expect(stored).toMatchObject({revision:3,theme:'night',topmost:true});
 });
 it('continues saving after a rejected change',async()=>{
  let stored={revision:1,theme:'system'} as Settings;
  const write=settingsWriter(async()=>stored,async value=>{if(value.theme==='bad')throw Error('invalid');return stored={...value,revision:value.revision+1};},async()=>{});
  await expect(write(s=>({...s,theme:'bad'}))).rejects.toThrow('invalid');await write(s=>({...s,theme:'day'}));expect(stored.theme).toBe('day');
 });
 it('cycles all three appearance choices and distinguishes unsaved text from saved model choices',()=>{
  expect(nextTheme(nextTheme(nextTheme('system')))).toBe('system');expect(nextTheme('system')).toBe('day');expect(nextTheme('day')).toBe('night');
  const base={system:'Clean',user:'{{input}}',model:'old'} as Transformation;
  expect(textDirty({...base,model:'new'},base)).toBe(false);expect(textDirty({...base,user:'Edit {{input}}'},base)).toBe(true);
 });
});
