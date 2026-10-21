<script lang="ts">
 import {onMount} from 'svelte';
 import {meetings,captureStatus} from '../../meetings/ipc';
 import {duration,message} from '../../ipc';
 import Icon from '../Icon.svelte';
 let {visible=false,compact=false,dictating=false}:{visible?:boolean;compact?:boolean;dictating?:boolean}=$props();
 let expanded=$state(true);
 let setup=$state(false),busy=$state(false),error=$state(''),discard=$state(false);
 let microphone=$state(true),system=$state(true),microphoneDevice=$state(''),outputDevice=$state('');
 let devices=$state<{microphones:{id:string;name:string}[];outputs:{id:string;name:string}[]}>({microphones:[],outputs:[]});
 let dialog=$state<HTMLDialogElement>();
 $effect(()=>{if(discard&&dialog&&!dialog.open)dialog.showModal();});
 async function read(){try{captureStatus.set(await meetings.captureStatus());}catch(e){error=message(e);}}
 async function run(work:()=>Promise<unknown>){busy=true;error='';try{await work();await read();return true;}catch(e){error=message(e);return false;}finally{busy=false;}}
 onMount(()=>{let pending=false;void read();const timer=setInterval(async()=>{if(pending)return;pending=true;await read();pending=false;},250);return()=>clearInterval(timer);});
</script>
{#if $captureStatus?.id}
 <section class="meeting-capture" class:compact aria-label="Meeting recording">
  <div class="capture-line"><span class="record-dot"></span><strong>{$captureStatus.active?'Recording meeting':busy?'Saving meeting…':'Meeting capture stopped'}</strong><span class="timer">{duration($captureStatus.elapsed)}</span><button class="primary" disabled={busy} onclick={()=>run(()=>meetings.stop())}><Icon name="stop" size={15}/>Stop &amp; save</button><button class="subtle-button danger" disabled={busy} onclick={()=>discard=true}>Discard</button></div>
  <button class="subtle-button" aria-expanded={expanded} onclick={()=>expanded=!expanded}>{expanded?'Collapse recording controls':'Expand recording controls'}</button>
  {#if expanded}<div class="capture-line">
   {#if $captureStatus.microphone}<button aria-pressed={$captureStatus.microphone_muted} disabled={busy} onclick={()=>run(()=>meetings.mute(true,!$captureStatus!.microphone_muted))}><Icon name="mic" size={15}/>{$captureStatus.microphone_muted?'Unmute microphone':'Mute microphone'}</button><meter aria-label="Microphone level" min="0" max="1" value={$captureStatus.microphone_level}></meter>{/if}
   {#if $captureStatus.system}<button aria-pressed={$captureStatus.system_muted} disabled={busy} onclick={()=>run(()=>meetings.mute(false,!$captureStatus!.system_muted))}>{$captureStatus.system_muted?'Unmute system audio':'Mute system audio'}</button><meter aria-label="System audio level" min="0" max="1" value={$captureStatus.system_level}></meter>{/if}
  </div>
  {/if}
  {#if $captureStatus.active&&$captureStatus.elapsed<15}<p class="responsibility" role="status">You are responsible for letting everyone know they're being recorded.</p>{/if}
  {#if $captureStatus.warning}<p class="banner" role="status">{$captureStatus.warning}</p>{/if}
  {#if $captureStatus.error}<p class="error-text" role="alert">{$captureStatus.error}</p>{/if}
  {#if error}<p class="error-text" role="alert">{error}</p>{/if}
 </section>
{:else if visible}
 <section class="meeting-capture">
  <div class="capture-line"><button class="primary" disabled={busy||dictating} aria-expanded={setup} onclick={()=>run(async()=>{if(!setup)devices=await meetings.devices();setup=!setup;})}><Icon name="mic" size={16}/>{setup?'Close recording setup':'Record a meeting'}</button><span class="muted">System audio + your microphone</span></div>
  {#if setup}<div class="setup-grid">
   <label class="check"><input type="checkbox" bind:checked={microphone}/>Microphone</label><select bind:value={microphoneDevice} disabled={!microphone} aria-label="Meeting microphone"><option value="">Windows default microphone</option>{#each devices.microphones as d}<option value={d.id}>{d.name}</option>{/each}</select>
   <label class="check"><input type="checkbox" bind:checked={system}/>System audio</label><select bind:value={outputDevice} disabled={!system} aria-label="Meeting system output"><option value="">Windows default output</option>{#each devices.outputs as d}<option value={d.id}>{d.name}</option>{/each}</select>
   <p class="help">Choose the output your call uses. xTalk's microphone mute is separate from your conferencing app.</p>
   <button class="primary" disabled={busy||dictating||!microphone&&!system} onclick={()=>run(async()=>{await meetings.start({microphone,system,microphone_device:microphoneDevice||null,output_device:outputDevice||null});setup=false;})}>{busy?'Starting…':'Start meeting'}</button>
  </div>{/if}
  {#if error}<p class="error-text" role="alert">{error}</p>{/if}
 </section>
{/if}
{#if discard}<dialog class="modal" bind:this={dialog} aria-labelledby="discard-meeting-title" oncancel={()=>discard=false} onclose={()=>discard=false}><h2 id="discard-meeting-title">Discard this meeting recording?</h2><p>The captured audio will be deleted without transcription or a provider charge.</p><div class="actions"><button disabled={busy} onclick={()=>discard=false}>Keep recording</button><button class="danger" disabled={busy} onclick={async()=>{if(await run(()=>meetings.stop(true)))discard=false;}}>Discard recording</button></div></dialog>{/if}
<style>
 .meeting-capture{margin:16px 32px 0;padding:16px 20px;border:1px solid var(--line);border-radius:16px;background:var(--surface)}
 .capture-line{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.capture-line{margin-top:8px}
 .timer{font-variant-numeric:tabular-nums;margin-right:auto}.record-dot{width:9px;height:9px;background:var(--accent);border-radius:50%}
 .responsibility{margin-top:12px;color:var(--muted);font-size:12px}meter{width:65px;height:10px}
 .setup-grid{display:grid;grid-template-columns:120px 1fr;gap:12px;margin-top:20px}.setup-grid p{grid-column:1/-1}
 .compact{margin:14px;padding:14px}.compact .capture-line{gap:7px}.compact meter{display:none}
 .error-text{margin-top:12px}
</style>
