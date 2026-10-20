<script lang="ts">
 import {onMount} from 'svelte';
 import {listen} from '@tauri-apps/api/event';
 import {meetings,stageLabel,type Summary,type View,type ImportPreview,type Edit,type Evidence} from '../../meetings/ipc';
 import {api,duration,message} from '../../ipc';
 import Icon from '../Icon.svelte';
 let rows=$state<Summary[]>([]), selected=$state<View|null>(null), preview=$state<ImportPreview|null>(null);
 let search=$state(''), filter=$state(''), speakerFilter=$state(''), original=$state(false);
 let tab=$state('brief'), busy=$state(false), error=$state(''), notice=$state('');
 let retain=$state<number|null>(50), retentionChoice=$state('50');
 let title=$state(''), callAt=$state('');
 let editTurn=$state(''), editText=$state(''), editSpeaker=$state('');
 let mergeFrom=$state(''), mergeInto=$state('');
 let confirmation=$state<'delete'|'retry'|'merge'|null>(null);
 let dialog=$state<HTMLDialogElement>();
 let readVersion=0;
 let restored=false;
 function remember(){try{localStorage.setItem('xtalk.meetings.view.v1',JSON.stringify({id:selected?.meeting.id??null,tab}));}catch{/* View preferences must not block meetings. */}}
 $effect(()=>{void selected;void tab;if(restored)remember();});
 const current=$derived(selected?.meeting);
 const transcript=$derived(current?.transcript);
 const brief=$derived(current?.brief.content);
 const filtered=$derived(rows.filter(m=>`${m.title} ${m.preview??''}`.toLowerCase().includes(search.toLowerCase())));
 const turns=$derived((transcript?.turns??[]).filter(t=>(!speakerFilter||t.speaker_id===speakerFilter)&&(original?t.original_text:t.text).toLowerCase().includes(filter.toLowerCase())));
 const editingBlocked=$derived(!!selected?.replacement&&!['failed','complete'].includes(current?.transcription_status??''));
 const canRetry=$derived(!!current&&['complete','failed','submission_unknown','recovered','waiting_for_keys'].includes(current.transcription_status));
 $effect(()=>{if(confirmation&&dialog&&!dialog.open)dialog.showModal();});
 async function load(){
  const version=++readVersion,id=selected?.meeting.id;
  try{const [list,detail]=await Promise.all([meetings.list(),id?meetings.get(id):Promise.resolve(null)]);
   if(version!==readVersion)return;rows=list;if(id&&selected?.meeting.id===id)selected=detail;
  }catch(e){error=message(e);}
 }
 async function run(work:()=>Promise<unknown>,success=''){
  busy=true;error='';notice='';
  try{await work();notice=success;await load();return true;}catch(e){error=message(e);return false;}finally{busy=false;}
 }
 async function open(id:string){await run(async()=>{selected=await meetings.get(id);tab='brief';filter='';speakerFilter='';editTurn='';title=selected.meeting.title;callAt=selected.meeting.call_at;});}
 async function edit(change:Edit){if(!current)return;const {id,revision}=current;return run(async()=>{selected=await meetings.edit(id,revision,change);},'Saved');}
 function speakerColor(id:string){let hash=0;for(const c of id)hash=(hash*31+c.charCodeAt(0))>>>0;return `hsl(${hash%360} 45% 55%)`;}
 function name(id:string){return transcript?.speakers.find(s=>s.id===id)?.name??`Speaker ${id}`;}
 function supporting(id:string){tab='transcript';filter='';speakerFilter='';setTimeout(()=>document.getElementById(`turn-${id}`)?.scrollIntoView({block:'center',behavior:'smooth'}),0);}
 onMount(()=>{
  void (async()=>{
   let saved:{id?:string;tab?:string}={};
   try{saved=JSON.parse(localStorage.getItem('xtalk.meetings.view.v1')??'{}');}catch{/* Ignore obsolete view preferences. */}
   await load();
   if(!selected&&saved.id&&rows.some(row=>row.id===saved.id)){
    try{selected=await meetings.get(saved.id);title=selected.meeting.title;callAt=selected.meeting.call_at;
     if(['brief','transcript','speakers','details'].includes(saved.tab??''))tab=saved.tab!;
    }catch{/* A removed meeting returns to the library. */}
   }
   restored=true;remember();
  })();void meetings.retention().then(v=>{retain=v;retentionChoice=v===null?'unlimited':String(v);}).catch(e=>error=message(e));
  const subscription=listen('meetings-changed',load);
  return()=>{readVersion++;void subscription.then(f=>f());};
 });
