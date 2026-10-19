import type {Job} from './types';
export type OrbPhase = 'idle'|'recording'|'transcribing'|'transforming'|'complete'|'canceling';
export const activeJob = (jobs:Job[]) => jobs.find(j=>['queued','running','canceling'].includes(j.status));
export function recorderPhase(active:boolean, jobs:Job[], operationStage:string|null|undefined, celebrating:boolean):OrbPhase {
 if(active)return 'recording';
 const job=activeJob(jobs);
 if(job?.status==='canceling')return 'canceling';
 const stage=job?.stage??operationStage;
 if(stage==='transforming')return 'transforming';
 if(job||stage)return 'transcribing';
 return celebrating?'complete':'idle';
}
/** A completion is a newly observed successful job, never startup history or a partial stage. */
export function newCompletion(previous:Map<string,string>, jobs:Job[]):string|null {
 return [...jobs].reverse().find(j=>j.status==='complete'&&previous.has(j.id)&&previous.get(j.id)!=='complete')?.id??null;
}
