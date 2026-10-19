<script lang="ts">
 import {tick,onDestroy} from 'svelte';
 import {fade} from 'svelte/transition';
 import {prefersReducedMotion} from 'svelte/motion';
 import GlassOrb from './GlassOrb.svelte';
 import {activeJob,newCompletion,recorderPhase} from '../recorder-state';
 import type {AppState,Entry} from '../types';
 import {duration,elapsedTime} from '../ipc';
 import Icon from './Icon.svelte';
 import CopyButton from './CopyButton.svelte';
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
 let transcriptBox=$state<HTMLDivElement>();
 let transformedBox=$state<HTMLDivElement>();
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
   {#if transcriptRear}<button class="card-reveal" aria-label="Show transcript" onclick={()=>show('transcript')}></button>{/if}
   <div class="text-heading"><h2><Icon name="mic" size={15}/>{live?'Live preview':'Transcript'}{#if latest?.transcription_ms!=null}<span class="output-timing" title={latest?.transcript_model?.includes('scribe_v2_realtime')?'Time from Stop until the final transcript returned':'Transcription time, excluding queue and audio preparation'}>– {elapsedTime(latest.transcription_ms)}</span>{/if}</h2>{#if transcriptRear}<span class="card-hint">Click to view <Icon name="arrow" size={12}/></span>{:else}<CopyButton available={!!latest?.transcript} label="Copy transcript" copy={()=>copyResult(latest!.id,'transcript')}/>{/if}</div>
   <div bind:this={transcriptBox} class="output home-output" role="textbox" aria-label="Transcript output" aria-readonly="true" aria-multiline="true" tabindex={transcriptRear?-1:0} inert={transcriptRear} class:placeholder={!output}>{#if live}{output||'Listening… Your preview may change.'}{:else}{#key latest?.transcript}<span transition:fade={{duration:prefersReducedMotion.current?0:450}}>{latest?.transcript??'Your words will appear here after recording.'}</span>{/key}{/if}</div>
  </section>
  {#if live}<p class="help" role="status">{finalizing?'Confirming final text…':live.error?live.error.message:live.status==='connecting'?'Connecting to ElevenLabs… Audio is being recorded locally.':'Live preview · Text is finalized after Stop.'}</p>{/if}
  {#if stacked}
   <section transition:fade={{duration:prefersReducedMotion.current?0:550}} class="output-card transformed-card" class:card-front={front==='transformed'} class:card-rear={transformedRear} aria-label="Transformed output card">
    {#if transformedRear}<button class="card-reveal" aria-label="Show transformed output" onclick={()=>show('transformed')}></button>{/if}
    <div class="text-heading"><h2><Icon name="spark" size={15}/>Transformed{#if latest?.transformation_ms!=null}<span class="output-timing" title="Transformation processing time">– {elapsedTime(latest.transformation_ms)}</span>{/if}{#if latest?.transformed_revision!==latest?.transcript_revision}<span class="badge">Out of date</span>{/if}</h2>{#if transformedRear}<span class="card-hint">Click to view <Icon name="arrow" size={12}/></span>{:else}<CopyButton available={!!latest?.transformed} label="Copy transformed output" copy={()=>copyResult(latest!.id,'transformed')}/>{/if}</div>
    <div bind:this={transformedBox} class="output home-output" role="textbox" aria-label="Transformed output" aria-readonly="true" aria-multiline="true" tabindex={transformedRear?-1:0} inert={transformedRear}>{#key latest?.transformed}<span transition:fade={{duration:prefersReducedMotion.current?0:450}}>{latest?.transformed}</span>{/key}</div>
   </section>
  {/if}
 </div>{/if}
 <div class="recorder-footer">{#if !compact}<button class="import-footer" disabled={busy||snapshot.operation_busy||active} onclick={importAudio}><Icon name="upload" size={13}/>Import audio</button>{/if}<div class="shortcut-hints">{#if snapshot.settings.shortcuts.toggle}<span><kbd>{snapshot.settings.shortcuts.toggle}</kbd> record / stop</span>{/if}{#if snapshot.settings.shortcuts.cancel}<span><kbd>{snapshot.settings.shortcuts.cancel}</kbd> cancel</span>{/if}</div></div>
</main>
