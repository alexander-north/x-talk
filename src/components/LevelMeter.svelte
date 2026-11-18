<script lang="ts">
 import {peakDb,levelLabel} from '../meetings/levels';
 let {level=0,label,muted=false,enabled=true}:{level?:number;label:string;muted?:boolean;enabled?:boolean}=$props();
 let value=$derived(enabled&&!muted?level:0);
</script>
<div class="level-meter" title={enabled?(muted?'Muted':`${levelLabel(value)} · peak level; 0 dBFS is full scale`):'Source disabled'}>
 <meter aria-label={label} min="-60" max="0" low="-12" high="-3" optimum="-20" value={peakDb(value)}></meter><span>{levelLabel(value)}</span>
</div>
<style>
 .level-meter{display:flex;align-items:center;gap:8px;min-width:128px}.level-meter meter{width:64px;height:12px}.level-meter span{font-size:10px;color:var(--muted);font-variant-numeric:tabular-nums;white-space:nowrap}
</style>
