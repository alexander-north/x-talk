<script lang="ts">
 import {notification,dismissNotification} from '../notifications';
 let host=$state<HTMLDivElement>();
 $effect(()=>{host?.hidePopover();if($notification)host?.showPopover();});
 import Icon from './Icon.svelte';
</script>
<div bind:this={host} popover="manual" class="toast-region" aria-live="polite" aria-atomic="true">
 {#if $notification}<div class="toast" class:toast-error={$notification.error}>
  <Icon name={$notification.error?'warning':'check'} size={18}/><span>{$notification.text}</span>
  <button class="icon-button" aria-label="Dismiss notification" onclick={dismissNotification}><Icon name="close" size={16}/></button>
 </div>{/if}
</div>
