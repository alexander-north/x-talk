<script lang="ts">
 import Icon from './Icon.svelte';
 let {page=$bindable(0),pageSize=$bindable(50),total,loading=false,noun='recordings'}:{page?:number;pageSize?:number;total:number;loading?:boolean;noun?:string}=$props();
</script>
<div class="pagination"><span>{total?`${page*pageSize+1}–${Math.min((page+1)*pageSize,total)} of ${total}`:`0 ${noun}`}{#if loading} · Updating…{/if}</span><div class="actions"><label class="page-size">Per page<select aria-label={`${noun[0].toUpperCase()+noun.slice(1)} per page`} bind:value={pageSize} onchange={()=>page=0}>{#each [25,50,100] as size}<option value={size}>{size}</option>{/each}</select></label><button class="icon-button" disabled={page===0||loading} aria-label={`Previous ${noun} page`} onclick={()=>page--}><Icon name="left" size={17}/></button><button class="icon-button" disabled={(page+1)*pageSize>=total||loading} aria-label={`Next ${noun} page`} onclick={()=>page++}><Icon name="right" size={17}/></button></div></div>
