import {writable} from 'svelte/store';
import type {UpdateStatus} from './types';
export const updateStatus=writable<UpdateStatus>({phase:'',version:null,notes:null,error:null,downloaded:0,total:null});
