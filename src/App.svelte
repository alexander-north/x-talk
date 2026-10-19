<script lang="ts">
 import {onMount} from 'svelte';
 import {listen} from '@tauri-apps/api/event';
 import {api,message} from './ipc';
 import {updateStatus} from './updates';
 import type {UpdateStatus} from './types';
 import type {AppState,Entry,Job,Playback} from './types';
 import Icon from './components/Icon.svelte';
 import Recorder from './components/Recorder.svelte';
 import RecordingsTable from './components/RecordingsTable.svelte';
 import CopyButton from './components/CopyButton.svelte';
 import SettingsView from './components/Settings.svelte';
 import {settingsWriter,nextTheme} from './settings';
 import {refreshQueue} from './refresh';
 import heart from './assets/heart.png';
 let snapshot=$state<AppState|null>(null);
 let latest=$state<Entry|null>(null);
 let spectrum=$state<number[]>([]);
 let selected=$state<Entry|null>(null);
 let view=$state('record');
 let error=$state('');let notice=$state('');let busy=$state(false);let captureBusy=$state(false);let loading=$state(true);
 let settingsView=$state<SettingsView>();let pendingAction:(()=>void|Promise<void>)|null=null;
 let confirm=$state<'delete'|'clear'|'quit'|'discard'|'edit'|'text'|'unsaved'|null>(null);
 let title=$state('');let transcript=$state('');let textKind=$state<'transcript'|'transformed'>('transcript');
 let dialog=$state<HTMLDialogElement>();let clock=$state(Date.now());let refreshKey=$state(0);let compact=$state(false);let fullscreen=$state(false);let prefersDark=$state(false);let playback=$state<Playback|null>(null);
 let noticeTimer:ReturnType<typeof setTimeout>;
 let hasPending=$derived(snapshot?.jobs.some(j=>['queued','running','canceling','paused','unknown'].includes(j.status))??false);
 let paused=$derived(snapshot?.jobs.filter(j=>['paused','unknown'].includes(j.status))??[]);
 let latestJob=$derived(snapshot?.jobs.filter(j=>j.entry_id===latest?.id).sort((a,b)=>b.seq-a.seq)[0]);
 let editPending=$derived(snapshot?.jobs.some(j=>j.entry_id===selected?.id&&['queued','running','canceling','paused','unknown'].includes(j.status))??false);
 let elapsed=$derived(snapshot?.capture.started?(clock-Date.parse(snapshot.capture.started))/1000:0);
 let theme=$derived(snapshot?.settings.theme??'system');
 let dark=$derived(theme==='night'||theme==='system'&&prefersDark);
 $effect(()=>{if(confirm&&dialog&&!dialog.open)dialog.showModal();});
 $effect(()=>{if(!snapshot?.capture.active)return;clock=Date.now();const timer=setInterval(()=>clock=Date.now(),250);return()=>clearInterval(timer);});
 $effect(()=>{if(!notice)return;clearTimeout(noticeTimer);noticeTimer=setTimeout(()=>notice='',2500);return()=>clearTimeout(noticeTimer);});
 const readUpdates=refreshQueue(async full=>{
  try{
   const [state,progress]=await Promise.all([full||!snapshot?api.state():Promise.resolve(null),api.progress()]);
   const base=state??snapshot;
   if(base)snapshot={...base,jobs:progress.jobs,capture:progress.capture,playing:progress.playback?.id??null,diagnostics:progress.diagnostics,operation_busy:progress.operation_busy,operation_stage:progress.operation_stage};
   latest=progress.latest;playback=progress.playback;refreshKey++;
  }catch(e){error=message(e);}finally{loading=false;}
 });
 function refresh(){return readUpdates(true);}
 function refreshProgress(){return readUpdates(false);}
 async function run(work:()=>Promise<unknown>,success=''){error='';notice='';busy=true;try{await work();notice=success;await refresh();return true;}catch(e){error=message(e);return false;}finally{busy=false;}}
 async function control(action:string){error='';captureBusy=true;try{await api.capture(action);await refreshProgress();}catch(e){error=message(e);}finally{captureBusy=false;}}
 const write=settingsWriter(async()=>(await api.state()).settings,api.settings,async saved=>{await refresh();if(snapshot&&snapshot.settings.revision<=saved.revision)snapshot={...snapshot,settings:saved};});
 function guard(action:()=>void|Promise<void>){if(settingsView?.isDirty()){pendingAction=action;confirm='unsaved';}else void action();}
 async function continueNavigation(save:boolean){error='';if(save&&!await settingsView?.saveAll()){error='Could not save your changes. Keep editing to review the highlighted settings error.';return;}if(!save)settingsView?.discardAll();const action=pendingAction;pendingAction=null;confirm=null;await action?.();}
 function jobLabel(job:Job){if(job.status==='complete')return job.delivery==='copy_failed'?'Saved · clipboard busy; use Copy':job.delivery==='copied'?'Copied to clipboard':'Saved';if(job.status==='unknown')return 'Response lost · retry may incur another charge';return job.status==='running'?job.stage:job.status;}
 async function navigate(next:string){if(compact&&next!=='record'){if(!await run(()=>api.windowMode('full')))return;compact=false;}view=next;if(next==='settings')await refresh();}
 function go(next:string){guard(()=>navigate(next));}
 async function setMode(next:string){if(await run(()=>api.windowMode(next))){if(next==='fullscreen')fullscreen=!fullscreen;else{compact=next==='compact';fullscreen=false;if(compact)view='record';}}}
 function mode(next:string){if(next==='compact')guard(()=>setMode(next));else void setMode(next);}
 async function toggleTheme(){await run(()=>write(s=>({...s,theme:nextTheme(s.theme)})));}
 function edit(entry:Entry){selected=entry;title=entry.title;transcript=entry.transcript??'';confirm='edit';}
 function remove(entry:Entry){selected=entry;confirm='delete';}
 function inspect(entry:Entry,kind:'transcript'|'transformed'){selected=entry;textKind=kind;confirm='text';}
 onMount(()=>{
  compact=innerWidth<=500&&innerHeight<=450;
  const media=matchMedia('(prefers-color-scheme: dark)');prefersDark=media.matches;const changed=(e:MediaQueryListEvent)=>prefersDark=e.matches;media.addEventListener('change',changed);
  void api.updateStatus().then(updateStatus.set).catch(()=>undefined);
  const subscriptions=[listen<number[]>('capture-spectrum',e=>spectrum=e.payload),listen<UpdateStatus>('update-changed',e=>updateStatus.set(e.payload)),listen('state-changed',refreshProgress),listen('configuration-changed',refresh),listen('quit-requested',()=>guard(()=>{confirm='quit';})),listen('playback-error',e=>error=message(e.payload))];
  void refresh();let polling=false;const timer=setInterval(async()=>{if(!playback||polling)return;polling=true;try{playback=await api.playbackState();}catch{/* The ordinary state refresh reports connection errors. */}finally{polling=false;}},250);
  return()=>{clearInterval(timer);clearTimeout(noticeTimer);media.removeEventListener('change',changed);for(const p of subscriptions)void p.then(unlisten=>unlisten());};
 });
