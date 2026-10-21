<script lang="ts">
 import {onMount} from 'svelte';
 import {meetings,captureStatus} from '../../meetings/ipc';
 import {duration,message} from '../../ipc';
 import {notify} from '../../notifications';
 $effect(()=>{if(error)notify(error,true);});
 import Icon from '../Icon.svelte';
 import GlassOrb from '../GlassOrb.svelte';
 let {visible=false,compact=false,dictating=false,busy=$bindable(false)}:{visible?:boolean;compact?:boolean;dictating?:boolean;busy?:boolean}=$props();
 let error=$state(''),discard=$state(false);
 $effect(()=>{if(visible)void meetings.devices().then(value=>devices=value).catch(e=>notify(message(e),true));});
 let microphone=$state(true),system=$state(true),microphoneDevice=$state(''),outputDevice=$state('');
 let devices=$state<{microphones:{id:string;name:string}[];outputs:{id:string;name:string}[]}>({microphones:[],outputs:[]});
 let dialog=$state<HTMLDialogElement>();
 $effect(()=>{if(discard&&dialog&&!dialog.open)dialog.showModal();});
 async function read(){try{captureStatus.set(await meetings.captureStatus());}catch(e){error=message(e);}}
 async function run(work:()=>Promise<unknown>){busy=true;error='';try{await work();await read();return true;}catch(e){error=message(e);return false;}finally{busy=false;}}
 onMount(()=>{let pending=false;void read();const timer=setInterval(async()=>{if(pending)return;pending=true;await read();pending=false;},250);return()=>clearInterval(timer);});
