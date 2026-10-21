export interface Failure { stage: string; category: string; message: string; recovery: string }
export interface Transcription { provider: string; model: string; prompt: string; temperature: number | null }
export interface Transformation { revision: number; provider: string; model: string; system: string; user: string; temperature: number | null; reasoning: string | null }
export interface Settings { transcription_mode: 'batch' | 'realtime'; revision: number; schema: number; microphone: string | null; transcription: Transcription; transform: boolean; preset_id: string; compression: boolean; silence_removal: boolean; copy_raw: boolean; copy_transformed: boolean; sounds: { started: boolean; stopped: boolean; canceled: boolean; transcribed: boolean; transformed: boolean }; keep_audio: number | null; theme: string; topmost: boolean; autostart: boolean; close_to_tray: boolean; diagnostic_logging: boolean; check_updates: boolean; shortcuts: { toggle: string; cancel: string } }
export interface Entry { id: string; created: string; source: string; title: string; subtitle: string; subtitle_manual: boolean; revision: number; audio: string | null; duration: number; bytes: number; transcript: string | null; transcript_revision: number; transformed: string | null; transformed_revision: number | null; transcription_ms: number | null; transformation_ms: number | null; transcript_model: string | null; transform_model: string | null }
export interface Job { id: string; entry_id: string; seq: number; kind: string; stage: string; status: string; remote_id: string | null; error: Failure | null; delivery: string; eligible: boolean }
export interface Model { provider: string; id: string; stage: string; prompt: boolean; context_field: string | null; temperature: [number, number] | null; max_bytes: number; context: number; output: number; label:string;description:string;reasoning:string[];default_reasoning:string }
export interface DiagnosticStatus { version: string; build_id: string; commit: string; directory: string; session: string; pid: number; enabled: boolean; error: string | null; dropped_events: number }
export interface AppState { operation_busy:boolean; operation_stage:string|null; diagnostics: DiagnosticStatus; recovery_warnings: Failure[]; settings: Settings; transformation: Transformation; jobs: Job[]; capture: { live?: {text:string;status:string;error:Failure|null}|null; finalizing?:boolean; active: boolean; started: string | null; error: Failure | null }; devices: { id: string; name: string }[]; credentials: Record<string, boolean>; models: Model[]; shortcut_error: Failure | null; playing: string | null }

export interface RecordingQuery {search:string;sort:string;descending:boolean;offset:number;page_size?:number}
export interface RecordingPage {entries:Entry[];total:number}
export interface Playback {id:string;paused:boolean;position:number;duration:number}

export interface Progress { operation_busy:boolean; operation_stage:string|null; jobs:Job[]; capture:AppState["capture"]; latest:Entry|null; playback:Playback|null; diagnostics:DiagnosticStatus }

export interface UpdateStatus {phase:string;version:string|null;notes:string|null;error:string|null;downloaded:number;total:number|null}
