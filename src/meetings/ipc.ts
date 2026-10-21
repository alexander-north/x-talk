import {invoke} from '@tauri-apps/api/core';
import {writable} from 'svelte/store';
export interface CaptureStatus {id:string|null;active:boolean;finalizing:boolean;elapsed:number;microphone:boolean;system:boolean;microphone_muted:boolean;system_muted:boolean;microphone_level:number;system_level:number;error:string|null;warning:string|null}
export const captureStatus=writable<CaptureStatus|null>(null);
export interface MonitorStatus {active:boolean;microphone_level:number;system_level:number;error:string|null}
export interface CaptureOptions {microphone:boolean;system:boolean;microphone_device:string|null;output_device:string|null}
export type Stage='importing'|'extracting'|'converting'|'queued'|'preparing'|'waiting_for_keys'|'submitting'|'polling'|'briefing'|'complete'|'failed'|'submission_unknown'|'recovered';
export interface Summary {call_at:string;id:string;title:string;added_at:string;duration:number|null;status:Stage;preview:string|null;speakers:number;audio_available:boolean;revision:number}
export interface Evidence {text:string;segment_ids:string[]}
export interface BriefContent {title:string;summary:string;topics:Evidence[];decisions:Evidence[];actions:(Evidence&{owner:string|null;due:string|null})[];open_questions:Evidence[];suggested_follow_ups:string[]}
export interface Speaker {id:string;name:string;turn_count:number;word_count:number;estimated_turn_coverage_seconds:number}
export interface Turn {id:string;speaker_id:string;start:number;end:number;text:string;original_text:string;original_speaker_id:string;text_changed:boolean;speaker_changed:boolean;correction:{text:string|null;speaker:string|null};original_words:{text:string;start:number|null;end:number|null;confidence:number|null}[]}
export interface Meeting {id:string;title:string;added_at:string;call_at:string;duration_seconds:number|null;source:'audio'|'video'|'recording';filename:string|null;audio_available:boolean;capture_warning:string|null;revision:number;content_revision:number;transcription_status:Stage;transcript:{generation:string;provider:string;model:string|null;speakers:Speaker[];turns:Turn[]}|null;brief:{status:Stage;stale:boolean;content:BriefContent|null;error:string|null;provider:string;model:string;actual_model:string|null}}
export interface View {meeting:Meeting;error:string|null;merges:number;replacement:boolean}
export interface ImportPreview {token:string;filename:string;inspection:{duration:number;audio_streams:number;video:boolean;channels:number}}
export type Edit={kind:'metadata';title:string;call_at:string}|{kind:'correct';turn:string;correction:{text:string|null;speaker:string|null}|null;speaker_name?:string}|{kind:'name';speaker:string;name:string}|{kind:'merge';from:string;into:string}|{kind:'undo_merge'}|{kind:'retry';transcription:boolean};
export const meetings={
 play:(id:string,start=0,paused=false)=>invoke<void>('meeting_play',{id,start,paused}),
 devices:()=>invoke<{microphones:{id:string;name:string}[];outputs:{id:string;name:string}[]}>('meeting_devices'),
 monitor:(options:CaptureOptions|null)=>invoke<MonitorStatus>('meeting_monitor',{options}),
 captureStatus:()=>invoke<CaptureStatus>('meeting_capture_status'),
 start:(options:CaptureOptions)=>invoke<void>('meeting_start',{options}),
 stop:(discard=false)=>invoke<string|null>('meeting_stop',{discard}),
 mute:(microphone:boolean,muted:boolean)=>invoke<void>('meeting_mute',{microphone,muted}),
 list:()=>invoke<Summary[]>('meetings_list'),
 get:(id:string)=>invoke<View>('meeting_get',{id}),
 pick:()=>invoke<ImportPreview|null>('meeting_pick'),
 preview:(path:string)=>invoke<ImportPreview>('meeting_preview',{path}),
 import:(token:string,title?:string,callAt?:string)=>invoke<string>('meeting_import',{token,title,callAt}),
 edit:(id:string,revision:number,edit:Edit)=>invoke<View>('meeting_edit',{id,revision,edit}),
 copy:(id:string)=>invoke<void>('meeting_copy',{id}),
 download:(id:string)=>invoke<void>('meeting_download',{id}),
 delete:(id:string,revision:number)=>invoke<void>('meeting_delete',{id,revision}),
 retention:()=>invoke<number|null>('meeting_retention'),
 setRetention:(count:number|null)=>invoke<void>('meeting_set_retention',{count}),
};
export function stageLabel(stage:Stage){return ({importing:'Waiting to import',extracting:'Extracting audio',converting:'Converting audio',queued:'Queued',preparing:'Uploading audio',waiting_for_keys:'Waiting for API keys',submitting:'Starting transcription',polling:'Transcribing',briefing:'Writing brief',complete:'Ready',failed:'Needs attention',submission_unknown:'Submission outcome unknown',recovered:'Recovered recording'})[stage];}
