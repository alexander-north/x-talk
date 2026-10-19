import {browser,$,$$,expect} from '@wdio/globals';
import {mkdirSync} from 'node:fs';
async function command(name,args={}){return browser.execute((name,args)=>window.__TAURI_INTERNALS__.invoke(name,args),name,args);}
let entry;
async function reveal(label){
 await browser.waitUntil(()=>browser.execute(()=>document.querySelector('.home-outputs').getAnimations({subtree:true}).every(a=>a.playState!=='running')),{timeout:3000});
 const point=await browser.execute(label=>{const b=document.querySelector(`button[aria-label="${label}"]`);const r=b.getBoundingClientRect();return {x:Math.round(r.left+70),y:Math.round(r.top+18)};},label);
 await browser.action('pointer').move({...point,origin:'viewport'}).down().up().perform();
 await browser.waitUntil(()=>browser.execute(()=>document.querySelector('.home-outputs').getAnimations({subtree:true}).every(a=>a.playState!=='running')),{timeout:3000});
}

describe('Recording workspace',()=>{
 before(async()=>{
  await browser.execute(()=>localStorage.removeItem('xtalk.recordings.columns.v1'));await command('window_mode',{mode:'full'});
  const state=await command('state');if(!state.test_providers)throw new Error('UI checks require isolated test providers');
  await command('save_settings',{settings:{...state.settings,theme:'day',transform:true,copy_raw:false,copy_transformed:false,sounds:{started:false,stopped:false,canceled:false,transcribed:false,transformed:false}}});
 });
 it('opens on the recorder with one navigation rail and displays both saved outputs',async()=>{
  await expect($('nav[aria-label="Main navigation"]')).toExist();await expect($('header')).not.toExist();await expect($('footer')).not.toExist();
  await expect($('button[aria-label="Start recording"]')).toBeDisplayed();await expect($('button[aria-label="Cancel recording"]')).not.toExist();
  await $('button[aria-label="Start recording"]').click();await expect($('button[aria-label="Stop recording"]')).toBeDisplayed();
  await expect($('button[aria-label="Cancel recording"]')).toBeEnabled();await browser.pause(5000);const recordTop=await browser.execute(()=>document.querySelector('.record-orb').getBoundingClientRect().top);await $('button[aria-label="Stop recording"]').click();const stoppedTop=await browser.execute(()=>document.querySelector('.record-orb').getBoundingClientRect().top);if(Math.abs(recordTop-stoppedTop)>1)throw Error('Processing shifted recording controls');await expect($('button[aria-label="Cancel recording"]')).not.toExist();
  await expect($('[aria-label="Transcript output"]')).toHaveText('Acceptance raw transcript.');await expect($('[aria-label="Transformed output"]')).not.toExist();await expect($('.record-instruction')).not.toExist();await expect($('.glass-orb')).toHaveAttribute('data-phase','transforming');await expect($('button[aria-label="Cancel processing"]')).toBeEnabled();await expect($('.queue')).not.toExist();
  await browser.waitUntil(async()=>{const entries=await command('history',{offset:0});entry=entries[0];return !!entry?.transformed;},{timeout:15000});
  await expect($('.transformed-card')).toHaveAttribute('class',expect.stringContaining('card-front'));await expect($('[aria-label="Transformed output"]')).toHaveText('Acceptance transformed output.');await expect($('.transcript-card .output-timing')).toHaveText(expect.stringContaining('seconds'));await expect($('.transformed-card .output-timing')).toHaveText(expect.stringContaining('seconds'));if(entry.transcription_ms===null||entry.transformation_ms<1100)throw Error('Stage timings were not saved');
  await browser.waitUntil(()=>browser.execute(()=>document.querySelector('.home-outputs').getAnimations({subtree:true}).every(a=>a.playState!=='running')),{timeout:3000});
  const offset=await browser.execute(()=>{const a=document.querySelector('.transcript-card').getBoundingClientRect();const b=document.querySelector('.transformed-card').getBoundingClientRect();return {x:b.x-a.x,y:b.y-a.y};});if(offset.x<=10||offset.y<30)throw new Error('Output cards do not overlap with the requested offset');
  await reveal('Show transcript');await expect($('.transcript-card')).toHaveAttribute('class',expect.stringContaining('card-front'));await expect($('[aria-label="Transcript output"]')).toHaveText('Acceptance raw transcript.');
  await $('button[aria-label="Copy transcript"]').click();await expect($('.toast')).toHaveText('Copied');await expect($('.transcript-card')).toHaveAttribute('class',expect.stringContaining('card-front'));
  await browser.execute(()=>document.querySelector('button[aria-label="Show transformed output"]').focus());await browser.keys('Enter');await expect($('.transformed-card')).toHaveAttribute('class',expect.stringContaining('card-front'));
  await reveal('Show transcript');
  mkdirSync('.tmp',{recursive:true});await browser.saveScreenshot('.tmp/recorder-sand.png');
 });
 it('cancels from the orbit button without admitting another recording',async()=>{
  const geometry=await browser.execute(()=>{const r=document.querySelector('.home-outputs').getBoundingClientRect();return {top:r.top,height:r.height};});const before=await command('history',{offset:0});await $('button[aria-label="Start recording"]').click();await expect($('button[aria-label="Stop recording"]')).toBeDisplayed();await expect($('button[aria-label="Cancel recording"]')).toBeEnabled();
  await browser.pause(650);const after=await browser.execute(()=>{const r=document.querySelector('.home-outputs').getBoundingClientRect();return {top:r.top,height:r.height};});if(Math.abs(after.top-geometry.top)>1||Math.abs(after.height-geometry.height)>1)throw Error('Starting capture changed reserved card geometry');
  const placement=await browser.execute(()=>{const record=document.querySelector('.record-orb').getBoundingClientRect();const cancel=document.querySelector('.cancel-orb').getBoundingClientRect();return {right:cancel.x>record.x+record.width/2,below:cancel.y+cancel.height/2>record.y+record.height/2,diameter:cancel.width};});if(!placement.right||!placement.below||placement.diameter<50||placement.diameter>57)throw new Error('Cancel is not the larger lower-right orbit');
  await $('button[aria-label="Cancel recording"]').click();await expect($('button[aria-label="Start recording"]')).toBeDisplayed();await expect($('button[aria-label="Cancel recording"]')).not.toExist();if((await command('history',{offset:0})).length!==before.length)throw new Error('Cancel admitted a history entry');
 });
 it('provides search, sorting, column controls, full-text viewing, and revision-safe transcript editing',async()=>{
  await $('button[aria-label="Recordings"]').click();await expect($('.recordings-table')).toBeDisplayed();const stickyActions=await browser.execute(()=>{const s=getComputedStyle(document.querySelector('th.actions-column'));return s.position==='sticky'&&s.top==='0px'&&s.backgroundColor!=='rgba(0, 0, 0, 0)';});if(!stickyActions)throw Error('Actions header is not sticky and opaque');
  await (await $$('.text-preview'))[0].click();await expect($('dialog')).toBeDisplayed();await expect($('[aria-label="Full saved text"]')).toHaveText('Acceptance raw transcript.');await $('button[aria-label="Close dialog"]').click();
  await $('button[title="Edit recording"]').click();await expect($('dialog')).toBeDisplayed();
  const inputs=await $$('dialog input');await inputs[0].setValue('Desert thoughts');
  await $('dialog textarea').setValue('Corrected transcript with a unique butterscotch phrase.\nSecond line.\nThird line.\nFourth line is preserved too.');await $('button=Save changes').click();await expect($('dialog')).not.toExist();
  const saved=await command('entry',{id:entry.id});if(saved.transcription_ms!==null)throw Error('Manual transcript edit kept the old timing');if(saved.transcript_revision!==entry.transcript_revision+1||saved.transformed!==entry.transformed||saved.transformed_revision===saved.transcript_revision)throw new Error('Transcript edit did not preserve stale output');entry=saved;
  await expect($('.badge.stale')).toHaveText('Out of date');await $('input[aria-label="Search recordings"]').setValue('butterscotch');await expect($('.recording-title')).toHaveText('Desert thoughts');
  await $('input[aria-label="Search recordings"]').setValue('not present in this recording');await expect($('.table-empty h2')).toHaveText('No recordings found');await $('button[aria-label="Clear search"]').click();await expect($('.recording-title')).toHaveText('Desert thoughts');
  await (await $$('button.sort-button'))[0].click();await expect($('th[aria-sort="ascending"]')).toExist();
  await $('summary').click();const source=await $('label=Source');await source.click();
  await expect($('button=Source')).toExist();await $('button[aria-label="Move Source column left"]').click();await $('summary').click();
  const resizer=$('button[aria-label="Resize Transcript column"]');await resizer.click();await browser.keys('ArrowRight');
  await $('button[aria-label="Recorder"]').click();await $('button[aria-label="Recordings"]').click();await expect($('button=Source')).toExist();
  await browser.saveScreenshot('.tmp/recordings-sand.png');
 });
 it('plays, pauses, seeks, and stops original audio without provider requests',async()=>{
  await $('button[aria-label="Play audio"]').click();await browser.waitUntil(async()=>{const p=await command('playback_state');return p?.position>0.1;},{timeout:5000});
  await $('button[aria-label="Pause audio"]').click();await browser.waitUntil(async()=>(await command('playback_state'))?.paused===true,{timeout:5000});
  const paused=await command('playback_state');await browser.pause(250);const still=await command('playback_state');if(Math.abs(still.position-paused.position)>0.05)throw new Error('Paused playback kept consuming audio');
  await browser.execute(()=>{const input=document.querySelector('input[type=range]');input.value='2';input.dispatchEvent(new Event('change',{bubbles:true}));});
  await browser.waitUntil(async()=>{const p=await command('playback_state');return p?.paused&&Math.abs(p.position-2)<0.05;},{timeout:5000});
  await $('button[aria-label="Play audio"]').click();await browser.waitUntil(async()=>{const p=await command('playback_state');return p&&!p.paused&&p.position>2.05;},{timeout:5000});await command('stop_playback');
 });
 it('switches themes and native compact/full/fullscreen modes while preserving outputs',async()=>{
  await $('button[aria-label="Theme: Light"]').click();await expect($('.app')).toHaveAttribute('data-theme','night');await $('button[aria-label="Recorder"]').click();await browser.saveScreenshot('.tmp/recorder-earth.png');
  await $('button[aria-label="Compact view"]').click();await expect($('.app')).toHaveAttribute('class',expect.stringContaining('compact'));await expect($('[aria-label="Transcript output"]')).not.toExist();await expect($('[aria-label="Transformed output"]')).not.toExist();await expect($('button[aria-label="Start recording"]')).toBeDisplayed();
  await expect($('button[aria-label="Cancel recording"]')).not.toExist();const small=await browser.getWindowSize();if((await browser.execute(()=>innerWidth))>450||(await browser.execute(()=>innerHeight))>430)throw new Error('Compact mode did not resize the native window');await browser.saveScreenshot('.tmp/recorder-compact.png');
  await $('button[aria-label="Expand workspace"]').click();await expect($('.app')).not.toHaveAttribute('class',expect.stringContaining('compact'));const full=await browser.getWindowSize();if(full.width<=small.width)throw new Error('Expand did not restore a larger workspace');await reveal('Show transcript');await expect($('[aria-label="Transcript output"]')).toHaveText(expect.stringContaining('butterscotch'));
  await $('button[aria-label="Full screen"]').click();await expect($('button[aria-label="Exit full screen"]')).toExist();await browser.keys('Escape');await expect($('button[aria-label="Full screen"]')).toExist();
  await $('button[aria-label="Theme: Dark"]').click();await expect($('button[aria-label="Theme: Auto (follow system)"]')).toExist();await $('button[aria-label="Theme: Auto (follow system)"]').click();await expect($('.app')).toHaveAttribute('data-theme','day');
  await $('button[aria-label="Settings"]').click();await $('button=Delivery and Sounds').click();if((await $$('button[title="Preview sound"]')).length!==5)throw new Error('Missing distinct cue previews');
 });
});

