<script lang="ts">
 import Pagination from './Pagination.svelte';
 import Activity from './Activity.svelte';
 import {api,message,duration} from '../ipc';
 import type {AppState,Entry,Job,Playback} from '../types';
 import {defaultColumns,restoreColumns,moveColumn,wordCount,preview} from '../recordings';
 import Icon from './Icon.svelte';import CopyButton from './CopyButton.svelte';import AudioPlayer from './AudioPlayer.svelte';
 let {snapshot,refreshKey,busy,run,edit,remove,clearAll,inspect,playback}: {snapshot:AppState;refreshKey:number;busy:boolean;run:(work:()=>Promise<unknown>,success?:string)=>Promise<boolean>;edit:(e:Entry)=>void;remove:(e:Entry)=>void;clearAll:()=>void;inspect:(e:Entry,kind:'transcript'|'transformed')=>void;playback:Playback|null}=$props();
 let pageSize=$state(50);
 let entries=$state<Entry[]>([]);let total=$state(0);let page=$state(0);let search=$state('');let query=$state('');let sort=$state('created');let descending=$state(true);let loading=$state(true);let error=$state('');let request=0;
 let columns=$state(restoreColumns(localStorage.getItem('xtalk.recordings.columns.v1')));
 let visible=$derived(columns.filter(c=>c.visible));
 $effect(()=>{try{localStorage.setItem('xtalk.recordings.columns.v1',JSON.stringify(columns));}catch{/* Cosmetic preferences must not block work. */}});
 $effect(()=>{const text=search;const timer=setTimeout(()=>{query=text;page=0;},250);return()=>clearTimeout(timer);});
 $effect(()=>{const key=refreshKey;const offset=page*pageSize;const filter=query;const order=sort;const desc=descending;void key;void load(offset,filter,order,desc,pageSize);});
 async function load(offset:number,search:string,sort:string,descending:boolean,page_size:number){
  const id=++request;loading=true;
  try{const result=await api.recordings({offset,search,sort,descending,page_size});if(id!==request)return;entries=result.entries;total=result.total;error='';if(page>0&&offset>=total)page=Math.max(0,Math.ceil(total/pageSize)-1);}catch(e){if(id===request)error=message(e);}finally{if(id===request)loading=false;}
 }
 function sortBy(key:string){if(sort===key)descending=!descending;else{sort=key;descending=key==='created';}page=0;}
 function jobs(e:Entry){return snapshot.jobs.filter(j=>j.entry_id===e.id).sort((a,b)=>b.seq-a.seq);}
 function pending(e:Entry){return jobs(e).some(j=>['queued','running','canceling','paused','unknown'].includes(j.status));}
 function retryDisabled(e:Entry){return busy||snapshot.operation_busy||snapshot.capture.active||(pending(e)&&jobs(e)[0]?.status!=='unknown');}
 function status(j:Job){if(j.status==='unknown')return 'Response lost · retry may incur another charge';return j.status==='running'?j.stage:j.status;}
 function resize(event:PointerEvent,key:string){
  const column=columns.find(c=>c.key===key)!;const initial=column.width;const x=event.clientX;const target=event.currentTarget as HTMLElement;target.setPointerCapture(event.pointerId);
  const move=(e:PointerEvent)=>{column.width=Math.max(100,Math.min(700,initial+e.clientX-x));};
  const end=()=>{target.removeEventListener('pointermove',move);target.removeEventListener('pointerup',end);target.removeEventListener('pointercancel',end);};
  target.addEventListener('pointermove',move);target.addEventListener('pointerup',end);target.addEventListener('pointercancel',end);
 }
