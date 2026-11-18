<script lang="ts">
 import {onMount} from 'svelte';
 import {getCurrentWebview} from '@tauri-apps/api/webview';
 import {meetings,type ImportPreview} from '../../meetings/ipc';
 import {duration,message} from '../../ipc';
 import Icon from '../Icon.svelte';
 let {close,created}:{close:()=>void;created:(id:string)=>void|Promise<void>}=$props();
 let dialog=$state<HTMLDialogElement>(),preview=$state<ImportPreview|null>(null),busy=$state(false),hover=$state(false),error=$state('');
 let title=$state(''),date=$state('');
 $effect(()=>{if(dialog&&!dialog.open)dialog.showModal();});
 async function choose(path?:string){if(busy)return;busy=true;error='';try{const next=path?await meetings.preview(path):await meetings.pick();if(next)preview=next;}catch(e){error=message(e);}finally{busy=false;}}
 async function submit(){if(!preview||busy)return;busy=true;error='';try{const id=await meetings.import(preview.token,title,date||undefined);await created(id);close();}catch(e){error=message(e);}finally{busy=false;}}
 onMount(()=>{const subscription=getCurrentWebview().onDragDropEvent(event=>{
  hover=event.payload.type==='enter'||event.payload.type==='over';
  if(event.payload.type==='drop'){
   if(event.payload.paths.length!==1){error='Choose one recording at a time.';return;}
   void choose(event.payload.paths[0]);
  }
 });return()=>{void subscription.then(unlisten=>unlisten());};});
</script>
<dialog class="modal import-dialog" bind:this={dialog} aria-labelledby="import-title" oncancel={e=>{if(busy)e.preventDefault();else close();}}>
 <form onsubmit={e=>{e.preventDefault();void submit();}}>
  <div class="dialog-heading"><div><p class="eyebrow">NEW RECORDING</p><h2 id="import-title">Bring the conversation.</h2></div><button type="button" class="subtle-button" aria-label="Close import" disabled={busy} onclick={close}>×</button></div>
  <button type="button" class="drop-zone" class:hover disabled={busy} onclick={()=>choose()}><Icon name="upload" size={28}/><strong>{busy&&!preview?'Checking recording…':preview?.filename??'Choose a recording or drop it here'}</strong><span>Audio or video · MP3, M4A, WAV, MP4, MOV, MKV, WebM and more</span></button>
  {#if preview}<div class="import-preview"><p>{duration(preview.inspection.duration)} · {preview.inspection.channels===2?'Stereo':'Mono'}</p>{#if preview.inspection.audio_streams>1}<p class="banner">This file contains {preview.inspection.audio_streams} audio tracks. Only the first audio track will be imported; the others will be omitted.</p>{/if}</div>{/if}
  <label>Meeting title (optional)<input aria-label="Import meeting title" bind:value={title} maxlength="500" placeholder="AI will name this after transcription"/></label>
  <label>Meeting date (optional)<input aria-label="Import recording date" type="date" bind:value={date}/></label>
  <p class="help">Video stays in its original location. Only audio is saved and sent to AssemblyAI; transcript text goes to OpenAI for the brief.</p>
  {#if error}<p class="banner error" role="alert">{error}</p>{/if}
  <button class="primary submit-import" disabled={busy||!preview}>{busy?'Adding recording…':'Create Transcript and Brief'}</button>
 </form>
</dialog>
<style>
 .import-dialog{max-width:560px;width:min(560px,calc(100vw - 40px));max-height:90vh;overflow:auto}.dialog-heading{display:flex;justify-content:space-between;align-items:center;gap:16px}.dialog-heading h2{margin:6px 0 20px}.dialog-heading button{font-size:26px}
 .drop-zone{white-space:normal;display:flex;flex-direction:column;align-items:center;gap:16px;padding:30px 22px;width:100%;border:1px dashed var(--muted);border-radius:16px;background:var(--surface);text-align:center}.drop-zone.hover{border-color:var(--accent);background:var(--accent-soft)}.drop-zone strong{overflow-wrap:anywhere}.drop-zone span{font-size:12px;color:var(--muted)}label{display:block;margin:20px 0}input{width:100%;margin-top:8px}.submit-import{width:100%;justify-content:center;margin-top:16px}
</style>
