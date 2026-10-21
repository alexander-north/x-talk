<script lang="ts">
 import {onMount,untrack,tick} from 'svelte';
 import {api,message,elapsedTime} from '../ipc';
 import type {AppState,Transformation,Settings} from '../types';
 import {providerNames,settingsSections,textDirty,thinkingLabel,thinkingValue,type SettingsChange} from '../settings';
 import {meetings} from '../meetings/ipc';
 let meetingRetention=$state<number|null>(50);
 onMount(()=>{void meetings.retention().then(value=>meetingRetention=value).catch(e=>error=message(e));});
 async function retainMeetings(value:number|null){await task(async()=>{await meetings.setRetention(value);meetingRetention=value;},'Meeting audio retention saved');}
 import Icon from './Icon.svelte';
 import Updates from './Updates.svelte';
 import ShortcutEditor from './ShortcutEditor.svelte';
 let {snapshot,write,refresh,guard}:{snapshot:AppState;write:(change:SettingsChange)=>Promise<Settings>;refresh:()=>Promise<void>;guard:(action:()=>void|Promise<void>)=>void}=$props();
 let section=$state('General');let saving=$state(0);let error=$state('');let saved=$state('');let removingKey=$state<string|null>(null);let removeDialog=$state<HTMLDialogElement>();
 let shortcuts=$state(untrack(()=>({...snapshot.settings.shortcuts})));let context=$state(untrack(()=>snapshot.settings.transcription.prompt));
 let base=$state<Transformation|null>(null);let draft=$state<Transformation|null>(null);
 let editingKey=$state<string|null>(null);let key=$state('');let keyInput=$state<HTMLInputElement>();let testInput=$state('');let testOutput=$state('');let testElapsed=$state<number|null>(null);let testing=$state(false);
 let configQueue:Promise<unknown>=Promise.resolve();
 const copy=<T,>(v:T):T=>JSON.parse(JSON.stringify(v));
 let settings=$derived(snapshot.settings);
 let model=$derived(snapshot.models.find(m=>m.provider===draft?.provider&&m.id===draft?.model&&m.stage==='transformation'));
 let transcriptionModel=$derived(snapshot.models.find(m=>m.provider===settings.transcription.provider&&m.id===settings.transcription.model&&m.stage==='transcription'));
 let transformDirty=$derived(!!draft&&!!base&&textDirty(draft,base));
 let shortcutDirty=$derived(JSON.stringify(shortcuts)!==JSON.stringify(settings.shortcuts));
 let contextDirty=$derived(context!==settings.transcription.prompt);
 let configuredText=$derived(['openai','anthropic'].filter(p=>snapshot.credentials[p]));
 let configuredSpeech=$derived(['openai','assemblyai','elevenlabs','deepgram'].filter(p=>snapshot.credentials[p]));
 $effect(()=>{if(!saved)return;const timer=setTimeout(()=>saved='',2000);return()=>clearTimeout(timer);});
 $effect(()=>{if(removingKey&&removeDialog&&!removeDialog.open)removeDialog.showModal();});
 $effect(()=>{if(!draft){base=copy(snapshot.transformation);draft=copy(snapshot.transformation);}});
 async function task(work:()=>Promise<unknown>,label='Saved'){saving++;error='';saved='';try{await work();saved=label;return true;}catch(e){error=message(e);return false;}finally{saving--;}}
 async function change(update:SettingsChange){await task(()=>write(update));}
 export function isDirty(){return transformDirty||shortcutDirty||contextDirty;}
 export function discardAll(){if(base)draft=copy(base);shortcuts=copy(settings.shortcuts);context=settings.transcription.prompt;}
 export async function saveAll(){
  await configQueue;
  if(shortcutDirty&&!await saveShortcuts())return false;
  if(contextDirty&&!await saveContext())return false;
  if(transformDirty&&!await saveTransformation())return false;
  return true;
 }
 function openSection(next:string){guard(()=>{section=next;key='';editingKey=null;});}
 async function saveShortcuts(){const value=copy(shortcuts);return task(async()=>{const result=await write(s=>({...s,shortcuts:value}));shortcuts=copy(result.shortcuts);});}
 async function saveContext(){const value=context;return task(async()=>{const result=await write(s=>({...s,transcription:{...s.transcription,prompt:value}}));context=result.transcription.prompt;});}
 async function saveTransformation(){
  await configQueue;if(!draft)return false;const value=copy(draft);
  return task(async()=>{const result=await api.transformation(value);base=copy(result);draft=copy(result);await refresh();},'Prompts saved');
 }
 function configureTransformation(update:Partial<Transformation>){
  const operation=configQueue.then(async()=>{if(!base||!draft)return;await task(async()=>{const result=await api.transformation({...copy(base!),...update});base=copy(result);draft={...draft!,...update,revision:result.revision};await refresh();});});
  configQueue=operation.catch(()=>undefined);
 }
 async function editKey(provider:string){editingKey=provider;key='';error='';await tick();keyInput?.focus();}
 async function saveKey(provider:string){const value=key;key='';if(await task(async()=>{await api.credential(provider,'set',value);await refresh();},'Key saved securely'))editingKey=null;}
 async function removeKey(provider:string){key='';if(await task(async()=>{await api.credential(provider,'delete');await refresh();},'Key removed')){editingKey=null;removingKey=null;}}
 function providerChanged(provider:string){const first=snapshot.models.find(m=>m.provider===provider&&m.stage==='transformation');if(first)configureTransformation({provider,model:first.id,reasoning:first.default_reasoning,temperature:null});}
 function modelChanged(id:string){const next=snapshot.models.find(m=>m.provider===draft?.provider&&m.id===id);if(next)configureTransformation({model:id,reasoning:next.default_reasoning,temperature:null});}
 function speechProvider(provider:string){guard(async()=>{const first=snapshot.models.find(m=>m.provider===provider&&m.stage==='transcription');if(first){await change(s=>({...s,transcription_mode:provider==='elevenlabs'?s.transcription_mode:'batch',transcription:{provider,model:first.id,prompt:'',temperature:null}}));context=settings.transcription.prompt;}});}
 function speechModel(id:string){guard(async()=>{await change(s=>({...s,transcription:{...s.transcription,model:id,prompt:'',temperature:null}}));context=settings.transcription.prompt;});}
 async function test(){if(!draft)return;testing=true;testOutput='';testElapsed=null;const started=performance.now();await task(async()=>{testOutput=await api.test(copy(draft!),testInput);testElapsed=performance.now()-started;},'Test complete');testing=false;}
