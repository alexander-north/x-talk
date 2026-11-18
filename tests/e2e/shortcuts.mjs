import {browser,$,expect} from '@wdio/globals';
const command=(name,args={})=>browser.execute((n,a)=>window.__TAURI_INTERNALS__.invoke(n,a),name,args);
const binding=async()=>(await command('state')).settings.shortcuts;
describe('Shortcut editor',()=>{
 let original;
 before(async()=>{
  const state=await command('state');
  if(!state.test_providers)throw Error('Shortcut checks require isolated test providers');
  original=state.settings.shortcuts;
  await $('button[aria-label="Settings"]').click();await $('button=Recording').click();
 });
 it('captures an existing registered combination without starting recording',async()=>{
  await $('button[aria-label="Edit toggle recording shortcut"]').click();
  const input=$('input[aria-label="Toggle recording shortcut"]');await expect(input).toBeEnabled();
  await browser.keys(['Control','Shift','K']);
  await expect(input).toHaveValue('Ctrl+Shift+K');
  await browser.keys(['Control','Alt','Shift','F7']);
  await expect(input).toHaveValue(expect.stringContaining('F7'));
  if((await command('state')).capture.active)throw Error('Capture widget triggered recording');
  await $('button=Cancel').click();
  if(JSON.stringify(await binding())!==JSON.stringify(original))throw Error('Cancel changed saved shortcuts');
 });
 it('preserves saved bindings on malformed and equivalent conflicting manual entry',async()=>{
  await $('button[aria-label="Edit toggle recording shortcut"]').click();await $('button=Enter manually').click();
  const input=$('input[aria-label="Toggle recording shortcut"]');
  for(const invalid of ['not-a-shortcut','Control+Alt+Shift+F9']){
   await input.setValue(invalid);await $('button=Save').click();
   await expect($('.toast-error')).toBeDisplayed();await expect(input).toBeDisplayed();
   if(JSON.stringify(await binding())!==JSON.stringify(original))throw Error('Rejected save changed shortcuts');
  }
  await $('button=Cancel').click();
 });
 it('removes, adds through capture, and saves a shortcut',async()=>{
  await $('button[aria-label="Edit toggle recording shortcut"]').click();await $('button=Remove shortcut').click();
  await expect($('button[aria-label="Add toggle recording shortcut"]')).toBeDisplayed();
  if((await binding()).toggle!=='')throw Error('Removal did not persist');
  await $('button[aria-label="Add toggle recording shortcut"]').click();
  await expect($('input[aria-label="Toggle recording shortcut"]')).toBeEnabled();
  await browser.keys(['Control','Alt','Shift','F7']);await $('button=Save').click();
  await expect($('button[aria-label="Edit toggle recording shortcut"]')).toBeDisplayed();
  if((await binding()).toggle!=='Ctrl+Alt+Shift+F7')throw Error('Captured shortcut did not persist');
  await $('button[aria-label="Recorder"]').click();
  await expect($('.shortcut-hints')).not.toHaveText(expect.stringContaining('F8'));
 });
});
