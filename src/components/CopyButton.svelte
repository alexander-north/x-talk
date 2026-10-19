<script lang="ts">
 import Icon from './Icon.svelte';
 let {available, label, copy}: {available:boolean;label:string;copy:()=>Promise<boolean>}=$props();
 let copied=$state(false);
 async function click(){if(await copy()){copied=true;setTimeout(()=>copied=false,1500);}}
</script>
<button class="icon-button copy-button" aria-label={copied?'Copied':label} title={copied?'Copied':label} disabled={!available} onclick={click}><Icon name={copied?'check':'copy'} size={17}/></button>
