<script lang="ts">
 import {onMount} from 'svelte';
 import Icon from './Icon.svelte';
 import type {OrbPhase} from '../recorder-state';
 import {paintOrb} from '../orb-art';
 let {phase,spectrum=[],timer='0:00',disabled=false,label,activate}:{phase:OrbPhase;spectrum?:number[];timer?:string;disabled?:boolean;label:string;activate:()=>void}=$props();
 let canvas:HTMLCanvasElement;
 onMount(()=>{
  const context=canvas.getContext('2d');if(!context)return;
  let frame=0,last=0,time=0,previous=phase,shape=phase==='transforming'?1:0;
  const smoothed=Array(48).fill(0);
  const motion=matchMedia('(prefers-reduced-motion: reduce)');
  const outgoing=document.createElement('canvas');
  let blend=1;
  function draw(now:number){
   const dt=Math.min((now-last)/1000||0,0.05);last=now;
   if(previous!==phase){
    outgoing.width=canvas.width;outgoing.height=canvas.height;
    outgoing.getContext('2d')?.drawImage(canvas,0,0);
    blend=motion.matches?1:0;time=0;previous=phase;
   }
   if(!document.hidden){
    time+=dt*(phase==='transcribing'||phase==='transforming'?1.5:1);
    const targetShape=phase==='transforming'?1:0;
    shape=motion.matches?targetShape:shape+(targetShape-shape)*(1-Math.exp(-dt*9));
    for(let i=0;i<48;i++){
     const target=phase==='recording'?(spectrum[i]??0):0;
     smoothed[i]+=(target-smoothed[i])*(1-Math.exp(-dt*(target>smoothed[i]?22:8)));
    }
    const size=canvas.clientWidth,dpr=Math.min(devicePixelRatio||1,2);
    if(canvas.width!==Math.round(size*dpr)){canvas.width=Math.round(size*dpr);canvas.height=canvas.width;}
    context!.setTransform(canvas.width/440,0,0,canvas.width/440,0,0);
    paintOrb(context!,phase,motion.matches?0:time,smoothed,motion.matches,shape);
    blend=Math.min(1,blend+dt/.55);
    if(blend<1&&outgoing.width){context!.save();context!.setTransform(1,0,0,1,0,0);context!.globalAlpha=1-blend*blend*(3-2*blend);context!.drawImage(outgoing,0,0,canvas.width,canvas.height);context!.restore();}
   }
   frame=requestAnimationFrame(draw);
  }
  frame=requestAnimationFrame(draw);
  return()=>cancelAnimationFrame(frame);
 });
</script>
<button class="record-orb glass-orb" class:recording={phase==='recording'} data-phase={phase} {disabled} aria-label={label} aria-pressed={phase==='recording'} onclick={activate}>
 <canvas bind:this={canvas} aria-hidden="true"></canvas>
 <span class="orb-glyphs" aria-hidden="true">
  <span class="glyph-layer" class:visible={phase==='idle'}><span class="orb-symbol"><Icon name="mic" size={54}/></span></span>
  <span class="glyph-layer" class:visible={phase==='recording'}><span class="orb-symbol"><span class="stop-glyph"><span class="record-timer">{timer}</span></span></span></span>
  <span class="glyph-layer" class:visible={phase==='complete'}><span class="orb-symbol reward"><Icon name="check" size={60}/></span></span>
  <span class="glyph-layer" class:visible={phase==='transforming'}><span class="orb-symbol starburst"><span class="star-art"><svg viewBox="0 0 48 48"><path d="M24 1C25 17 31 23 47 24C31 25 25 31 24 47C23 31 17 25 1 24C17 23 23 17 24 1Z" fill="currentColor"/></svg></span></span></span>
 </span>
</button>