</script>
<main class="settings">
 <div class="page-heading"><div><p class="eyebrow">MAKE IT YOURS</p><h1>Settings</h1></div><span class="save-state" role="status">{saving?'Saving…':saved}</span></div>
 <nav class="settings-tabs" aria-label="Settings sections">{#each settingsSections as name}<button class:chosen={section===name} onclick={()=>openSection(name)}>{name}</button>{/each}</nav>
 <section class="settings-content" aria-label={section}><h2 class="settings-section-title">{section}</h2>
 {#if error}<p class="error-text" role="alert">{error}</p>{/if}
 {#if section==='General'}
  <label>Appearance<select value={settings.theme} onchange={e=>{const value=e.currentTarget.value;void change(s=>({...s,theme:value}));}}><option value="system">Auto · follow system</option><option value="day">Light</option><option value="night">Dark</option></select></label>
  {#each [['topmost','Always on top'],['autostart','Start at Windows login in the tray'],['close_to_tray','Keep running when the window is closed']] as [field,label]}<label class="check"><input type="checkbox" checked={settings[field as 'topmost'|'autostart'|'close_to_tray']} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,[field]:value}));}}>{label}</label>{/each}
  <Updates {snapshot} {guard} change={change}/>
  <h2>Diagnostics</h2><p class="help">Version {snapshot.diagnostics.version} · Build {snapshot.diagnostics.build_id}</p><label class="check"><input id="diagnostic-logging" type="checkbox" checked={settings.diagnostic_logging} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,diagnostic_logging:value}));}}>Enable diagnostic logging</label><p class="help">Logs contain app activity and error codes, without API keys, audio, or text content. Nothing is uploaded. Changes apply immediately.</p><p class="help">Logs folder: <code>{snapshot.diagnostics.directory}</code></p><button onclick={()=>task(api.openLogs,'Logs folder opened')}>Open logs folder</button>{#if snapshot.diagnostics.error}<p class="error-text" role="alert">{snapshot.diagnostics.error}</p>{/if}{#if snapshot.diagnostics.dropped_events}<p class="help">{snapshot.diagnostics.dropped_events} diagnostic events could not be written.</p>{/if}
  <h2>Audio retention</h2><label class="check"><input type="checkbox" checked={settings.keep_audio===null} onchange={e=>{const value=e.currentTarget.checked?null:100;void change(s=>({...s,keep_audio:value}));}}>Keep all original audio</label>{#if settings.keep_audio!==null}<label>Keep audio for the latest N entries<input type="number" value={settings.keep_audio} min="1" max="100000" step="1" required onchange={e=>{const value=e.currentTarget.valueAsNumber;void change(s=>({...s,keep_audio:value}));}}></label>{/if}<p class="help">Older text remains in your library. Pending work keeps its audio until it completes or is discarded.</p>
 {:else if section==='Meetings'}
  <label>Transcription provider<select aria-label="Meeting transcription provider"><option value="assemblyai">AssemblyAI</option></select></label><p class="help">Speaker-labelled meeting transcripts use AssemblyAI. Meeting briefs use OpenAI Luna. Manage their keys in API Keys.</p>
  <h2>Audio retention</h2><label class="check"><input type="checkbox" checked={meetingRetention===null} disabled={saving>0} onchange={e=>void retainMeetings(e.currentTarget.checked?null:50)}/>Keep all meeting audio</label>
  {#if meetingRetention!==null}<label>Keep audio for the latest N meetings<input aria-label="Meeting audio retention" type="number" min="1" max="100000" step="1" value={meetingRetention} disabled={saving>0} onchange={e=>void retainMeetings(e.currentTarget.valueAsNumber)}/></label>{/if}
  <p class="help">Transcripts, speaker edits and briefs stay indefinitely. Pending work keeps its audio until it completes. This setting is separate from dictation audio retention.</p>
 {:else if section==='Recording'}
  <label>Microphone<select value={settings.microphone??''} onchange={e=>{const value=e.currentTarget.value||null;void change(s=>({...s,microphone:value}));}}><option value="">System default at recording start</option>{#each snapshot.devices as device}<option value={device.id}>{device.name}</option>{/each}</select></label>
  <label class="check"><input type="checkbox" checked={settings.compression} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,compression:value}));}}>Compress upload audio</label><p class="help">Creates a smaller upload copy. The complete original is kept for playback and export.</p>
  <label class="check"><input type="checkbox" checked={settings.silence_removal} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,silence_removal:value}));}}>Shorten long near-silent pauses in uploads</label>
  <ShortcutEditor bind:value={shortcuts} saved={settings.shortcuts} saving={saving>0} save={saveShortcuts}/>

 {:else if section==='API Keys'}
  <p class="help">Keys are stored in Windows Credential Manager. Adding a key enables that provider in the model pickers. OpenAI uses one key for speech and transformations.</p>
  <div class="credential-list">{#each Object.entries(providerNames) as [provider,name]}
   <section class="credential-row" aria-label={`${name} API key`}>
    <div class="credential-summary"><div><h3>{name}</h3><span class="credential-status"><span class="status-dot" class:configured={snapshot.credentials[provider]}></span>{snapshot.credentials[provider]?'Key saved':'No key'}</span></div><div class="actions">
     {#if snapshot.credentials[provider]}<button class="subtle-button" aria-label={`Replace ${name} key`} disabled={saving>0} onclick={()=>editKey(provider)}>Replace</button><button class="subtle-button danger" aria-label={`Remove ${name} key`} disabled={saving>0} onclick={()=>{error='';removingKey=provider;}}>Remove</button>
     {:else}<button class="subtle-button" aria-label={`Add ${name} key`} disabled={saving>0} onclick={()=>editKey(provider)}>Add key</button>{/if}
    </div></div>
    {#if editingKey===provider}<form class="credential-editor" onsubmit={e=>{e.preventDefault();void saveKey(provider);}}><label>{snapshot.credentials[provider]?`Replace ${name} API key`:`${name} API key`}<input bind:this={keyInput} type="password" bind:value={key} autocomplete="off" spellcheck="false" placeholder={snapshot.credentials[provider]?'Enter replacement key':'Enter API key'}></label><div class="actions"><button class="primary" disabled={!key.trim()||saving>0}>Save key</button><button type="button" disabled={saving>0} onclick={()=>{editingKey=null;key='';}}>Cancel</button></div></form>{/if}
   </section>
  {/each}</div>
 {:else if section==='Providers'}
  <p class="help">Speech-to-text for new recordings and imports. Enable providers in API Keys.</p>
  <div class="two-col"><label>Provider<select value={snapshot.credentials[settings.transcription.provider]?settings.transcription.provider:''} onchange={e=>speechProvider(e.currentTarget.value)}><option value="" disabled>Choose a provider with a saved key</option>{#each configuredSpeech as p}<option value={p}>{providerNames[p]}</option>{/each}</select></label><label>Model<select disabled={!snapshot.credentials[settings.transcription.provider]} value={settings.transcription.model} onchange={e=>speechModel(e.currentTarget.value)}>{#each snapshot.models.filter(m=>m.provider===settings.transcription.provider&&m.stage==='transcription') as m}<option value={m.id}>{m.id}</option>{/each}</select></label></div>
  <label>Recording transcription<select aria-label="Recording transcription" value={settings.transcription_mode??'batch'} disabled={saving>0} onchange={e=>{const value=e.currentTarget.value as 'batch'|'realtime';guard(()=>change(s=>({...s,transcription_mode:value})));}}><option value="batch">After recording</option><option value="realtime" disabled={settings.transcription.provider!=='elevenlabs'||!snapshot.credentials.elevenlabs}>Real-time · ElevenLabs</option></select></label>
  {#if settings.transcription_mode==='realtime'}<p class="help">Microphone recordings use Scribe v2 Realtime with a live preview. Final text is delivered after Stop. Audio is sent while you speak; Cancel cannot recall audio already sent. Connection failures keep the original for manual retry. Imports and retries use the batch model above. Compression, silence removal and temperature apply only to batch processing. Up to 50 vocabulary phrases.</p>{:else}<p class="help">Audio is sent after Stop. Select ElevenLabs to enable real-time recording.</p>{/if}
  {#if !configuredSpeech.length}<button class="subtle-button" onclick={()=>openSection('API Keys')}>Add a speech provider key</button>{/if}
  {#if transcriptionModel?.prompt}<label>{transcriptionModel.context_field==='prompt'?'Transcription context':'Vocabulary hints (one phrase per line)'}<textarea bind:value={context} rows="4" maxlength="4000" placeholder="Names or vocabulary relevant to your speech"></textarea></label><button disabled={!contextDirty||saving>0} onclick={saveContext}>Save context</button>{/if}
  {#if settings.transcription.provider==='elevenlabs'&&context.trim()}<p class="help">ElevenLabs applies a 20% surcharge when vocabulary hints are used.</p>{/if}
  {#if transcriptionModel?.temperature}<label>Temperature (blank uses provider default)<input type="number" value={settings.transcription.temperature??''} min={transcriptionModel.temperature[0]} max={transcriptionModel.temperature[1]} step="0.1" onchange={e=>{const value=e.currentTarget.value===''?null:e.currentTarget.valueAsNumber;void change(s=>({...s,transcription:{...s.transcription,temperature:value}}));}}></label>{/if}
 {:else if section==='Transformation'}
  <label class="check"><input type="checkbox" checked={settings.transform} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,transform:value}));}}>Transform new recordings and imports</label>
  {#if draft&&base}<div class="transformation-details">
   <div class="two-col"><label>Provider<select value={snapshot.credentials[draft.provider]?draft.provider:''} onchange={e=>providerChanged(e.currentTarget.value)} disabled={saving>0}><option value="" disabled>Choose a provider with a saved key</option>{#each configuredText as p}<option value={p}>{providerNames[p]}</option>{/each}</select></label><label>Model<select value={draft.model} disabled={!snapshot.credentials[draft.provider]||saving>0} onchange={e=>modelChanged(e.currentTarget.value)}>{#each snapshot.models.filter(m=>m.provider===draft?.provider&&m.stage==='transformation') as m}<option value={m.id}>{m.label||m.id}</option>{/each}</select></label></div>
   {#if !configuredText.length}<button class="subtle-button" onclick={()=>openSection('API Keys')}>Add a transformation provider key</button>{/if}
   <p class="help model-description">{model?.description}</p>
   {#if model?.reasoning.length}<label>Thinking<select value={thinkingValue(draft,model)} disabled={saving>0} onchange={e=>configureTransformation({reasoning:e.currentTarget.value,temperature:null})}>{#each model.reasoning as level}<option value={level}>{thinkingLabel(level)}</option>{/each}</select></label><p class="help">More thinking can increase latency and cost. Off, minimal, or low is usually enough for transcript cleanup.</p>{/if}
   {#if model?.temperature&&['none','off'].includes(thinkingValue(draft,model))}<label>Temperature (blank uses provider default)<input type="number" value={draft.temperature??''} min={model.temperature[0]} max={model.temperature[1]} step="0.1" onchange={e=>configureTransformation({temperature:e.currentTarget.value===''?null:e.currentTarget.valueAsNumber})}></label>{/if}
   <label>System prompt<textarea class="prompt" bind:value={draft.system} rows="8"></textarea></label><label>User prompt template <small>Exactly one {'{{input}}'} placeholder</small><textarea class="prompt" bind:value={draft.user} rows="4"></textarea></label><div class="actions"><button class="primary" disabled={!transformDirty||saving>0} onclick={saveTransformation}>Save prompts</button><span class="help">{transformDirty?'Unsaved text changes':'Text saved · model choices apply immediately'}</span></div>
   <details class="transformation-test"><summary>Try this transformation</summary><label>Test input<textarea bind:value={testInput} rows="4" placeholder="Paste a transcript to test this prompt"></textarea></label><button disabled={testing||snapshot.operation_busy||snapshot.capture.active||!testInput.trim()||!snapshot.credentials[draft.provider]} onclick={test}>{testing?'Processing…':'Run test'}</button><div class="text-heading"><h3>Test output{#if testElapsed!==null}<span class="output-timing" title="Elapsed time from clicking Run test until the result returned">– {elapsedTime(testElapsed)}</span>{/if}</h3><button disabled={!testOutput} onclick={()=>task(()=>api.copyTest(testOutput),'Test output copied')}>Copy</button></div><div class="output" role="textbox" aria-readonly="true" aria-multiline="true" aria-label="Transformation test output" tabindex="0">{testOutput||'Test output stays in this panel.'}</div></details>
  </div>{/if}
 {:else if section==='Delivery and Sounds'}
  <h2>Automatic clipboard copying</h2>{#each [['copy_raw','Copy transcript as soon as it is ready'],['copy_transformed','Replace clipboard with transformed text when ready']] as [field,label]}<label class="check"><input type="checkbox" checked={settings[field as 'copy_raw'|'copy_transformed']} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,[field]:value}));}}>{label}</label>{/each}<p class="help">Applies to new microphone recordings in this session. Imports, retries and resumed jobs are saved for manual copying.</p>
  <h2>Event sounds</h2><p class="hint">Completion sounds wait their turn and stay silent while recording. With speakers, the start sound may be picked up by your microphone; turn off Recording started or use headphones to avoid this. No speech is muted or trimmed.</p>{#each [['started','Recording started'],['stopped','Recording stopped'],['canceled','Recording canceled'],['transcribed','Transcription completed'],['transformed','Transformation completed']] as [event,label]}<div class="sound-option"><label class="check"><input type="checkbox" checked={settings.sounds[event as keyof Settings['sounds']]} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,sounds:{...s.sounds,[event]:value}}));}}>{label}</label><button class="icon-button" aria-label={`Preview ${label.toLowerCase()} sound`} title="Preview sound" onclick={()=>task(()=>api.previewSound(event),'Sound previewed')}><Icon name="play" size={16}/></button></div>{/each}
 {/if}
 </section>
</main>

{#if removingKey}<dialog bind:this={removeDialog} class="modal key-removal-modal" aria-labelledby="key-removal-title" oncancel={e=>{if(saving)e.preventDefault();else removingKey=null;}} onclose={()=>removingKey=null}>
 <div class="dialog-heading"><h2 id="key-removal-title">Remove API key?</h2></div>
 <p>Are you sure you want to remove the saved {providerNames[removingKey]} API key?</p>
 <div class="actions dialog-actions"><button disabled={saving>0} onclick={()=>removingKey=null}>Cancel</button><button class="danger" disabled={saving>0} onclick={()=>removeKey(removingKey!)}>Remove key</button></div>
 {#if error}<p class="error-text" role="alert">{error}</p>{/if}
</dialog>{/if}
