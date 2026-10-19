<script lang="ts">
 import {onMount,tick} from 'svelte';
 import {listen} from '@tauri-apps/api/event';
 import {api,message} from '../ipc';
 import {capturedShortcut} from '../shortcuts';
 import type {Settings} from '../types';
 type Action=keyof Settings['shortcuts'];
 let {value=$bindable(),saved,saving,save}:{value:Settings['shortcuts'];saved:Settings['shortcuts'];saving:boolean;save:()=>Promise<boolean>}=$props();
 let editing=$state<Action|null>(null);let manual=$state(false);let ready=$state(false);let error=$state('');let input=$state<HTMLInputElement>();
 let subscription:Promise<()=>void>;let disposed=false;let generation=0;
 onMount(()=>{
  subscription=listen<string>('shortcut-captured',e=>{if(editing&&ready&&!manual&&document.activeElement===input)value={...value,[editing]:e.payload};});
  return()=>{disposed=true;generation++;void subscription.then(off=>off());void api.shortcutEditing(false).catch(()=>undefined);};
 });
 async function arm(){
  const current=++generation;ready=false;
  try{await subscription;if(disposed||current!==generation)return;await api.shortcutEditing(true);if(!disposed&&current===generation)ready=true;}
  catch(e){error=message(e);}
 }
 async function open(action:Action){
  editing=action;manual=false;error='';await arm();await tick();input?.focus();
 }
 async function close(){
  generation++;ready=false;
  try{await api.shortcutEditing(false);editing=null;error='';}catch(e){error=message(e);}
 }
 async function cancel(){if(editing)value={...value,[editing]:saved[editing]};await close();}
 async function commit(){if(await save())await close();}
 async function mode(){manual=!manual;await tick();input?.focus();}
 function capture(e:KeyboardEvent){
  if(manual)return;
  // Keep Tab available for navigating to the manual-entry and cancel controls.
  if(e.code==='Tab'&&!e.ctrlKey&&!e.altKey&&!e.metaKey)return;
  e.preventDefault();e.stopPropagation();
  if(!ready||!editing)return;
  const key=capturedShortcut(e);if(key)value={...value,[editing]:key};
 }
 function blur(){generation++;ready=false;void api.shortcutEditing(false).catch(e=>error=message(e));}
</script>
<svelte:window onblur={blur} onfocus={()=>{if(editing)void arm();}}/>
<fieldset class="shortcut-editor"><legend>Global shortcuts</legend>
 {#each [['toggle','Toggle recording'],['cancel','Cancel recording']] as [action,label]}
  {@const name=action as Action}
  <div class="shortcut-row">
   <span class="shortcut-label">{label}</span>
   {#if editing===name}
    <label>{manual?'Enter shortcut manually':'Press a key combination…'}
     <input bind:this={input} aria-label={`${label} shortcut`} readonly={!manual} value={value[name]} disabled={!ready||saving} placeholder={manual?'Ctrl+Alt+Space':'Press a key combination…'} onkeydown={capture} oninput={e=>value={...value,[name]:e.currentTarget.value}} autocomplete="off" spellcheck="false">
    </label>
    <div class="actions"><button type="button" disabled={saving||!ready} onclick={mode}>{manual?'Capture keys':'Enter manually'}</button><button type="button" class="primary" disabled={saving||!ready||!value[name].trim()} onclick={commit}>Save</button><button type="button" disabled={saving} onclick={cancel}>Cancel</button>{#if saved[name]}<button type="button" disabled={saving||!ready} onclick={()=>{value={...value,[name]:''};void commit();}}>Remove shortcut</button>{/if}</div>
   {:else}
    <button type="button" disabled={saving||editing!==null} aria-label={`${value[name]?'Edit':'Add'} ${label.toLowerCase()} shortcut`} onclick={()=>open(name)}>{value[name]||'Add shortcut'}</button>
   {/if}
  </div>
 {/each}
 {#if error}<p class="error-text" role="alert">{error}</p>{/if}
 <p class="hint">Toggle starts or stops recording. Cancel discards the active recording.</p>
</fieldset>
