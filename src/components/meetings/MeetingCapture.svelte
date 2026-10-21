<script lang="ts">
 import {onMount} from 'svelte';
 import {meetings,captureStatus,type MonitorStatus} from '../../meetings/ipc';
 import {duration,message} from '../../ipc';
 import {notify} from '../../notifications';
 $effect(()=>{if(error)notify(error,true);});
 import Icon from '../Icon.svelte';
 import LevelMeter from '../LevelMeter.svelte';
 import GlassOrb from '../GlassOrb.svelte';
 let {visible=false,compact=false,dictating=false,openMeeting,busy=$bindable(false)}:{visible?:boolean;compact?:boolean;dictating?:boolean;openMeeting:(id:string)=>void;busy?:boolean}=$props();
 let error=$state(''),discard=$state(false),completed=$state<string|null>(null);
 async function stop(discard=false){const id=await meetings.stop(discard);if(!discard&&id)completed=id;}
 async function toggle(){if(completed){completed=null;return;}if($captureStatus?.id)await stop();else await meetings.start({microphone,system,microphone_device:microphoneDevice||null,output_device:outputDevice||null});}
 let monitor=$state<MonitorStatus|null>(null);
 let monitorRequested=false;
 let windowVisible=$state(true);
 $effect(()=>{if(visible)void meetings.devices().then(value=>devices=value).catch(e=>notify(message(e),true));});
 let microphone=$state(true),system=$state(true),microphoneDevice=$state(''),outputDevice=$state('');
 let devices=$state<{microphones:{id:string;name:string}[];outputs:{id:string;name:string}[]}>({microphones:[],outputs:[]});
 let dialog=$state<HTMLDialogElement>();
 $effect(()=>{if(discard&&dialog&&!dialog.open)dialog.showModal();});
 async function read(){try{captureStatus.set(await meetings.captureStatus());}catch(e){error=message(e);}}
 async function preview(){
  const wanted=visible&&windowVisible&&!compact&&!dictating&&!busy&&!completed&&!$captureStatus?.id&&(microphone||system);
  if(wanted){monitorRequested=true;try{monitor=await meetings.monitor({microphone,system,microphone_device:microphoneDevice||null,output_device:outputDevice||null});}catch(e){monitor={active:false,microphone_level:0,system_level:0,spectrum:[],error:message(e)};}}
  else if(monitorRequested){monitorRequested=false;monitor=null;try{await meetings.monitor(null);}catch{/* Device lease also expires without polling. */}}
 }

 async function changeDevice(isMicrophone:boolean,event:Event){
  const select=event.currentTarget as HTMLSelectElement;
  const next=select.value,previous=isMicrophone?microphoneDevice:outputDevice;
  if(!$captureStatus?.active||await run(()=>meetings.changeDevice(isMicrophone,next||null))){if(isMicrophone)microphoneDevice=next;else outputDevice=next;}
  else select.value=previous;
 }
 async function run(work:()=>Promise<unknown>){busy=true;error='';try{await work();await read();return true;}catch(e){error=message(e);return false;}finally{busy=false;}}
 onMount(()=>{
  let pending=false,disposed=false;
  const visibility=()=>{windowVisible=!document.hidden;};visibility();document.addEventListener('visibilitychange',visibility);
  const poll=async()=>{if(pending||disposed)return;pending=true;try{await read();if(!disposed)await preview();}finally{pending=false;}};
  void poll();const timer=setInterval(poll,50);
  return()=>{disposed=true;clearInterval(timer);document.removeEventListener('visibilitychange',visibility);void meetings.monitor(null).catch(()=>{});};
 });