describe('Settings workflow and responsive library',()=>{
 it('starts with General, autosaves simple choices, and omits removed controls',async()=>{
  await $('button[aria-label="Recorder"]').click();
  await $('button[aria-label="Settings"]').click();
  const labels=await $$('.settings-tabs button').map(b=>b.getText());
  if(labels.join('|')!=='General|Recording|API Keys|Providers|Transformation|Delivery and Sounds')throw Error('Settings tab order changed');
  await expect($('section[aria-label="General"]')).toExist();await expect($('button=Save settings')).not.toExist();await expect($('button=Reload saved settings')).not.toExist();await expect($('button=Quit x-talk…')).not.toExist();
  await $('section[aria-label="General"] select').selectByAttribute('value','system');await browser.waitUntil(async()=>(await command('state')).settings.theme==='system');
  await $('button=Recording').click();const checkbox=(await $$('section input[type=checkbox]'))[0];const was=await checkbox.isSelected();await checkbox.click();await browser.waitUntil(async()=>(await command('state')).settings.compression===!was);
 });
 it('edits one transformation, saves choices immediately, and guards unsaved prompts',async()=>{
  await $('button=Transformation').click();await expect($('.transformation-details')).toBeDisplayed();await expect($('.preset-sidebar')).not.toExist();await expect($('button[aria-label="Clone preset"]')).not.toExist();await expect($('button[aria-label="Delete preset"]')).not.toExist();await expect($('button=Use for new recordings')).not.toExist();
  const initial=await command('state');if('presets' in initial||!initial.transformation)throw Error('Expected a single configuration in native state');const activeId=initial.settings.preset_id;
  const modelSelect=(await $$('.transformation-details .two-col select'))[1];await modelSelect.selectByAttribute('value','gpt-5-nano');await browser.waitUntil(async()=>(await command('state')).transformation.model==='gpt-5-nano');
  await $('label=Thinking').$('select').selectByAttribute('value','high');await browser.waitUntil(async()=>(await command('state')).transformation.reasoning==='high');
  const prompt=(await $$('textarea.prompt'))[0];const editedPrompt='Clean this transcript. Preserve proper names.';await prompt.setValue(editedPrompt);
  await $('button=General').click();await expect($('dialog')).toBeDisplayed();await $('button=Keep editing').click();await expect($('dialog')).not.toExist();await expect(prompt).toHaveValue(editedPrompt);
  await $('button[aria-label="Recorder"]').click();await expect($('dialog')).toBeDisplayed();await $('button=Save and continue').click();await expect($('dialog')).not.toExist();await expect($('button[aria-label="Start recording"]')).toBeDisplayed();
  const saved=await command('state');if(saved.transformation.system!==editedPrompt||saved.settings.preset_id!==activeId)throw Error('Transformation save lost its content or changed identity');
  await $('button[aria-label="Settings"]').click();await $('button=Transformation').click();const user=(await $$('textarea.prompt'))[1];const template=await user.getValue();await user.setValue('Unsaved '+template);await $('button=Providers').click();await $('button=Discard changes').click();await expect($('section[aria-label="Providers"]')).toExist();
  await $('button=Transformation').click();expect(await (await $$('textarea.prompt'))[1].getValue()).toBe(template);await expect($('button=Save prompts')).toBeDisabled();
  const blocked=await browser.execute(async settings=>{try{await window.__TAURI_INTERNALS__.invoke('save_settings',{settings:{...settings,preset_id:crypto.randomUUID()}});return false;}catch{return true;}},(await command('state')).settings);if(!blocked)throw Error('Settings exposed active-preset switching');
  await browser.saveScreenshot('.tmp/settings-transformation.png');
 });
 it('shows saved key status, supports deliberate add/replace/cancel/remove, and gates providers',async()=>{
  await $('button=API Keys').click();await expect($('input[type=password]')).not.toExist();if((await $$('.credential-row')).length!==5)throw Error('Missing provider key rows');
  await expect($('section[aria-label="OpenAI API key"] .credential-status')).toHaveText('Key saved');await $('button[aria-label="Replace OpenAI key"]').click();await expect($('input[type=password]')).toHaveValue('');await $('input[type=password]').setValue('unused replacement');await $('button=Cancel').click();await expect($('input[type=password]')).not.toExist();if(!(await command('state')).credentials.openai)throw Error('Cancel removed a saved key');
  await $('button[aria-label="Remove Anthropic key"]').click();await expect($('.key-removal-modal')).toBeDisplayed();await $('button=Cancel').click();if(!(await command('state')).credentials.anthropic)throw Error('Cancel removed the saved key');
  await $('button[aria-label="Remove Anthropic key"]').click();await $('button=Remove key').click();await expect($('.key-removal-modal')).not.toExist();await browser.waitUntil(async()=>!(await command('state')).credentials.anthropic);
  await browser.waitUntil(async()=>await browser.execute(()=>document.querySelector('.save-state').textContent.trim()===''),{timeout:4000,timeoutMsg:'Settings status did not clear'});await expect($('section[aria-label="Anthropic API key"] .credential-status')).toHaveText('No key');await expect($('input[type=password]')).not.toExist();
  await $('button=Transformation').click();await expect($('.transformation-details select option[value="anthropic"]')).not.toExist();
  await $('button=API Keys').click();await $('button[aria-label="Add Anthropic key"]').click();await $('input[type=password]').setValue('isolated-placeholder');await $('button=Save key').click();await browser.waitUntil(async()=>(await command('state')).credentials.anthropic);await expect($('input[type=password]')).not.toExist();await expect($('section[aria-label="Anthropic API key"] .credential-status')).toHaveText('Key saved');
  await $('button[aria-label="Replace Anthropic key"]').click();await expect($('input[type=password]')).toHaveValue('');await $('input[type=password]').setValue('second-isolated-placeholder');await $('button=Save key').click();await expect($('input[type=password]')).not.toExist();await browser.saveScreenshot('.tmp/settings-api-keys.png');
  await $('button=Transformation').click();await expect($('.transformation-details select option[value="anthropic"]')).toExist();
 });
 it('reports transformation test time without creating recordings or jobs',async()=>{
  const before=await command('recordings',{query:{search:'',sort:'created',descending:true,offset:0}});const jobs=(await command('state')).jobs.length;
  await $('.transformation-test summary').click();await $('.transformation-test textarea').setValue('A short test transcript.');await $('button=Run test').click();
  await expect($('[aria-label="Transformation test output"]')).toHaveText('Acceptance transformed output.');await expect($('.transformation-test .output-timing')).toHaveText(expect.stringContaining('seconds'));
  const label=await $('.transformation-test .output-timing').getText();if(parseFloat(label.replace(/[^0-9.]/g,''))<1.1)throw Error('Test elapsed time did not include provider wait');
  if((await command('recordings',{query:{search:'',sort:'created',descending:true,offset:0}})).total!==before.total||(await command('state')).jobs.length!==jobs)throw Error('Preset test created recording work');
 });
 it('fits all available columns without horizontal scrolling and searches saved text',async()=>{
  await $('button[aria-label="Recordings"]').click();await $('summary').click();
  const checkboxes=await $$('.column-menu input[type=checkbox]');for(const box of checkboxes)if(!await box.isSelected())await box.click();await $('summary').click();
  await expect($('input[aria-label="Search recordings"]')).toExist();await expect($('button=Import audio')).not.toExist();await expect($('select[aria-label="Recordings per page"]')).toHaveValue('50');await $('select[aria-label="Recordings per page"]').selectByAttribute('value','25');await expect($('select[aria-label="Recordings per page"]')).toHaveValue('25');
  for(const width of [1200,800,460]){await browser.setWindowSize(width,800);await browser.pause(150);const overflow=await browser.execute(()=>[document.documentElement,document.querySelector('.main-shell'),document.querySelector('.table-scroll')].some(e=>e.scrollWidth>e.clientWidth+1));if(overflow)throw Error(`Horizontal overflow at ${width}px`);}
  await browser.setWindowSize(1100,800);await $('input[aria-label="Search recordings"]').click();const focus=await browser.execute(()=>getComputedStyle(document.querySelector('.search-field')).outlineStyle);if(focus==='none')throw Error('Search container is missing focus outline');
  await $('button[aria-label="Recorder"]').click();await expect($('.record-instruction')).not.toExist();await expect($('.focus-caption')).not.toExist();
  const footer=await browser.execute(()=>{const b=document.querySelector('.import-footer').getBoundingClientRect();return {bottom:innerHeight-b.bottom,left:b.left};});if(footer.bottom>25||footer.left>130)throw Error('Import is not in the recorder bottom-left');await $('button[aria-label="Recordings"]').click();await $('input[aria-label="Search recordings"]').setValue('no matching recording');await expect($('.table-empty h2')).toHaveText('No recordings found');
  await $('button=Clear All').click();await expect($('dialog')).toBeDisplayed();await $('button=Cancel').click();if((await command('recordings',{query:{search:'',sort:'created',descending:true,offset:0}})).total!==1)throw Error('Cancel cleared the library');
  await $('button=Clear All').click();await $('button=Clear all recordings').click();await expect($('dialog')).not.toExist();if((await command('recordings',{query:{search:'',sort:'created',descending:true,offset:0}})).total!==0)throw Error('Clear All was limited to the current search');
 });
});
