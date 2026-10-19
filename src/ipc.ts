import { invoke } from '@tauri-apps/api/core';
import type { AppState, Progress, Entry, Settings, Transformation, RecordingQuery, RecordingPage, Playback } from './types';
export const api = {
 updateStatus: () => invoke<import('./types').UpdateStatus>('update_status'),
 checkUpdate: () => invoke<import('./types').UpdateStatus>('check_update'),
 installUpdate: () => invoke<void>('install_update', {confirmed:true}),
 shortcutEditing: (editing:boolean) => invoke<void>('shortcut_editing', {editing}),
 clearRecordings: () => invoke<number>('clear_recordings', {confirmed:true}),
 recordings: (query: RecordingQuery) => invoke<RecordingPage>('recordings', {query}),
 editRecording: (e: Entry, title: string, subtitle: string, transcript: string) => invoke<void>('edit_recording', {id:e.id,revision:e.revision,title,subtitle,transcript}),
 playbackState: () => invoke<Playback|null>('playback_state'),
 pause: (id:string, paused:boolean) => invoke<void>('pause_playback', {id,paused}),
 windowMode: (mode:string) => invoke<void>('window_mode', {mode}),
 repository: () => invoke<void>('open_repository'),
 previewSound: (event:string) => invoke<void>('preview_sound', {event}),
 openLogs: () => invoke<void>('open_logs_folder'),
 progress: () => invoke<Progress>('progress'),
 state: () => invoke<AppState>('state'),
 history: (offset: number) => invoke<Entry[]>('history', { offset }),
 entry: (id: string) => invoke<Entry>('entry', { id }),
 capture: (action: string) => invoke<void>('capture_action', { action }),
 import: () => invoke<void>('import_audio'),
 retry: (e: Entry, kind: string) => invoke<void>('retry', { id: e.id, revision: e.revision, kind }),
 edit: (e: Entry, title: string, subtitle: string) => invoke<void>('edit_metadata', { id: e.id, revision: e.revision, title, subtitle }),
 delete: (e: Entry) => invoke<void>('delete_entry', { id: e.id, revision: e.revision, confirmed: true }),
 export: (id: string) => invoke<void>('export_audio', { id }),
 play: (id: string, startAt=0, paused=false) => invoke<void>('play_audio', { id, startAt, paused }),
 stopPlayback: () => invoke<void>('stop_playback'),
 copy: (id: string, kind: string) => invoke<void>('copy_text', { id, kind, text: null }),
 copyTest: (text: string) => invoke<void>('copy_text', { id: null, kind: null, text }),
 settings: (settings: Settings) => invoke<Settings>('save_settings', { settings }),
 transformation: (transformation: Transformation) => invoke<Transformation>('save_transformation', { transformation }),
 credential: (provider: string, action: string, key: string | null = null) => invoke<void>('credential', { provider, action, key }),
 test: (transformation: Transformation, input: string) => invoke<string>('test_transformation', { transformation, input }),
 recover: (ids: string[], discard: boolean) => invoke<void>('recover_jobs', { ids, discard }),
 quit: (discardCapture: boolean) => invoke<void>('quit', { discardCapture }),
};
export function message(error: unknown): string {
 if (typeof error === 'object' && error && 'message' in error) return String(error.message);
 return String(error);
}
export function duration(seconds: number): string { return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`; }

export function elapsedTime(milliseconds:number|null|undefined):string { return milliseconds==null||!Number.isFinite(milliseconds)||milliseconds<0?'':`${(milliseconds/1000).toFixed(1)} seconds`; }