</script>

<svelte:window onkeydown={e=>{if(e.key==='Escape'&&fullscreen&&!confirm)void mode('fullscreen');}}/>
<div class="app" role="region" aria-label="x-talk workspace" onpointermove={e=>{const r=e.currentTarget.getBoundingClientRect();e.currentTarget.style.setProperty('--light-x',`${e.clientX-r.left}px`);e.currentTarget.style.setProperty('--light-y',`${e.clientY-r.top}px`);}} data-theme={dark?'night':'day'} class:compact>
 <nav class="nav-rail" aria-label="Main navigation">
  <button class="rail-home" class:chosen={view==='record'} class:live={snapshot?.capture.active} onclick={()=>go('record')} aria-label="Recorder" title="Recorder" aria-current={view==='record'?'page':undefined}><img src={heart} alt="x-talk" width="36" height="36"></button>
  <div class="rail-divider"></div>
  <button class="rail-button" class:chosen={view==='recordings'} onclick={()=>go('recordings')} aria-label="Recordings" title="Recordings" aria-current={view==='recordings'?'page':undefined}><Icon name="list"/></button>
  <button class="rail-button" class:chosen={view==='settings'} onclick={()=>go('settings')} aria-label="Settings" title="Settings" aria-current={view==='settings'?'page':undefined}><Icon name="gear"/></button>
  <div class="rail-divider"></div>
  <button class="rail-button" disabled={!snapshot||busy} onclick={()=>mode(compact?'full':'compact')} aria-label={compact?'Expand workspace':'Compact view'} title={compact?'Expand workspace':'Compact view'}><Icon name="compact"/></button>
  <button class="rail-button" disabled={!snapshot||busy} onclick={()=>mode('fullscreen')} aria-label={fullscreen?'Exit full screen':'Full screen'} title={fullscreen?'Exit full screen':'Full screen'}><Icon name="fullscreen"/></button>
  <div class="rail-bottom"><button class="rail-button" disabled={!snapshot||busy} onclick={toggleTheme} aria-label={`Theme: ${theme==='system'?'Auto (follow system)':theme==='day'?'Light':'Dark'}`} title={`Theme: ${theme==='system'?'Auto (follow system)':theme==='day'?'Light':'Dark'}`}><Icon name={theme==='system'?'auto':theme==='day'?'sun':'moon'}/></button><button class="rail-button" onclick={()=>run(api.repository)} aria-label="GitHub repository" title="GitHub repository"><Icon name="github"/></button></div>
 </nav>
 <div class="main-shell">
  {#if $updateStatus.phase==='available'&&view!=='settings'&&!compact}<div class="banner" role="status"><span>x-talk {$updateStatus.version} is available.</span><button onclick={()=>go('settings')}>View update</button></div>{/if}
  {#if error}<div class="banner error" role="alert"><span>{error}</span><button class="icon-button" onclick={()=>error=''} aria-label="Dismiss error"><Icon name="close" size={16}/></button></div>{/if}
  {#if snapshot?.capture.error}<div class="banner error" role="alert">{snapshot.capture.error.message}</div>{/if}
  {#if snapshot?.shortcut_error}<div class="banner error">{snapshot.shortcut_error.message} Use the Recorder to record.</div>{/if}
  {#each snapshot?.recovery_warnings??[] as warning}<div class="banner recovery-warning" role="alert">{warning.message}</div>{/each}
  {#if paused.length}<section class="recovery"><div><strong>{paused.length} interrupted job{paused.length===1?'':'s'}</strong><small>Paused until you resume. Fresh recordings can continue.</small></div><button disabled={busy||snapshot?.operation_busy||paused.every(j=>j.status==='unknown')} onclick={()=>run(()=>api.recover(paused.filter(j=>j.status==='paused').map(j=>j.id),false))}>Resume available</button><button class="subtle-button" disabled={busy} onclick={()=>confirm='discard'}>Discard pending</button></section>{/if}
  {#if loading}<main class="empty">Opening your workspace…</main>
  {:else if !snapshot}<main class="empty"><h1>Could not connect to x-talk</h1><p>{error||'Open this interface through the desktop application.'}</p><button onclick={refresh}>Retry connection</button></main>
  {:else if view==='record'}
   <Recorder {spectrum} {snapshot} entry={latest} {elapsed} {captureBusy} {busy} {compact} {control} copy={(id,kind)=>run(()=>api.copy(id,kind),'Copied')} importAudio={()=>run(api.import)}/>
   {#if latestJob&&['failed','unknown','paused'].includes(latestJob.status)}<div class="home-job-error stage-error" role="alert"><strong>{jobLabel(latestJob)}</strong>{#if latestJob.error}<p>{latestJob.error.message}</p>{/if}<button class="subtle-button" onclick={()=>go('recordings')}>View recording</button></div>{/if}
  {:else if view==='recordings'}<RecordingsTable {snapshot} {refreshKey} {busy} {run} {edit} {remove} clearAll={()=>{error='';confirm='clear';}} {inspect} {playback}/>
  {:else}<SettingsView bind:this={settingsView} {snapshot} {write} {refresh} {guard}/>
  {/if}
 </div>
 {#if notice}<div class="toast" role="status"><Icon name="check" size={16}/>{notice}</div>{/if}
 {#if confirm}<dialog bind:this={dialog} class="modal" class:wide={confirm==='edit'||confirm==='text'} aria-labelledby="dialog-title" oncancel={()=>confirm=null} onclose={()=>confirm=null}>
  <div class="dialog-heading"><h2 id="dialog-title">{confirm==='edit'?'Edit recording':confirm==='text'?(textKind==='transcript'?'Transcript':'Transformed output'):confirm==='delete'?'Delete this recording?':confirm==='clear'?'Clear all recordings?':confirm==='discard'?'Discard interrupted work?':confirm==='unsaved'?'Save your changes?':'Quit x-talk?'}</h2><button class="icon-button" onclick={()=>confirm=null} aria-label="Close dialog"><Icon name="close" size={18}/></button></div>
  {#if confirm==='edit'&&selected}<form onsubmit={e=>{e.preventDefault();void run(()=>api.editRecording(selected!,title,selected!.subtitle,transcript),'Recording saved').then(saved=>{if(saved)confirm=null;});}}><label>Title<input bind:value={title} maxlength="500" required></label><label>Transcript<textarea rows="8" bind:value={transcript} disabled={editPending} maxlength="1000000"></textarea></label><p class="help">{editPending?'Finish or discard pending work before editing the transcript.':'Editing the transcript preserves previous transformed output and marks it out of date. Retry transformation to refresh it.'}</p><div class="actions dialog-actions"><button type="button" onclick={()=>confirm=null}>Cancel</button><button class="primary" disabled={busy}>Save changes</button></div></form>
  {:else if confirm==='text'&&selected}<div class="text-heading"><p class="muted">{selected.title}</p><CopyButton available={!!selected[textKind]} label="Copy full text" copy={()=>run(()=>api.copy(selected!.id,textKind),'Copied')}/></div><div class="output full-text" role="textbox" aria-label="Full saved text" aria-readonly="true" aria-multiline="true" tabindex="0">{selected[textKind]}</div>
  {:else if confirm==='unsaved'}<p>You have unsaved text changes. Save them before leaving this view?</p><div class="actions dialog-actions"><button onclick={()=>{confirm=null;pendingAction=null;}}>Keep editing</button><button disabled={busy} onclick={()=>continueNavigation(false)}>Discard changes</button><button class="primary" disabled={busy} onclick={()=>continueNavigation(true)}>Save and continue</button></div>
  {:else if confirm==='clear'}<p>This permanently deletes your entire recordings library and its saved audio, including paused work. Search and pagination do not limit this action. Imported source files are preserved.</p><div class="actions dialog-actions"><button disabled={busy} onclick={()=>confirm=null}>Cancel</button><button class="danger" disabled={busy||snapshot?.capture.active||snapshot?.jobs.some(j=>['queued','running','canceling'].includes(j.status))} onclick={()=>{void run(api.clearRecordings,'Recordings cleared').then(cleared=>{if(cleared){selected=null;confirm=null;}});}}>Clear all recordings</button></div>
  {:else}{#if confirm!=='quit'||snapshot?.capture.active||hasPending}<p>{confirm==='delete'?'This removes the app-managed audio and all saved text. Your imported source file is preserved.':confirm==='discard'?'Saved history and text stay available. Pending work will be discarded and audio retention will apply.':snapshot?.capture.active?'Choose whether to save or discard the active recording. Pending processing will pause after exit.':'Pending processing will pause until you explicitly resume it after reopening.'}</p>{/if}<div class="actions dialog-actions"><button onclick={()=>confirm=null}>Keep working</button>{#if confirm==='delete'}<button class="danger" disabled={busy} onclick={()=>{void run(()=>api.delete(selected!),'Recording deleted').then(saved=>{if(saved){selected=null;confirm=null;}});}}>Delete recording</button>{:else if confirm==='discard'}<button class="danger" disabled={busy} onclick={()=>{void run(()=>api.recover(paused.map(j=>j.id),true),'Pending work discarded').then(saved=>{if(saved)confirm=null;});}}>Discard work</button>{:else}{#if snapshot?.capture.active}<button class="danger" disabled={busy} onclick={()=>run(()=>api.quit(true))}>Discard and quit</button>{/if}<button class="primary" disabled={busy} onclick={()=>run(()=>api.quit(false))}>{snapshot?.capture.active?'Stop and quit':'Quit'}</button>{/if}</div>{/if}
  {#if error}<p class="error-text" role="alert">{error}</p>{/if}
 </dialog>{/if}
</div>
