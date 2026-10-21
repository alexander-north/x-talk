<script lang="ts">
 import Icon from './Icon.svelte';
 let {size=16}:{size?:number}=$props();
</script>
<span class="activity" aria-hidden="true"><Icon name="spark" {size}/></span>
