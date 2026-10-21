<script lang="ts">
 import {tick,onDestroy} from 'svelte';
 import {fade} from 'svelte/transition';
 import {prefersReducedMotion} from 'svelte/motion';
 import GlassOrb from './GlassOrb.svelte';
 import {activeJob,newCompletion,recorderPhase} from '../recorder-state';
 import type {AppState,Entry} from '../types';
 import {duration,elapsedTime} from '../ipc';
 import Icon from './Icon.svelte';
 import {tailText} from '../tail-text';
 let {snapshot,entry,elapsed,captureBusy,busy,compact,spectrum=[],control,copy,importAudio}: {snapshot:AppState;entry:Entry|null;elapsed:number;captureBusy:boolean;busy:boolean;compact:boolean;spectrum?:number[];control:(action:string)=>Promise<void>;copy:(id:string,kind:string)=>Promise<boolean>;importAudio:()=>void}=$props();
 let active=$derived(snapshot.capture.active);
 let finalizing=$derived(!!snapshot.capture.finalizing);
 let live=$derived(active?snapshot.capture.live:null);
 let job=$derived(activeJob(snapshot.jobs));
 let processing=$derived(snapshot.operation_busy&&!active);
 let celebrating=$state(false);
 let completionTimer:ReturnType<typeof setTimeout>;
 let previousJobs=new Map<string,string>();
 $effect(()=>{
  const completed=newCompletion(previousJobs,snapshot.jobs);
  previousJobs=new Map(snapshot.jobs.map(j=>[j.id,j.status]));
  if(active||processing){celebrating=false;clearTimeout(completionTimer);}
  else if(completed){celebrating=true;clearTimeout(completionTimer);completionTimer=setTimeout(()=>celebrating=false,5000);}
 });
 onDestroy(()=>clearTimeout(completionTimer));
 let phase=$derived(finalizing?'transcribing':recorderPhase(active,snapshot.jobs,snapshot.operation_stage??(processing?'queued':null),celebrating));
 let status=$derived(finalizing?'Finalizing transcript…':active?duration(elapsed):phase==='complete'?'Ready to use':phase==='canceling'?'Canceling…':processing?({preparing:'Preparing audio…',importing:'Importing audio…',transcribing:'Transcribing…',transforming:'Transforming…',queued:'Preparing…'} as Record<string,string>)[job?.stage??snapshot.operation_stage??'queued']??'Processing…':'Click to record');
 async function copyResult(id:string,kind:string){const copied=await copy(id,kind);if(copied){celebrating=false;clearTimeout(completionTimer);}return copied;}

 let hiddenEntry=$state<string|null>(null);
 $effect(()=>{if(active)hiddenEntry=entry?.id??null;else if(!processing)hiddenEntry=null;});
 let latest=$derived(active||entry?.id===hiddenEntry?null:entry);
 let output=$derived(live?.text||latest?.transcript||'');
 let stacked=$derived(snapshot.settings.transform&&!!latest?.transformed);
 let front=$state<'transcript'|'transformed'>('transcript');
 let transcriptRear=$derived(stacked&&front!=='transcript');
 let transformedRear=$derived(front!=='transformed');
 let transcriptBox=$state<HTMLButtonElement>();
 let transformedBox=$state<HTMLButtonElement>();
 let copied=$state<string|null>(null);
 let copyTimer:ReturnType<typeof setTimeout>;
 onDestroy(()=>clearTimeout(copyTimer));
 async function copyCard(kind:'transcript'|'transformed'){
  const selected=latest;
  if(!selected)return;
  if(await copyResult(selected.id,kind)){
   copied=`${selected.id}:${kind}`;
   clearTimeout(copyTimer);copyTimer=setTimeout(()=>copied=null,1500);
  }
 }
 let previousEntry:string|null=null;
 let previousTransformation:string|null=null;
 $effect(()=>{
  const id=latest?.id??null;
  const result=stacked?`${latest?.transformed_revision}\0${latest?.transformed}`:null;
  // A refreshed state object must not undo a manual swap. Only a new result takes the front.
  if(id!==previousEntry||result!==previousTransformation){
   front=result===null?'transcript':'transformed';
   previousEntry=id;previousTransformation=result;
  }
 });
 async function show(kind:'transcript'|'transformed'){
  front=kind;
  await tick();
  (kind==='transcript'?transcriptBox:transformedBox)?.focus({preventScroll:true});
 }
