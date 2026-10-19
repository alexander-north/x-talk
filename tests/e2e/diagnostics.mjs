import { browser, $, expect } from '@wdio/globals';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
async function command(name,args={}){return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);}
function logs(folder){try{return readdirSync(folder).filter(n=>n.endsWith('.log')).map(n=>readFileSync(path.join(folder,n),'utf8')).join('');}catch{return '';}}
describe('Local diagnostics settings',()=>{
 it('defaults on, persists UI changes, and stops/resumes writing without a restart',async()=>{
  const initial=await command('state');
  if(!initial.test_providers)throw new Error('Diagnostics tests require isolated test providers');
  if(!initial.settings.diagnostic_logging||!initial.diagnostics.enabled)throw new Error('Logging did not default on');
  await $('button[aria-label="Settings"]').click();await $('button=General').click();
  await expect($('#diagnostic-logging')).toBeSelected();
  await expect($('button=Open logs folder')).toExist();
  await $('#diagnostic-logging').click();
  await browser.waitUntil(async()=>!(await command('state')).diagnostics.enabled,{timeout:5000});
  await command('capture_action',{action:'start'});await browser.pause(500);await command('capture_action',{action:'cancel'});
  if(logs(initial.diagnostics.directory).includes('capture_action_requested'))throw new Error('Disabled logging recorded capture actions');
  await $('#diagnostic-logging').click();
  await browser.waitUntil(async()=>(await command('state')).diagnostics.enabled,{timeout:5000});
  await command('capture_action',{action:'start'});await browser.pause(500);await command('capture_action',{action:'cancel'});
  await browser.waitUntil(()=>logs(initial.diagnostics.directory).includes('capture_action_completed'),{timeout:5000});
  const text=logs(initial.diagnostics.directory);
  if(!text.includes('application_start')||!text.includes('diagnostic_logging_changed'))throw new Error('Missing useful lifecycle events');
  if((await command('state')).diagnostics.error)throw new Error('Logging reported a write failure');
 });
});
