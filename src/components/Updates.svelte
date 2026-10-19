<script lang="ts">
 import {api,message} from '../ipc';
 import {updateStatus} from '../updates';
 import type {AppState} from '../types';
 import type {SettingsChange} from '../settings';
 let {snapshot,guard,change}:{snapshot:AppState;guard:(action:()=>void|Promise<void>)=>void;change:(update:SettingsChange)=>Promise<void>}=$props();
 let confirm=$state(false);let dialog=$state<HTMLDialogElement>();let error=$state('');
 let working=$derived(['checking','downloading','installing'].includes($updateStatus.phase));
 let occupied=$derived(snapshot.capture.active||snapshot.jobs.some(j=>['queued','running'].includes(j.status)));
 $effect(()=>{if(confirm&&dialog&&!dialog.open)dialog.showModal();});
 async function check(){error='';try{updateStatus.set(await api.checkUpdate());}catch(e){error=message(e);}}
 async function install(){confirm=false;error='';try{await api.installUpdate();}catch(e){error=message(e);}}
</script>
<section aria-label="Application updates">
 <h2>Updates</h2>
 <p class="help">Installed version {snapshot.diagnostics.version}. Updates preserve your recordings and settings.</p>
 <label class="check"><input type="checkbox" checked={snapshot.settings.check_updates} onchange={e=>{const value=e.currentTarget.checked;void change(s=>({...s,check_updates:value}));}}>Automatically check for updates</label>
 <p class="help">Checks GitHub after startup and every 12 hours. Download and restart only when you choose.</p>
 <div class="actions"><button disabled={working} onclick={check}>{$updateStatus.phase==='checking'?'Checking…':'Check for updates'}</button>
 {#if $updateStatus.phase==='available'}<button class="primary" disabled={occupied} onclick={()=>guard(()=>{confirm=true;})}>Update and restart</button>{/if}</div>
 {#if occupied&&$updateStatus.phase==='available'}<p class="help">Finish recording and processing before updating.</p>{/if}
 <div role="status" aria-live="polite">
 {#if $updateStatus.phase==='current'}<p>You’re up to date.</p>
 {:else if $updateStatus.phase==='available'}<p>Version {$updateStatus.version} is ready.</p>{#if $updateStatus.notes}<details><summary>What’s new</summary><p class="update-notes">{$updateStatus.notes}</p></details>{/if}
 {:else if $updateStatus.phase==='downloading'}<p>Downloading update… {$updateStatus.total?Math.min(100,Math.round($updateStatus.downloaded/$updateStatus.total*100))+'%':Math.round($updateStatus.downloaded/1024)+' KB'}</p>
 {:else if $updateStatus.phase==='installing'}<p>Starting the installer… x-talk will restart.</p>{/if}
 </div>
 {#if error||$updateStatus.error}<p class="error-text" role="alert">{error||$updateStatus.error}</p>{/if}
</section>
{#if confirm}<dialog bind:this={dialog} class="modal" aria-labelledby="update-title" oncancel={()=>confirm=false} onclose={()=>confirm=false}>
 <h2 id="update-title">Update and restart x-talk?</h2><p>Download version {$updateStatus.version} and restart to install it. Recording will be unavailable during the update. Your recordings, settings, and API keys will be kept.</p>
 <div class="actions dialog-actions"><button onclick={()=>confirm=false}>Not now</button><button class="primary" onclick={install}>Update and restart</button></div>
</dialog>{/if}
<style>.update-notes{white-space:pre-wrap;overflow-wrap:anywhere;max-height:16rem;overflow:auto}</style>
