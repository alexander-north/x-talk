<script lang="ts">
 import type {Entry,Playback} from '../types';
 import {api,duration} from '../ipc';
 import Icon from './Icon.svelte';
 let {entry,playback,disabled,run}: {entry:Entry;playback:Playback|null;disabled:boolean;run:(work:()=>Promise<unknown>,success?:string)=>Promise<boolean>}=$props();
 let current=$derived(playback?.id===entry.id?playback:null);
 let position=$derived(current?.position??0);
 async function toggle(){await run(()=>current?api.pause(entry.id,!current.paused):api.play(entry.id));}
</script>
<div class="audio-player" aria-label={`Audio for ${entry.title}`}>
 <button class="icon-button" disabled={!entry.audio||disabled} onclick={toggle} title={current&&!current.paused?'Pause audio':'Play audio'} aria-label={current&&!current.paused?'Pause audio':'Play audio'}><Icon name={current&&!current.paused?'pause':'play'} size={17}/></button>
 <div class="audio-track"><input aria-label={`Seek audio for ${entry.title}`} type="range" min="0" max={Math.max(entry.duration,0.1)} step="0.05" value={position} disabled={!entry.audio||disabled} onchange={e=>run(()=>api.play(entry.id,Number(e.currentTarget.value),current?.paused??false))}><span>{entry.audio?`${duration(position)} / ${duration(entry.duration)}`:'Audio expired'}</span></div>
</div>