</script>
{#if visible&&$captureStatus}
 <main class="recording-home meeting-recorder" class:compact aria-label="Meeting recorder">
 <section class="recording-focus" aria-label="Meeting recording controls"><div class="record-orbit">
 <GlassOrb variant="meeting" phase={$captureStatus.finalizing?'transcribing':$captureStatus.active?'recording':'idle'} timer={duration($captureStatus.elapsed)} disabled={busy||dictating||$captureStatus.finalizing||(!$captureStatus.id&&!microphone&&!system)} label={$captureStatus.active?'Stop meeting recording':'Start meeting recording'} activate={()=>run(()=>$captureStatus?.id?meetings.stop():meetings.start({microphone,system,microphone_device:microphoneDevice||null,output_device:outputDevice||null}))}/>
 {#if $captureStatus.id}<button class="cancel-orb" aria-label="Cancel meeting recording" disabled={busy||$captureStatus.finalizing} onclick={()=>discard=true}><Icon name="close" size={38}/></button>{/if}
 </div></section>
 {#if !compact}<section class="meeting-sources" aria-label="Meeting audio sources">
  <div class="source-row"><label class="check"><input type="checkbox" bind:checked={microphone} disabled={!!$captureStatus.id||busy}/>Microphone</label><select bind:value={microphoneDevice} disabled={!microphone||!!$captureStatus.id||busy} aria-label="Meeting microphone"><option value="">Windows default microphone</option>{#each devices.microphones as d}<option value={d.id}>{d.name}</option>{/each}</select>
  {#if $captureStatus.active&&$captureStatus.microphone}<button aria-pressed={$captureStatus.microphone_muted} disabled={busy} onclick={()=>run(()=>meetings.mute(true,!$captureStatus!.microphone_muted))}>{$captureStatus.microphone_muted?'Unmute microphone':'Mute microphone'}</button><meter aria-label="Microphone level" min="0" max="1" value={$captureStatus.microphone_level}></meter>{/if}</div>
  <div class="source-row"><label class="check"><input type="checkbox" bind:checked={system} disabled={!!$captureStatus.id||busy}/>System audio</label><select bind:value={outputDevice} disabled={!system||!!$captureStatus.id||busy} aria-label="Meeting system output"><option value="">Windows default output</option>{#each devices.outputs as d}<option value={d.id}>{d.name}</option>{/each}</select>
  {#if $captureStatus.active&&$captureStatus.system}<button aria-pressed={$captureStatus.system_muted} disabled={busy} onclick={()=>run(()=>meetings.mute(false,!$captureStatus!.system_muted))}>{$captureStatus.system_muted?'Unmute system audio':'Mute system audio'}</button><meter aria-label="System audio level" min="0" max="1" value={$captureStatus.system_level}></meter>{/if}</div>
  <p class="help">Choose the output your call uses. Microphone mute here is separate from your conferencing app.</p>
 </section>{/if}
 {#if $captureStatus.active&&$captureStatus.elapsed<15}<p class="responsibility" role="status">You are responsible for letting everyone know they're being recorded.</p>{/if}
 {#if $captureStatus.warning}<p class="banner" role="status">{$captureStatus.warning}</p>{/if}
 {#if $captureStatus.error}<p class="error-text" role="alert">{$captureStatus.error}</p>{/if}
 {#if compact&&$captureStatus.active}<div class="compact-sources">{#if $captureStatus.microphone}<button disabled={busy} aria-pressed={$captureStatus.microphone_muted} onclick={()=>run(()=>meetings.mute(true,!$captureStatus!.microphone_muted))}>{$captureStatus.microphone_muted?'Unmute mic':'Mute mic'}</button>{/if}{#if $captureStatus.system}<button disabled={busy} aria-pressed={$captureStatus.system_muted} onclick={()=>run(()=>meetings.mute(false,!$captureStatus!.system_muted))}>{$captureStatus.system_muted?'Unmute system':'Mute system'}</button>{/if}</div>{/if}
 <div class="recorder-footer"><span class="help">{$captureStatus.finalizing?'Saving meeting…':$captureStatus.active?'Click to stop and create transcript and brief':'Record a meeting · system audio and microphone'}</span></div>
 </main>
{:else if $captureStatus?.id}
 <section class="meeting-capture" aria-label="Meeting recording"><strong>{$captureStatus.finalizing?'Saving meeting…':'Recording meeting'}</strong><span>{duration($captureStatus.elapsed)}</span><button disabled={busy||$captureStatus.finalizing} onclick={()=>run(()=>meetings.stop())}>Stop &amp; save</button><button disabled={busy||$captureStatus.finalizing} onclick={()=>discard=true}>Discard</button>{#if $captureStatus.active&&$captureStatus.elapsed<15}<p class="responsibility" role="status">You are responsible for letting everyone know they're being recorded.</p>{/if}{#if $captureStatus.warning}<p class="banner">{$captureStatus.warning}</p>{/if}{#if $captureStatus.error}<p role="alert">{$captureStatus.error}</p>{/if}</section>
{/if}
{#if discard}<dialog class="modal" bind:this={dialog} aria-labelledby="discard-meeting-title" oncancel={()=>discard=false} onclose={()=>discard=false}><h2 id="discard-meeting-title">Discard this meeting recording?</h2><p>The captured audio will be deleted without transcription or a provider charge.</p><div class="actions"><button disabled={busy} onclick={()=>discard=false}>Keep recording</button><button class="danger" disabled={busy} onclick={async()=>{if(await run(()=>meetings.stop(true)))discard=false;}}>Discard recording</button></div></dialog>{/if}
<style>
 .compact .responsibility{font-size:10px;margin:0;max-width:260px}.compact .recorder-footer{display:none}.compact-sources{display:flex;gap:8px}.compact-sources button{padding:4px 8px;font-size:11px}.meeting-capture{display:flex;flex-wrap:wrap;align-items:center;gap:16px;margin:16px 32px;padding:12px 20px;border:1px solid var(--line);border-radius:16px;background:var(--surface)}
 .meeting-sources{width:min(680px,90%);margin:0 auto}.source-row{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin:12px 0}.source-row label{width:120px}.source-row select{flex:1;min-width:180px;max-width:100%}.source-row meter{width:80px}.responsibility{text-align:center;color:var(--muted);font-size:12px;margin:12px}.meeting-recorder{overflow:auto}.meeting-recorder .recording-focus{flex-shrink:0}
</style>
