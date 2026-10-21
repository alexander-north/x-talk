import {writable} from 'svelte/store';
export const notification=writable<{text:string;error:boolean}|null>(null);
let timer:ReturnType<typeof setTimeout>|undefined;
export function dismissNotification(){clearTimeout(timer);notification.set(null);}
export function notify(text:string,error=false){
 if(!text)return;
 clearTimeout(timer);notification.set({text,error});
 timer=setTimeout(dismissNotification,error?7000:4000);
}
