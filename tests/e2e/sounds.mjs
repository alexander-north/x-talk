import {browser} from '@wdio/globals';
import {readFileSync,readdirSync} from 'node:fs';
import path from 'node:path';
async function command(name,args={}) {return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);}
let diagnostics;
function lines(){return readdirSync(diagnostics.directory).filter(n=>n.endsWith('.log')&&n.includes(diagnostics.session.slice(0,8))).sort().flatMap(n=>readFileSync(path.join(diagnostics.directory,n),'utf8').split('\n'));}
function played(){return lines().filter(l=>l.includes('cue_started')).map(l=>({event:l.match(/event="?([a-z]+)/)?.[1],time:Date.parse(l.split(' ')[0])}));}
async function configure(enabled){const s=await command('state');await command('save_settings',{settings:{...s.settings,transform:true,copy_raw:false,copy_transformed:false,sounds:Object.fromEntries(['started','stopped','canceled','transcribed','transformed'].map(k=>[k,enabled]))}});}
async function finished(){await browser.waitUntil(async()=>!(await command('state')).jobs.some(j=>['queued','running'].includes(j.status)),{timeout:10000});}
describe('Native event sound integration',()=>{
 before(async()=>{const s=await command('state');if(!s.test_providers)throw Error('Requires isolated non-network test providers');diagnostics=s.diagnostics;});
 it('plays real start/stop/completion cues in order without cutting off Stop',async()=>{
  await configure(true);
  await command('capture_action',{action:'start'});await browser.pause(500);
  await command('capture_action',{action:'stop'});await finished();
  await browser.waitUntil(()=>played().some(e=>e.event==='transformed'),{timeout:5000});
  const events=played();
  if(events.map(e=>e.event).join('|')!=='started|stopped|transcribed|transformed')throw Error('Unexpected cue sequence: '+JSON.stringify(events));
  if(events[2].time-events[1].time<330||events[3].time-events[2].time<400)throw Error('Completion interrupted an unfinished cue');
  if(lines().some(l=>l.includes('cue_playback_failed')))throw Error('Windows rejected audio playback');
 });
 it('plays explicit previews with toggles off but rejects them during capture',async()=>{
  await browser.pause(500);await configure(false);const before=played().length;
  for(const event of ['started','stopped','canceled','transcribed','transformed']){await command('preview_sound',{event});await browser.pause(550);}
  await browser.waitUntil(()=>played().length===before+5,{timeout:5000});
  await command('capture_action',{action:'start'});
  let rejected=false;try{await command('preview_sound',{event:'transformed'});}catch{rejected=true;}
  if(!rejected)throw Error('Preview was allowed during microphone capture');
  await browser.pause(200);await command('capture_action',{action:'cancel'});await browser.pause(500);
  if(played().length!==before+5)throw Error('Disabled capture cue or rejected preview played');
 });
 it('does not announce an old transformation after a new recording and cancel',async()=>{
  await configure(true);const before=played().length;
  await command('capture_action',{action:'start'});await browser.pause(450);
  await command('capture_action',{action:'stop'});
  await command('capture_action',{action:'start'});await browser.pause(300);
  await command('capture_action',{action:'cancel'});await finished();await browser.pause(700);
  if(played().slice(before).some(e=>e.event==='transformed'))throw Error('Superseded completion sound played');
  if(!(await command('history',{offset:0})).some(e=>e.transformed))throw Error('Sound suppression lost processing results');
 });
});