</script>
<main class="meetings-page">
 {#if error}<div class="banner error" role="alert">{error}</div>{/if}
 {#if notice}<p class="meeting-notice" role="status">{notice}</p>{/if}
 {#if !current}
  <div class="page-heading"><div><p class="eyebrow">ROOM FOR EVERY VOICE</p><h1>Meetings <span class="count">{rows.length}</span></h1><p class="muted">The conversation, the people, and what comes next.</p></div>
   <button class="primary" disabled={busy} onclick={()=>run(async()=>{preview=await meetings.pick();})}><Icon name="upload" size={17}/>Import recording</button>
  </div>
  {#if preview}<section class="meeting-panel import-preview"><h2>{preview.filename}</h2><p>{duration(preview.inspection.duration)} · {preview.inspection.channels===2?'Stereo':'Mono'} audio</p>
   {#if preview.inspection.video}<p>Only extracted audio will be saved. The video stays in its original location.</p>{/if}
   {#if preview.inspection.audio_streams>1}<p class="banner">This file contains {preview.inspection.audio_streams} audio tracks. Only the first audio track will be imported; the others will be omitted.</p>{/if}
   <div class="actions"><button disabled={busy} onclick={()=>preview=null}>Cancel</button><button class="primary" disabled={busy} onclick={()=>run(async()=>{const id=await meetings.import(preview!.token);preview=null;selected=await meetings.get(id);title=selected.meeting.title;callAt=selected.meeting.call_at;})}>{busy?'Saving audio…':'Import and transcribe'}</button></div>
  </section>{/if}
  <div class="table-toolbar"><label class="search-field"><Icon name="search" size={17}/><input aria-label="Search meetings" placeholder="Search titles and briefs…" bind:value={search}/></label>
   <details class="meeting-retention"><summary>Audio retention</summary><label>Recordings to keep<input aria-label="Meeting audio retention" bind:value={retentionChoice} placeholder="50 or unlimited"/></label><p class="help">Keep the latest N recordings, or enter unlimited. Text and briefs stay.</p><button disabled={busy} onclick={()=>run(async()=>{const n=retentionChoice.trim().toLowerCase()==='unlimited'?null:Number(retentionChoice);if(n!==null&&(!Number.isInteger(n)||n<1))throw new Error('Enter a positive number or unlimited.');await meetings.setRetention(n);retain=n;},'Audio retention saved')}>Save</button><small>Currently {retain===null?'unlimited':retain} recordings</small></details>
  </div>
  <div class="meeting-library">
   {#each filtered as row (row.id)}<article class="meeting-card">
    <div class="meeting-card-heading"><button class="meeting-title" onclick={()=>open(row.id)}>{row.title}</button><span class="badge">{stageLabel(row.status)}</span></div>
    <p class="recording-meta">{new Date(row.added_at).toLocaleString()} · {duration(row.duration??0)} · {row.speakers} speakers{#if !row.audio_available} · Audio not retained{/if}</p>
    <div class="meeting-preview"><button class="text-preview" onclick={()=>open(row.id)}>{row.preview??'Your meeting brief will appear here.'}</button><button class="subtle-button" disabled={busy} onclick={()=>run(()=>meetings.copy(row.id),'Meeting JSON copied')}><Icon name="copy" size={15}/>Copy meeting JSON</button></div>
   </article>
   {:else}<div class="empty"><Icon name="heart" size={32}/><h2>{search?'No meetings found':'Every conversation has a next step'}</h2><p>{search?'Try another search.':'Import an audio or video recording to create a transcript and meeting brief.'}</p></div>{/each}
  </div>
 {:else}
  <button class="subtle-button" onclick={()=>{selected=null;editTurn='';confirmation=null;readVersion++;void load();}}><Icon name="left" size={16}/>All meetings</button>
  <div class="page-heading"><div><p class="eyebrow">{stageLabel(current.transcription_status)}</p><h1>{current.title}</h1><p class="muted">{duration(current.duration_seconds??0)} · {transcript?.speakers.length??0} speakers</p></div>
   <button disabled={busy} onclick={()=>run(()=>meetings.copy(current!.id),'Meeting JSON copied')}><Icon name="copy" size={16}/>Copy meeting JSON</button></div>
  {#if current.capture_warning}<p class="banner" role="status">{current.capture_warning}</p>{/if}
  {#if selected?.error}<div class="banner error" role="alert">{selected.error}</div>{/if}
  {#if current.transcription_status==='submission_unknown'}<p class="banner">The provider may have accepted the previous request. Review its dashboard before retrying; another request may incur another charge.</p>{/if}
  <div class="meeting-tabs" role="tablist" aria-label="Meeting review">
   {#each [['brief','Meeting brief'],['transcript','Transcript'],['speakers','Manage speakers'],['details','Details']] as [key,label]}<button role="tab" aria-selected={tab===key} class:chosen={tab===key} onclick={()=>tab=key}>{label}</button>{/each}
  </div>
  <section class="meeting-panel">
  {#if tab==='brief'}
   <div class="meeting-card-heading"><h2>Meeting brief</h2><button class="subtle-button" disabled={busy||!transcript||!canRetry} onclick={()=>edit({kind:'retry',transcription:false})}><Icon name="spark" size={16}/>Regenerate brief</button></div>
   {#if current.brief.stale}<p class="banner">This brief predates transcript or speaker changes. Regenerate it to reflect your edits.</p>{/if}
   {#if brief}<p class="meeting-summary">{brief.summary}</p>
    {#each ([['Topics',brief.topics],['Decisions',brief.decisions],['Open questions',brief.open_questions]] as [string,Evidence[]][]) as section}
     <h3>{section[0]}</h3>
     {#each section[1] as item}<div class="brief-item"><p>{item.text}</p><div class="evidence-links">{#each item.segment_ids as id}<button class="subtle-button" onclick={()=>supporting(id)}>{duration(transcript?.turns.find(t=>t.id===id)?.start??0)}</button>{/each}</div></div>{:else}<p class="muted">None recorded.</p>{/each}
    {/each}
    <h3>Action items</h3>{#each brief.actions as action}<div class="brief-item"><p>{action.text}</p><small>{action.owner??'No owner stated'} · {action.due??'No due date stated'}</small><div class="evidence-links">{#each action.segment_ids as id}<button class="subtle-button" onclick={()=>supporting(id)}>{duration(transcript?.turns.find(t=>t.id===id)?.start??0)}</button>{/each}</div></div>{:else}<p class="muted">No explicit commitments recorded.</p>{/each}
    <h3>Suggested follow-up questions</h3>{#each brief.suggested_follow_ups as question}<p>{question}</p>{:else}<p class="muted">None suggested.</p>{/each}
   {:else}<div class="empty"><Icon name="spark" size={30}/><h2>{current.brief.status==='failed'?'Brief needs attention':'Your brief is on its way'}</h2><p>{current.brief.error??'The transcript is saved independently. You can review and copy it as soon as it is ready.'}</p></div>{/if}
  {:else if tab==='transcript'}
   <div class="table-toolbar"><label class="search-field"><Icon name="search" size={16}/><input bind:value={filter} placeholder="Find in transcript…" aria-label="Search transcript"/></label><select bind:value={speakerFilter} aria-label="Filter speaker"><option value="">All speakers</option>{#each transcript?.speakers??[] as speaker}<option value={speaker.id}>{speaker.name}</option>{/each}</select><label class="check"><input type="checkbox" bind:checked={original}/>Original text</label></div>
   <p class="help">Copy meeting JSON always includes the complete corrected transcript, original evidence, and brief, regardless of these filters.</p>
   {#each turns as turn (turn.id)}<article class="meeting-turn" id={`turn-${turn.id}`} style:border-left={`3px solid ${speakerColor(turn.speaker_id)}`}>
    <div class="meeting-card-heading"><strong>{name(original?turn.original_speaker_id:turn.speaker_id)}</strong><button class="subtle-button" disabled={busy||!current.audio_available} onclick={()=>run(()=>meetings.play(current!.id,turn.start))}>{duration(turn.start)}–{duration(turn.end)} · Play</button></div>
    {#if editTurn===turn.id}<label>Speaker<select bind:value={editSpeaker}>{#each transcript?.speakers??[] as speaker}<option value={speaker.id}>{speaker.name}</option>{/each}</select></label><label>Turn text<textarea rows="4" bind:value={editText}></textarea></label><div class="actions"><button onclick={()=>editTurn=''}>Cancel</button><button disabled={busy||editingBlocked} onclick={async()=>{if(await edit({kind:'correct',turn:turn.id,correction:{text:editText,speaker:editSpeaker}}))editTurn='';}}>Save turn</button></div>
    {:else}<p class="turn-text">{original?turn.original_text:turn.text}</p><div class="actions"><button class="subtle-button" disabled={busy||editingBlocked} onclick={()=>{editTurn=turn.id;editText=turn.text;editSpeaker=turn.speaker_id;}}>Edit turn</button>{#if turn.correction.text!==null||turn.correction.speaker!==null}<button class="subtle-button" disabled={busy||editingBlocked} onclick={()=>edit({kind:'correct',turn:turn.id,correction:null})}>Restore original turn</button>{/if}</div>{/if}
   </article>{:else}<p class="empty">{transcript?'No turns match these filters.':'The transcript will appear here after processing.'}</p>{/each}
  {:else if tab==='speakers'}
   <h2>The people in this conversation</h2><p class="help">Estimated time measures diarized turn coverage, including pauses. Speakers may overlap.</p>
   {#each transcript?.speakers??[] as speaker}<form class="speaker-row" onsubmit={e=>{e.preventDefault();const data=new FormData(e.currentTarget);void edit({kind:'name',speaker:speaker.id,name:String(data.get('name'))});}}><label>Speaker name<input name="name" value={speaker.name} maxlength="200" required/></label><small>{speaker.turn_count} turns · {speaker.word_count} words · {duration(speaker.estimated_turn_coverage_seconds)}</small><button disabled={busy||editingBlocked}>Save name</button></form>{/each}
   <h3>Merge speakers</h3><p class="help">Combine two labels that refer to the same person. Merges can be undone in reverse order.</p><div class="actions"><select bind:value={mergeFrom} aria-label="Speaker to merge"><option value="">Choose speaker</option>{#each transcript?.speakers??[] as s}<option value={s.id}>{s.name}</option>{/each}</select><span>into</span><select bind:value={mergeInto} aria-label="Merge destination"><option value="">Choose speaker</option>{#each transcript?.speakers??[] as s}<option value={s.id}>{s.name}</option>{/each}</select><button disabled={busy||editingBlocked||!mergeFrom||!mergeInto||mergeFrom===mergeInto} onclick={()=>confirmation='merge'}>Preview merge</button><button disabled={busy||editingBlocked||!selected?.merges} onclick={()=>edit({kind:'undo_merge'})}>Undo last merge</button></div>
  {:else}
   <h2>Meeting details</h2><form onsubmit={e=>{e.preventDefault();void edit({kind:'metadata',title,call_at:callAt});}}><label>Title<input bind:value={title} maxlength="500" required/></label><label>Meeting date<input bind:value={callAt} required/></label><button disabled={busy}>Save details</button></form>
   <dl><dt>Source</dt><dd>{current.source}{current.filename?` · ${current.filename}`:''}</dd><dt>Audio</dt><dd>{current.audio_available?'Retained':'No longer retained; transcript and brief remain available'}</dd><dt>Transcription</dt><dd>{transcript?`${transcript.provider} · ${transcript.model??'Model not reported'}`:'Pending'}</dd><dt>Brief</dt><dd>{current.brief.provider} · {current.brief.actual_model??current.brief.model}</dd></dl>
   <div class="actions"><button disabled={busy||!current.audio_available} onclick={()=>run(()=>meetings.play(current!.id))}>Play audio</button><button disabled={busy} onclick={()=>run(api.stopPlayback)}>Stop playback</button><button disabled={busy||!current.audio_available} onclick={()=>run(()=>meetings.download(current!.id),'Audio saved')}><Icon name="download" size={16}/>Download audio</button><button disabled={busy||!current.audio_available||!canRetry} onclick={()=>confirmation='retry'}>Re-transcribe</button><button class="danger" disabled={busy} onclick={()=>confirmation='delete'}>Delete meeting</button></div>
  {/if}
  </section>
 {/if}
 {#if confirmation&&current}<dialog class="modal" bind:this={dialog} oncancel={()=>confirmation=null} onclose={()=>confirmation=null} aria-labelledby="meeting-confirm-title"><h2 id="meeting-confirm-title">{confirmation==='delete'?'Delete this meeting?':confirmation==='retry'?'Re-transcribe this meeting?':'Merge these speakers?'}</h2><p>{confirmation==='delete'?'This deletes the saved transcript, brief, speaker edits and retained audio. Your imported source file is preserved.':confirmation==='retry'?'This may incur a provider charge. Existing results remain available until success. A successful replacement resets speaker names, merges and transcript corrections.':`${name(mergeFrom)} will be merged into ${name(mergeInto)} across the entire transcript. This marks the brief out of date.`}</p><div class="actions"><button onclick={()=>confirmation=null}>Cancel</button><button class="primary" disabled={busy} onclick={async()=>{let success=false;if(confirmation==='delete'){const id=current!.id,revision=current!.revision;success=await run(async()=>{await meetings.delete(id,revision);selected=null;});}else if(confirmation==='retry')success=!!await edit({kind:'retry',transcription:true});else success=!!await edit({kind:'merge',from:mergeFrom,into:mergeInto});if(success)confirmation=null;}}>Confirm</button></div></dialog>{/if}
</main>
<style>
 .meetings-page{padding:32px;max-width:1240px;margin:0 auto;width:100%;overflow:auto}
 .meetings-page h1{font-size:28px;margin:0}.meeting-library{display:grid;gap:16px;margin-top:20px}
 .meeting-card,.meeting-panel{border:1px solid var(--line);background:var(--surface);border-radius:18px;padding:24px}
 .meeting-card-heading{display:flex;align-items:center;justify-content:space-between;gap:16px}
 .meeting-title{border:0;background:transparent;padding:0;font-size:19px;text-align:left;font-weight:600}
 .meeting-preview{display:flex;align-items:flex-start;gap:20px;margin-top:18px}.meeting-preview .text-preview{flex:1;line-height:1.65}
 .meeting-tabs{display:flex;gap:8px;margin:20px 0;flex-wrap:wrap}.meeting-tabs button{border-radius:20px}.meeting-tabs .chosen{background:var(--accent-soft);font-weight:600}
 .meeting-summary{font-size:17px;line-height:1.8}.brief-item{padding:8px 0 12px;border-bottom:1px solid var(--line)}.brief-item p{margin:8px 0}
 .evidence-links{display:flex;gap:8px}.meeting-turn{padding:22px 16px;border-bottom:1px solid var(--line);scroll-margin:24px}
 .turn-text{white-space:pre-wrap;line-height:1.7}.meeting-panel h3{margin-top:28px}
 .speaker-row{display:flex;gap:16px;align-items:end;padding:16px 0}.speaker-row label{flex:1}.speaker-row small{padding-bottom:10px}
 .meeting-retention{position:relative;max-width:300px}.meeting-retention small{display:block;padding:8px}
 .meeting-panel label{display:block;margin:12px 0}.meeting-panel input,.meeting-panel textarea{width:100%}.meeting-panel select{max-width:250px}
 .meeting-panel dl{display:grid;grid-template-columns:120px 1fr;gap:16px;margin:28px 0}.meeting-panel dd{margin:0}
 .meeting-notice{color:var(--accent);margin:0 0 12px}
 @media(max-width:700px){.meetings-page{padding:20px}.page-heading,.meeting-preview,.speaker-row{flex-direction:column;align-items:stretch}.meeting-card-heading{align-items:flex-start}.meeting-panel{padding:18px}}
</style>