</script>
<main class="recordings-page">
 <div class="page-heading"><div><p class="eyebrow">YOUR WORKSPACE</p><h1>Transcripts <span class="count">{total}</span></h1></div><button class="danger plain" disabled={busy||snapshot.capture.active||snapshot.jobs.some(j=>['queued','running','canceling'].includes(j.status))} onclick={clearAll}><Icon name="trash" size={16}/>Clear All</button></div>
 <div class="table-toolbar"><label class="search-field"><Icon name="search" size={17}/><input aria-label="Search recordings" placeholder="Search transcript and transformed text…" maxlength="500" bind:value={search}>{#if search}<button class="icon-button" onclick={()=>search=''} title="Clear search" aria-label="Clear search"><Icon name="close" size={15}/></button>{/if}</label>
 <label class="table-sort">Sort<select aria-label="Sort recordings" value={sort} onchange={e=>{sort=e.currentTarget.value;page=0;}}>{#each columns.filter(c=>c.sortable) as c}<option value={c.key}>{c.label}</option>{/each}</select></label><button class="icon-button" aria-label={descending?'Sort ascending':'Sort descending'} onclick={()=>{descending=!descending;page=0;}}><Icon name="arrow" size={16}/></button><details class="column-picker"><summary><Icon name="columns" size={16}/>Columns</summary><div class="column-menu"><p class="help">Show, arrange, and resize columns.</p>{#each columns as c,i (c.key)}<div class="column-option"><label class="check"><input type="checkbox" bind:checked={c.visible} disabled={c.visible&&visible.length===1}>{c.label}</label><button class="icon-button" disabled={i===0} aria-label={`Move ${c.label} column left`} onclick={()=>columns=moveColumn(columns,c.key,-1)}><Icon name="left" size={14}/></button><button class="icon-button" disabled={i===columns.length-1} aria-label={`Move ${c.label} column right`} onclick={()=>columns=moveColumn(columns,c.key,1)}><Icon name="right" size={14}/></button></div>{/each}<button class="subtle-button" onclick={()=>columns=defaultColumns.map(c=>({...c}))}>Reset columns</button></div></details></div>
 {#if error}<div class="banner error" role="alert">{error}</div>{/if}
 <div class="table-scroll" aria-busy={loading}>
 <table class="recordings-table" class:many-columns={visible.length>6} ><colgroup>{#each visible as c}<col style:width={`${c.width/(visible.reduce((n,c)=>n+c.width,0)+172)*100}%`}>{/each}<col style:width={`${172/(visible.reduce((n,c)=>n+c.width,0)+172)*100}%`}></colgroup><thead><tr>{#each visible as c}<th scope="col" aria-sort={sort===c.key?(descending?'descending':'ascending'):'none'}>{#if c.sortable}<button class="sort-button" onclick={()=>sortBy(c.key)}>{c.label}{#if sort===c.key}<span class:ascending={!descending}><Icon name="arrow" size={14}/></span>{/if}</button>{:else}<span class="column-label">{c.label}</span>{/if}<button class="column-resizer" aria-label={`Resize ${c.label} column`} title="Drag to resize; arrow keys also work" onpointerdown={e=>resize(e,c.key)} onkeydown={e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();c.width=Math.max(100,Math.min(700,c.width+(e.key==='ArrowLeft'?-16:16)));}}}></button></th>{/each}<th scope="col" class="actions-column">Actions</th></tr></thead><tbody>
 {#each entries as e (e.id)}<tr>{#each visible as c}<td data-label={c.label}>
 {#if c.key==='created'}<div class="recording-title">{e.title}</div><p class="recording-meta">{duration(e.duration)}{#if wordCount(e.transcript)} · {wordCount(e.transcript)} words{/if}</p>{#if jobs(e)[0]&&jobs(e)[0].status!=='complete'}{@const job=jobs(e)[0]}<div class="row-status" class:error-text={['failed','unknown'].includes(job.status)}>{#if ['queued','running','canceling'].includes(job.status)}<Activity size={12}/>{/if}{status(job)}</div>{#if job.error&&['failed','unknown','paused'].includes(job.status)}<p class="stage-error">{job.error.message}</p>{/if}{#if job.status==='paused'}<button class="subtle-button" disabled={busy||snapshot.operation_busy} onclick={()=>run(()=>api.recover([job.id],false))}>Resume job</button>{/if}{/if}
 {:else if c.key==='transcript'||c.key==='transformed'}{@const kind=c.key}{@const text=e[kind]}<div class="table-text-cell"><button class="text-preview" disabled={!text} onclick={()=>inspect(e,kind)} title={text?'View full text':undefined}>{text?preview(text):'—'}</button><CopyButton available={!!text} label={`Copy ${kind==='transcript'?'transcript':'transformed output'} for ${e.title}`} copy={()=>run(()=>api.copy(e.id,kind),'Copied')}/></div>{#if kind==='transformed'&&e.transformed&&e.transformed_revision!==e.transcript_revision}<span class="badge stale">Out of date</span>{/if}
 {:else if c.key==='audio'}<AudioPlayer entry={e} {playback} disabled={snapshot.capture.active||busy} {run}/>
 {:else if c.key==='duration'}{duration(e.duration)}
 {:else if c.key==='bytes'}{(e.bytes/1024/1024).toFixed(2)} MB
 {:else if c.key==='source'}{e.source==='import'?'Imported':'Microphone'}
 {:else}<span class="plain-cell">{e[c.key as keyof Entry]??'—'}</span>{/if}
 </td>{/each}<td class="row-actions" data-label="Actions"><div class="row-action-buttons"><button class="icon-button" aria-label={`Retry transcription for ${e.title}`} title="Retry transcription (provider charge)" disabled={!e.audio||retryDisabled(e)} onclick={()=>run(()=>api.retry(e,'transcribe'))}><Icon name="retry" size={17}/></button><button class="icon-button" aria-label={`Retry transformation for ${e.title}`} title="Retry transformation (provider charge)" disabled={!e.transcript?.trim()||retryDisabled(e)} onclick={()=>run(()=>api.retry(e,'transform'))}><Icon name="spark" size={17}/></button><button class="icon-button" aria-label={`Edit ${e.title}`} title="Edit recording" disabled={busy} onclick={()=>edit(e)}><Icon name="edit" size={17}/></button><button class="icon-button" aria-label={`Download audio for ${e.title}`} title="Download original audio" disabled={!e.audio||busy} onclick={()=>run(()=>api.export(e.id),'Audio exported')}><Icon name="download" size={17}/></button><button class="icon-button danger" aria-label={`Delete ${e.title}`} title="Delete recording" disabled={busy||jobs(e).some(j=>['queued','running','canceling'].includes(j.status))||playback?.id===e.id} onclick={()=>remove(e)}><Icon name="trash" size={17}/></button></div></td></tr>
 {:else}<tr><td colspan={visible.length+1} class="table-empty"><Icon name={query?'search':'list'} size={30}/><h2>{query?'No recordings found':'Your recordings start here'}</h2><p>{query?'Try a different word or clear the search.':'Record a thought or import an audio file.'}</p></td></tr>{/each}
 </tbody></table></div>
 <Pagination bind:page bind:pageSize {total} {loading}/>
</main>
