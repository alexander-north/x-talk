import { browser, expect, $ } from '@wdio/globals';
import { execFileSync } from 'node:child_process';
async function command(name, args={}) {
 return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);
}
function clipboard() {
 const encoded=execFileSync('powershell.exe',['-NoProfile','-Command',"[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((Get-Clipboard -Raw)))"],{encoding:'utf8'}).trim();
 return Buffer.from(encoded,'base64').toString('utf8').replace(/\r\n/g,'\n').trimEnd();
}
describe('Windows normal recording Stop',()=>{
 before(async()=>{
  const state=await command('state');
  // Refuse to record unless an isolated test build has selected its non-network backend.
  if(state.test_providers!==true)throw new Error('Stop tests require the isolated test-providers debug build');
  if(!state.recovery_warnings.some(w=>w.category==='incomplete_recovery'))throw new Error('Missing recoverable journal warning');
  await expect($('[role=alert]')).toHaveText(expect.stringContaining('manual recovery'));
 });
 for(const [transform,copyRaw,copyTransformed] of [[true,true,true],[false,true,false],[true,false,true],[true,true,false],[false,false,true]]){
  it(`saves exactly once and delivers enabled clipboard stages (transform=${transform}, raw=${copyRaw}, transformed=${copyTransformed})`,async()=>{
   const s=await command('state');
   await command('save_settings',{settings:{...s.settings,transform,copy_raw:copyRaw,copy_transformed:copyTransformed,sounds:{started:false,stopped:false,canceled:false,transcribed:false,transformed:false}}});
   const before=await command('history',{offset:0});
   const ids=new Set(before.map(e=>e.id));
   const sentinel='Acceptance clipboard sentinel';
   await command('copy_text',{id:null,kind:null,text:sentinel});
   await command('capture_action',{action:'start'});
   if(!(await command('state')).capture.active)throw new Error('Microphone did not start');
   await browser.pause(1000);
   await command('capture_action',{action:'stop'});
   const stopped=await command('state');
   if(stopped.capture.active||stopped.capture.error)throw new Error('Normal Stop did not finalize cleanly');
   await browser.waitUntil(async()=>{
    const s=await command('state');return s.jobs.some(j=>!ids.has(j.entry_id)&&j.status==='complete');
   },{timeout:15000,timeoutMsg:'Recording job did not complete'});
   // A repeated Stop must be harmless and may not admit another entry/job.
   await command('capture_action',{action:'stop'});
   const after=await command('history',{offset:0});
   if(after.length!==before.length+1)throw new Error('Stop did not create exactly one history entry');
   const [entry]=after.filter(e=>!ids.has(e.id));
   const state=await command('state');const jobs=state.jobs.filter(j=>j.entry_id===entry.id);
   if(jobs.length!==1||jobs[0].error)throw new Error('Stop did not admit exactly one successful processing job');
   if(!entry.audio||entry.duration<=0||entry.bytes<=44)throw new Error('Original audio was not preserved');
   if(entry.transcript!=='Acceptance raw transcript.')throw new Error('Wrong transcript');
   if(transform&&entry.transformed!=='Acceptance transformed output.')throw new Error('Missing selected transformation');
   const shouldCopy=(transform&&copyTransformed)||copyRaw;
   const expected=shouldCopy?(transform&&copyTransformed?entry.transformed:entry.transcript):sentinel;
   const actual=clipboard();
   if(actual!==expected){const category=actual===sentinel?'sentinel':actual===entry.transcript?'transcript':actual===entry.transformed?'transformed':actual===''?'empty':'external content';throw new Error(`Stage-output clipboard policy violated: observed ${category}; native delivery ${jobs[0].delivery}`);}
   if(jobs[0].delivery!==(shouldCopy?'copied':'manual_only'))throw new Error('Wrong delivery status');
   await expect($('.stage-error')).not.toExist();
   await command('delete_entry',{id:entry.id,revision:entry.revision,confirmed:true});
  });
 }
});