</script>
{#if visible&&$captureStatus}
 <main class="recording-home meeting-recorder" class:compact aria-label="Meeting recorder">
 <section class="recording-focus" class:recording={$captureStatus.active} aria-label="Meeting recording controls"><div class="record-orbit">
 <GlassOrb variant="meeting" phase={completed?'complete':$captureStatus.finalizing?'transcribing':$captureStatus.active?'recording':'idle'} timer={duration($captureStatus.elapsed)} disabled={busy||dictating||$captureStatus.finalizing||(!$captureStatus.id&&!microphone&&!system)} label={completed?'New recording':$captureStatus.active?'Stop meeting recording':'Start meeting recording'} previewActivity={!!monitor?.active&&!completed} spectrum={$captureStatus.id?$captureStatus.spectrum:monitor?.spectrum??[]} activate={()=>run(toggle)}/>
 {#if $captureStatus.id}<button class="cancel-orb" aria-label="Cancel meeting recording" disabled={busy||$captureStatus.finalizing} onclick={()=>discard=true}><Icon name="close" size={38}/></button>{/if}
 {#if $captureStatus.active&&$captureStatus.elapsed<15}<p class="responsibility" role="status">You are responsible for letting everyone know they're being recorded.</p>{/if}
 </div></section>
 {#if completed}<div class="meeting-complete-actions"><button onclick={()=>completed=null}>New recording</button><button onclick={()=>openMeeting(completed!)}>Open meeting</button></div>{:else if !compact}<section class="meeting-sources" aria-label="Meeting audio sources">
  <div class="source-row"><label class="check"><input type="checkbox" bind:checked={microphone} disabled={!!$captureStatus.id||busy}/>Microphone</label><select value={microphoneDevice} onchange={event=>changeDevice(true,event)} disabled={!microphone||$captureStatus.finalizing||busy} aria-label="Meeting microphone"><option value="">Windows default microphone</option>{#each devices.microphones as d}<option value={d.id}>{d.name}</option>{/each}</select>
  {#if $captureStatus.active&&$captureStatus.microphone}<button aria-pressed={$captureStatus.microphone_muted} disabled={busy} onclick={()=>run(()=>meetings.mute(true,!$captureStatus!.microphone_muted))}>{$captureStatus.microphone_muted?'Unmute microphone':'Mute microphone'}</button>{/if}<LevelMeter label="Microphone level" enabled={microphone} muted={$captureStatus.microphone_muted} level={$captureStatus.id?$captureStatus.microphone_level:monitor?.microphone_level??0}/></div>
  <div class="source-row"><label class="check"><input type="checkbox" bind:checked={system} disabled={!!$captureStatus.id||busy}/>System audio</label><select value={outputDevice} onchange={event=>changeDevice(false,event)} disabled={!system||$captureStatus.finalizing||busy} aria-label="Meeting system output"><option value="">Windows default output</option>{#each devices.outputs as d}<option value={d.id}>{d.name}</option>{/each}</select>
  {#if $captureStatus.active&&$captureStatus.system}<button aria-pressed={$captureStatus.system_muted} disabled={busy} onclick={()=>run(()=>meetings.mute(false,!$captureStatus!.system_muted))}>{$captureStatus.system_muted?'Unmute system audio':'Mute system audio'}</button>{/if}<LevelMeter label="System audio level" enabled={system} muted={$captureStatus.system_muted} level={$captureStatus.id?$captureStatus.system_level:monitor?.system_level??0}/></div>
  {#if monitor?.error}<p class="error-text" role="status">{monitor.error}</p>{/if}
 </section>{/if}
 {#if $captureStatus.warning}<p class="banner" role="status">{$captureStatus.warning}</p>{/if}
 {#if $captureStatus.error}<p class="error-text" role="alert">{$captureStatus.error}</p>{/if}
 {#if compact&&$captureStatus.active}<div class="compact-sources">{#if $captureStatus.microphone}<button disabled={busy} aria-pressed={$captureStatus.microphone_muted} onclick={()=>run(()=>meetings.mute(true,!$captureStatus!.microphone_muted))}>{$captureStatus.microphone_muted?'Unmute mic':'Mute mic'}</button>{/if}{#if $captureStatus.system}<button disabled={busy} aria-pressed={$captureStatus.system_muted} onclick={()=>run(()=>meetings.mute(false,!$captureStatus!.system_muted))}>{$captureStatus.system_muted?'Unmute system':'Mute system'}</button>{/if}</div>{/if}

 </main>
{:else if $captureStatus?.id}
 <section class="meeting-capture" aria-label="Meeting recording"><strong>{$captureStatus.finalizing?'Saving meeting…':'Recording meeting'}</strong><span>{duration($captureStatus.elapsed)}</span><button disabled={busy||$captureStatus.finalizing} onclick={()=>run(()=>stop())}>Stop &amp; save</button><button disabled={busy||$captureStatus.finalizing} onclick={()=>discard=true}>Discard</button>{#if $captureStatus.active&&$captureStatus.elapsed<15}<p class="responsibility" role="status">You are responsible for letting everyone know they're being recorded.</p>{/if}{#if $captureStatus.warning}<p class="banner">{$captureStatus.warning}</p>{/if}{#if $captureStatus.error}<p role="alert">{$captureStatus.error}</p>{/if}</section>
{/if}
{#if discard}<dialog class="modal" bind:this={dialog} aria-labelledby="discard-meeting-title" oncancel={()=>discard=false} onclose={()=>discard=false}><h2 id="discard-meeting-title">Discard this meeting recording?</h2><p>The captured audio will be deleted without transcription or a provider charge.</p><div class="actions"><button disabled={busy} onclick={()=>discard=false}>Keep recording</button><button class="danger" disabled={busy} onclick={async()=>{if(await run(()=>stop(true)))discard=false;}}>Discard recording</button></div></dialog>{/if}
<style>
 .compact .responsibility{font-size:10px;margin:0;max-width:260px}.compact-sources{display:flex;gap:8px}.compact-sources button{padding:4px 8px;font-size:11px}.meeting-capture{display:flex;flex-wrap:wrap;align-items:center;gap:16px;margin:16px 32px;padding:12px 20px;border:1px solid var(--line);border-radius:16px;background:var(--surface)}
 .meeting-sources{width:min(800px,100%);margin:0 auto}.source-row{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin:12px 0}.source-row label{width:clamp(65px,10cqw,105px)}.source-row select{flex:1;min-width:100px;max-width:100%}.responsibility{text-align:center;color:var(--muted);font-size:12px;margin:12px}.meeting-sources{flex-shrink:0}.source-row{margin:6px 0}.responsibility{margin:4px 0}.source-row button{order:4}.source-row select{padding-top:6px;padding-bottom:6px}
 .recording-focus .responsibility{position:absolute;top:calc(100% + var(--orb-size)*.43 + 12px);left:50%;transform:translateX(-50%);width:min(400px,90cqw);font-size:11px;line-height:1.3;margin:0}.compact .recording-focus .responsibility{font-size:10px;width:min(260px,90cqw)}.meeting-complete-actions{display:flex;justify-content:center;gap:20px;width:100%;padding:24px 0}
</style>