</script>
<main class="recording-home" aria-label="Recorder">
 <section class="recording-focus" aria-label="Recording controls">
  <div class="record-orbit">
   <GlassOrb {phase} timer={duration(elapsed)} {spectrum} disabled={captureBusy||finalizing||processing||busy&&!active} label={active?'Stop recording':processing?status:'Start recording'} activate={()=>control('toggle')}/>
   {#if active||processing}
   <button transition:fade={{duration:prefersReducedMotion.current?0:350}} class="cancel-orb" disabled={captureBusy||finalizing||phase==='canceling'} onclick={()=>control('cancel')} aria-label={active?'Cancel recording':'Cancel processing'} title={active?'Cancel recording':'Cancel processing; keep saved audio and text'}><Icon name="close" size={38}/></button>
   {/if}
  </div>
  <span class="visually-hidden" role="status" aria-live={active?'off':'polite'}>{status}</span>
 </section>
 {#if !compact}<div class="home-outputs" class:stacked aria-label="Recording output cards">
  <section class="output-card transcript-card" class:card-front={stacked&&front==='transcript'} class:card-rear={transcriptRear} aria-label="Transcript card">
   <button bind:this={transcriptBox} class="card-reveal card-action" disabled={!transcriptRear&&!latest?.transcript} aria-label={transcriptRear?'Show transcript':'Copy transcript'} onclick={()=>transcriptRear?show('transcript'):copyCard('transcript')}></button>
   <div class="text-heading"><h2><Icon name="mic" size={15}/>{live?'Live preview':'Transcript'}{#if latest?.transcription_ms!=null}<span class="output-timing" title={latest?.transcript_model?.includes('scribe_v2_realtime')?'Time from Stop until the final transcript returned':'Transcription time, excluding queue and audio preparation'}>– {elapsedTime(latest.transcription_ms)}</span>{/if}</h2>{#if transcriptRear}<span class="card-hint">Click to view <Icon name="arrow" size={12}/></span>{:else}<span class="card-copy-hint" aria-hidden="true"><Icon name={copied===`${latest?.id}:transcript`?'check':'copy'} size={17}/></span>{/if}</div>
   <div class="output home-output" aria-label="Transcript output" inert={transcriptRear} class:placeholder={!output} use:tailText={live?(output||'Listening… Your preview may change.'):(latest?.transcript??'Your words will appear here after recording.')}></div>
  </section>
  {#if live}<p class="help live-preview-note" role="status">{finalizing?'Confirming final text…':live.error?live.error.message:live.status==='connecting'?'Connecting to ElevenLabs… Audio is being recorded locally.':'Live preview · Text is finalized after Stop.'}</p>{/if}
  {#if stacked}
   <section transition:fade={{duration:prefersReducedMotion.current?0:550}} class="output-card transformed-card" class:card-front={front==='transformed'} class:card-rear={transformedRear} aria-label="Transformed output card">
    <button bind:this={transformedBox} class="card-reveal card-action" aria-label={transformedRear?'Show transformed output':'Copy transformed output'} onclick={()=>transformedRear?show('transformed'):copyCard('transformed')}></button>
    <div class="text-heading"><h2><Icon name="spark" size={15}/>Transformed{#if latest?.transformation_ms!=null}<span class="output-timing" title="Transformation processing time">– {elapsedTime(latest.transformation_ms)}</span>{/if}{#if latest?.transformed_revision!==latest?.transcript_revision}<span class="badge">Out of date</span>{/if}</h2>{#if transformedRear}<span class="card-hint">Click to view <Icon name="arrow" size={12}/></span>{:else}<span class="card-copy-hint" aria-hidden="true"><Icon name={copied===`${latest?.id}:transformed`?'check':'copy'} size={17}/></span>{/if}</div>
    <div class="output home-output" aria-label="Transformed output" inert={transformedRear} use:tailText={latest?.transformed??''}></div>
   </section>
  {/if}
  <span class="visually-hidden" role="status">{copied?'Copied':''}</span>
 </div>{/if}
 <div class="recorder-footer">{#if !compact}<button class="import-footer" disabled={busy||snapshot.operation_busy||active} onclick={importAudio}><Icon name="upload" size={13}/>Import audio</button>{/if}<div class="shortcut-hints">{#if snapshot.settings.shortcuts.toggle}<span><kbd>{snapshot.settings.shortcuts.toggle}</kbd> record / stop</span>{/if}{#if snapshot.settings.shortcuts.cancel}<span><kbd>{snapshot.settings.shortcuts.cancel}</kbd> cancel</span>{/if}</div></div>
</main>
